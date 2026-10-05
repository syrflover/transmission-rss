use std::{path::Path, time::Duration};

use axum::{
    body::Body,
    http::{header, Method, Request, StatusCode},
    Router,
};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;

use super::*;
use trss_core::{Db, DbError};
use trss_jobs::{upload::Limits, ReceiveArea, Uploads};
use trss_subtitles::verify::zip_of;

const BOUNDARY: &str = "----trss-test-boundary";
const ASS: &[u8] = b"\xEF\xBB\xBF[Script Info]\nTitle: x\n\n[Events]\nDialogue: 0,0:00:01.00,0:00:02.00,Default,,0,0,0,,Hi\n";
const SRT: &[u8] = b"1\r\n00:00:01,000 --> 00:00:02,500\r\nHello\r\n";
const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR\x00\x00\x00\x01\x00\x00\x00\x01\x08\x06";

fn ttf() -> Vec<u8> {
    let mut bytes = b"\x00\x01\x00\x00\x00\x0C".to_vec();
    bytes.resize(12 + 16 * 12, 0);
    bytes
}

struct Setup {
    state: AppState,
    router: Router,
    dir: tempfile::TempDir,
}

impl Setup {
    fn receive(&self) -> std::path::PathBuf {
        self.dir.path().join("receive")
    }
}

async fn sql(state: &AppState, sql: &'static str) {
    state
        .jobs
        .db()
        .run::<_, DbError, _>(move |c| Ok(c.execute_batch(sql)?))
        .await
        .unwrap();
}

/// Work `w1` with season 1 linked to anime 3424 (creators `s1` 에루샤 and
/// `s2` 다른, each with a candidate) and season 2 with no link.
async fn setup_with(limits: Option<Limits>) -> Setup {
    let dir = tempfile::tempdir().unwrap();
    let area = ReceiveArea::new(dir.path().join("receive"));
    let mut state = AppState::new(Db::open_blocking(":memory:").unwrap()).with_receive_area(&area);
    if let Some(limits) = limits {
        let uploads = Uploads::new(state.jobs.clone(), area.clone()).with_limits(limits);
        state = state.with_uploads(uploads);
    }
    sql(
        &state,
        "INSERT INTO watch_folders (id, path, created_at) VALUES ('f1', '/media', 1);
         INSERT INTO works (id, watch_folder_id, dir_name) VALUES ('w1', 'f1', 'Show');
         INSERT INTO seasons (work_id, number) VALUES ('w1', 1);
         INSERT INTO seasons (work_id, number) VALUES ('w1', 2);
         INSERT INTO anissia_anime (anime_no, subject, week, status, fetched_at)
             VALUES (3424, '작품', 2, 'ON', 77);
         INSERT INTO season_anissia (work_id, season, anime_no, version) VALUES ('w1', 1, 3424, 1);
         INSERT INTO subtitle_sources (id, anime_no, creator_name, created_at)
             VALUES ('s1', 3424, '에루샤', 5), ('s2', 3424, '다른', 5), ('s9', 9, '남', 5);
         INSERT INTO caption_observations (source_id, post_url, episode, updated, first_seen_at)
             VALUES ('s1', 'https://fake.trss.invalid/ok/1', '1', 'x', 6),
                    ('s2', 'https://fake.trss.invalid/ok/4', '4', 'x', 6),
                    ('s9', 'https://fake.trss.invalid/ok/9', '9', 'x', 6);",
    )
    .await;
    let router = Router::new().nest("/api", crate::api::router().with_state(state.clone()));
    Setup { state, router, dir }
}

async fn setup() -> Setup {
    setup_with(None).await
}

/// A multipart body, its parts in the order they were added.
#[derive(Default, Clone)]
struct Form {
    body: Vec<u8>,
}

impl Form {
    /// The parts every upload starts with.
    fn to(id: &str, season: u32, creator: Option<&str>) -> Form {
        let form = Form::default()
            .text("id", id)
            .text("work_id", "w1")
            .text("season", &season.to_string());
        match creator {
            Some(creator) => form.text("creator", creator),
            None => form,
        }
    }

    fn text(mut self, name: &str, value: &str) -> Form {
        self.body.extend(
            format!(
                "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n"
            )
            .into_bytes(),
        );
        self
    }

    fn file(mut self, filename: &str, bytes: &[u8]) -> Form {
        self.body.extend(
            format!(
                "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{filename}\"\r\nContent-Type: application/octet-stream\r\n\r\n"
            )
            .into_bytes(),
        );
        self.body.extend(bytes);
        self.body.extend(b"\r\n");
        self
    }

    fn finish(&self) -> Vec<u8> {
        let mut body = self.body.clone();
        body.extend(format!("--{BOUNDARY}--\r\n").into_bytes());
        body
    }
}

fn request(body: Body) -> Request<Body> {
    Request::builder()
        .method(Method::POST)
        .uri("/api/subtitle-jobs/upload")
        .header(
            header::CONTENT_TYPE,
            format!("multipart/form-data; boundary={BOUNDARY}"),
        )
        .body(body)
        .unwrap()
}

async fn answer(router: &Router, request: Request<Body>) -> (StatusCode, Value) {
    let response = router.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

async fn upload(router: &Router, form: &Form) -> (StatusCode, Value) {
    answer(router, request(Body::from(form.finish()))).await
}

async fn detail(router: &Router, id: &str) -> Value {
    let request = Request::builder()
        .uri(format!("/api/subtitle-jobs/{id}"))
        .body(Body::empty())
        .unwrap();
    let (status, json) = answer(router, request).await;
    assert_eq!(status, StatusCode::OK, "{json}");
    json
}

/// The files and folders under `dir`, relative, sorted.
fn tree(dir: &Path) -> Vec<String> {
    fn walk(dir: &Path, base: &Path, out: &mut Vec<String>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let rel = path
                .strip_prefix(base)
                .unwrap()
                .to_string_lossy()
                .into_owned();
            out.push(rel);
            if path.is_dir() {
                walk(&path, base, out);
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out);
    out.sort();
    out
}

fn names(files: &Value) -> Vec<String> {
    files
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["name"].as_str().unwrap().to_owned())
        .collect()
}

async fn job_count(s: &Setup) -> usize {
    s.state.jobs.open_jobs().await.unwrap().len()
        + s.state.jobs.done_page(None, 50).await.unwrap().total
}

#[tokio::test]
async fn two_ass_a_font_and_a_text_file_make_one_package_that_says_what_it_dropped() {
    let s = setup().await;
    let form = Form::to("u1", 1, None)
        .text("skipped", "readme.txt")
        .file("01.ass", ASS)
        .file("02.ass", ASS)
        .file("Font.ttf", &ttf());
    let (status, made) = upload(&s.router, &form).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{made}");
    assert_eq!(
        made["kept"],
        json!({ "subtitles": 2, "fonts": 1, "archives": 0 })
    );
    assert_eq!(
        made["dropped"],
        json!([{ "name": "readme.txt", "reason": trss_jobs::upload::SKIPPED_REASON }])
    );

    let id = made["id"].as_str().unwrap();
    let job = detail(&s.router, id).await;
    assert_eq!(job["origin"], "upload");
    // Received, it waits for the worker's analysis and the 배치 확인.
    assert_eq!(job["state"], "pending");
    assert_eq!(job["receiving"], false);
    assert_eq!(job["title"], "작품");
    assert_eq!(job["season"], 1);
    assert_eq!(job["creator"], Value::Null);
    assert_eq!(job["episodes"], json!([]));
    assert_eq!(
        job["upload"],
        json!({ "subtitles": 2, "fonts": 1, "archives": 0, "dropped": 1 })
    );
    // One package: one item, three files, and no fetch steps.
    let items = job["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    let files = &items[0]["files"];
    assert_eq!(names(files), ["01.ass", "02.ass", "Font.ttf"]);
    let kinds: Vec<(&str, &str)> = files
        .as_array()
        .unwrap()
        .iter()
        .map(|f| (f["kind"].as_str().unwrap(), f["format"].as_str().unwrap()))
        .collect();
    assert_eq!(
        kinds,
        [("subtitle", "ass"), ("subtitle", "ass"), ("font", "other")]
    );
    assert_eq!(files[0]["size"], ASS.len());
    assert_eq!(files[0]["state"], "done");
    assert_eq!(
        job["dropped"],
        json!([{ "name": "readme.txt", "reason": trss_jobs::upload::SKIPPED_REASON }])
    );
    let steps: Vec<(&str, &str)> = job["steps"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| (s["step"].as_str().unwrap(), s["state"].as_str().unwrap()))
        .collect();
    assert_eq!(steps, [("receive", "done")]);
    assert!(job["log"][0]["detail"]
        .as_str()
        .unwrap()
        .contains("뺀 파일 1개"));

    // The bytes are in the job's folder of the receive area, and nowhere else.
    assert_eq!(
        std::fs::read(s.receive().join(id).join("01.ass")).unwrap(),
        ASS
    );
    assert_eq!(
        tree(&s.receive()),
        [
            ".tmp".to_owned(),
            id.to_owned(),
            format!("{id}/01.ass"),
            format!("{id}/02.ass"),
            format!("{id}/Font.ttf"),
        ]
    );

    // It is among the waiting jobs of the list.
    let request = Request::builder()
        .uri("/api/subtitle-jobs")
        .body(Body::empty())
        .unwrap();
    let (_, groups) = answer(&s.router, request).await;
    assert_eq!(groups["waiting"][0]["id"], id);
    assert_eq!(groups["waiting"][0]["origin"], "upload");
}

#[tokio::test]
async fn a_text_file_sent_anyway_is_dropped_by_the_server_and_named_in_the_job() {
    let s = setup().await;
    let form = Form::to("u1", 1, None)
        .file("01.ass", ASS)
        .file("readme.txt", b"Thanks for downloading.")
        .file("Font.ttf", &ttf());
    let (status, made) = upload(&s.router, &form).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{made}");
    assert_eq!(made["dropped"][0]["name"], "readme.txt");
    assert_eq!(
        made["dropped"][0]["reason"],
        "내용이 자막이나 폰트가 아니에요"
    );
    let job = detail(&s.router, made["id"].as_str().unwrap()).await;
    assert_eq!(job["dropped"][0]["name"], "readme.txt");
    assert_eq!(names(&job["items"][0]["files"]), ["01.ass", "Font.ttf"]);
}

#[tokio::test]
async fn an_image_with_an_ass_name_is_dropped_by_its_content() {
    let s = setup().await;
    let form = Form::to("u1", 1, None)
        .file("cover.ass", PNG)
        .file("real.ass", ASS)
        // A subtitle under a name that is no subtitle's is kept all the same.
        .file("notes.txt", SRT);
    let (status, made) = upload(&s.router, &form).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{made}");
    assert_eq!(names(&made["dropped"]), ["cover.ass"]);
    assert_eq!(
        made["kept"],
        json!({ "subtitles": 2, "fonts": 0, "archives": 0 })
    );
    let job = detail(&s.router, made["id"].as_str().unwrap()).await;
    assert_eq!(names(&job["items"][0]["files"]), ["real.ass", "notes.txt"]);
    // The image was not stored.
    let id = made["id"].as_str().unwrap();
    assert!(!s.receive().join(id).join("cover.ass").exists());
}

#[tokio::test]
async fn a_zip_stays_whole_in_the_package() {
    let s = setup().await;
    let zip = zip_of(&[
        ("01.ass", ASS),
        ("fonts/Font.ttf", &ttf()),
        ("readme.txt", b"hi"),
    ]);
    let (status, made) = upload(&s.router, &Form::to("u1", 1, None).file("pack.zip", &zip)).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{made}");
    assert_eq!(
        made["kept"],
        json!({ "subtitles": 0, "fonts": 0, "archives": 1 })
    );
    // What is inside is not judged here: the readme is not a dropped file.
    assert_eq!(made["dropped"], json!([]));
    let id = made["id"].as_str().unwrap();
    let job = detail(&s.router, id).await;
    let file = &job["items"][0]["files"][0];
    assert_eq!(file["name"], "pack.zip");
    assert_eq!(file["kind"], "archive");
    assert_eq!(file["format"], "zip");
    assert_eq!(file["size"], zip.len());
    assert_eq!(
        std::fs::read(s.receive().join(id).join("pack.zip")).unwrap(),
        zip
    );

    // A ZIP that cannot be read to its end is dropped, with the reason.
    let (status, made) = upload(
        &s.router,
        &Form::to("u2", 1, None)
            .file("broken.zip", &zip[..zip.len() / 2])
            .file("ok.srt", SRT),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{made}");
    assert_eq!(names(&made["dropped"]), ["broken.zip"]);
    assert!(made["dropped"][0]["reason"]
        .as_str()
        .unwrap()
        .contains("ZIP"));
}

#[tokio::test]
async fn a_rar_or_a_7z_stays_whole_in_the_package_and_a_fake_one_is_dropped() {
    let s = setup().await;
    let rar = [&b"Rar!\x1A\x07\x01\x00"[..], b"volume bytes"].concat();
    let sevenz = [&b"7z\xBC\xAF\x27\x1C\x00\x04"[..], b"volume bytes"].concat();
    let (status, made) = upload(
        &s.router,
        &Form::to("a1", 1, None)
            .file("pack.part1.rar", &rar)
            .file("pack.7z.001", &sevenz)
            .file("fake.rar", b"this is a text file")
            .file("01.ass", ASS),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{made}");
    assert_eq!(
        made["kept"],
        json!({ "subtitles": 1, "fonts": 0, "archives": 2 })
    );
    assert_eq!(names(&made["dropped"]), ["fake.rar"]);
    assert_eq!(made["dropped"][0]["reason"], "내용이 압축 파일이 아니에요");
    let id = made["id"].as_str().unwrap();
    let job = detail(&s.router, id).await;
    let files = job["items"][0]["files"].as_array().unwrap();
    let archives: Vec<_> = files
        .iter()
        .filter(|f| f["kind"] == "archive")
        .map(|f| {
            (
                f["name"].as_str().unwrap(),
                f["archive"].as_str().unwrap(),
                f["format"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        archives,
        [
            ("pack.part1.rar", "rar", "other"),
            ("pack.7z.001", "7z", "other")
        ]
    );
    assert_eq!(
        std::fs::read(s.receive().join(id).join("pack.part1.rar")).unwrap(),
        rar
    );
    // A ZIP says so too, and a file that is not an archive says nothing.
    let (_, made) = upload(
        &s.router,
        &Form::to("a2", 1, None).file("z.zip", &zip_of(&[("a.srt", SRT)])),
    )
    .await;
    let job = detail(&s.router, made["id"].as_str().unwrap()).await;
    assert_eq!(job["items"][0]["files"][0]["archive"], "zip");
    let ass = files.iter().find(|f| f["name"] == "01.ass").unwrap();
    assert_eq!(ass["archive"], Value::Null);
}

#[tokio::test]
async fn nothing_to_receive_makes_no_job_and_leaves_no_file() {
    let s = setup().await;
    for form in [
        Form::to("n1", 1, None)
            .file("cover.jpg", PNG)
            .file("readme.txt", b"text"),
        Form::to("n2", 1, None)
            .text("skipped", "a.mkv")
            .text("skipped", "b.nfo"),
        Form::to("n3", 1, None),
        Form::to("n4", 1, None).file("empty.ass", b""),
    ] {
        let (status, refused) = upload(&s.router, &form).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{refused}");
        assert_eq!(refused["error"], "invalid");
        assert!(
            refused["message"]
                .as_str()
                .unwrap()
                .contains("받을 자막이나 폰트가 없어요")
                || refused["message"]
                    .as_str()
                    .unwrap()
                    .contains("올린 파일이 없어요"),
            "{refused}"
        );
    }
    assert_eq!(job_count(&s).await, 0);
    // No folder of a job, and nothing staged is left.
    assert_eq!(tree(&s.receive()), [".tmp".to_owned()]);
}

#[tokio::test]
async fn names_cannot_leave_the_jobs_folder_and_a_folders_paths_are_kept_as_names() {
    let s = setup().await;
    let form = Form::to("u1", 1, None)
        .file("../../evil.ass", ASS)
        .file("Show/Season 1/01.ass", ASS)
        .file("Show/Season 2/01.ass", SRT)
        .file("Show/fonts/\u{202E}gpj.ttf", &ttf())
        .file("/abs/path/ep.srt", SRT)
        .file("..\\..\\win.ass", ASS);
    let (status, made) = upload(&s.router, &form).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{made}");
    let id = made["id"].as_str().unwrap();
    let job = detail(&s.router, id).await;
    assert_eq!(
        names(&job["items"][0]["files"]),
        [
            "evil.ass",
            "Show/Season 1/01.ass",
            "Show/Season 2/01.ass",
            "Show/fonts/gpj.ttf",
            "abs/path/ep.srt",
            "win.ass"
        ]
    );
    // Every file is flat in the job's folder, the second `01.ass` numbered.
    let on_disk = tree(&s.receive());
    for file in [
        "evil.ass",
        "01.ass",
        "01 (2).ass",
        "gpj.ttf",
        "ep.srt",
        "win.ass",
    ] {
        assert!(
            on_disk.contains(&format!("{id}/{file}")),
            "{file}: {on_disk:?}"
        );
    }
    assert!(on_disk
        .iter()
        .all(|p| p == ".tmp" || p == id || p.starts_with(&format!("{id}/"))));
    assert!(!s.dir.path().join("evil.ass").exists());
    assert!(!s.dir.path().parent().unwrap().join("evil.ass").exists());
    // The paths the detail shows are in the job's folder too.
    for file in job["items"][0]["files"].as_array().unwrap() {
        assert!(file["path"]
            .as_str()
            .unwrap()
            .starts_with(&format!("{}/", s.receive().join(id).to_string_lossy())));
    }
}

#[tokio::test]
async fn the_creator_is_one_of_the_seasons_or_unknown() {
    let s = setup().await;
    let (status, made) = upload(&s.router, &Form::to("c1", 1, Some("s1")).file("a.ass", ASS)).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{made}");
    let job = detail(&s.router, made["id"].as_str().unwrap()).await;
    assert_eq!(job["creator"], "에루샤");

    // An empty creator is `제작자 알 수 없음`.
    let (status, made) = upload(&s.router, &Form::to("c2", 1, Some("")).file("a.ass", ASS)).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{made}");
    let job = detail(&s.router, made["id"].as_str().unwrap()).await;
    assert_eq!(job["creator"], Value::Null);

    // Another anime's creator, one that is not there, and any creator of a
    // season with no Anissia link are refused before a file is stored.
    for (form, expected) in [
        (
            Form::to("c3", 1, Some("s9")).file("a.ass", ASS),
            StatusCode::BAD_REQUEST,
        ),
        (
            Form::to("c4", 1, Some("nope")).file("a.ass", ASS),
            StatusCode::BAD_REQUEST,
        ),
        (
            Form::to("c5", 2, Some("s1")).file("a.ass", ASS),
            StatusCode::BAD_REQUEST,
        ),
        (
            Form::to("c6", 3, None).file("a.ass", ASS),
            StatusCode::NOT_FOUND,
        ),
    ] {
        let (status, refused) = upload(&s.router, &form).await;
        assert_eq!(status, expected, "{refused}");
    }
    // A season with no link takes an upload by an unknown creator.
    let (status, made) = upload(&s.router, &Form::to("c7", 2, None).file("a.ass", ASS)).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{made}");
    let job = detail(&s.router, made["id"].as_str().unwrap()).await;
    assert_eq!(
        (job["season"].as_i64(), job["title"].as_str()),
        (Some(2), Some("Show"))
    );
    assert_eq!(job_count(&s).await, 3);
}

#[tokio::test]
async fn a_bad_request_is_refused_before_a_file_is_stored() {
    let s = setup().await;
    let file = |form: Form| form.file("a.ass", ASS);
    let blank = Form::default().text("work_id", "w1").text("season", "1");
    for (form, expected) in [
        (file(blank.clone()), StatusCode::BAD_REQUEST),
        (file(Form::to("   ", 1, None)), StatusCode::BAD_REQUEST),
        (file(Form::to("auto:3", 1, None)), StatusCode::BAD_REQUEST),
        (
            file(Form::to("recheck:3:0123456789abcdef", 1, None)),
            StatusCode::BAD_REQUEST,
        ),
        (
            file(Form::to(&"x".repeat(129), 1, None)),
            StatusCode::BAD_REQUEST,
        ),
        (
            file(Form::default().text("id", "a").text("season", "1")),
            StatusCode::BAD_REQUEST,
        ),
        (
            file(
                Form::default()
                    .text("id", "a")
                    .text("work_id", "w1")
                    .text("season", "one"),
            ),
            StatusCode::BAD_REQUEST,
        ),
        // A file before the parts that name the season.
        (
            Form::default()
                .file("a.ass", ASS)
                .text("id", "a")
                .text("work_id", "w1")
                .text("season", "1"),
            StatusCode::BAD_REQUEST,
        ),
        (
            Form::to("u", 1, None)
                .text("surprise", "x")
                .file("a.ass", ASS),
            StatusCode::BAD_REQUEST,
        ),
        (
            Form::to("u", 1, None).file("a.ass", ASS).text("id", "late"),
            StatusCode::BAD_REQUEST,
        ),
    ] {
        let (status, refused) = upload(&s.router, &form).await;
        assert_eq!(status, expected, "{refused}");
    }
    assert_eq!(job_count(&s).await, 0);
    assert_eq!(tree(&s.receive()), [".tmp".to_owned()]);

    // Not a multipart body at all.
    let request = Request::builder()
        .method(Method::POST)
        .uri("/api/subtitle-jobs/upload")
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .body(Body::from(ASS.to_vec()))
        .unwrap();
    assert_eq!(answer(&s.router, request).await.0, StatusCode::BAD_REQUEST);
}

fn limits(files: usize, entries: usize, total: u64, one: u64) -> Limits {
    Limits {
        files,
        entries,
        total_bytes: total,
        file_bytes: one,
        ..Limits::default()
    }
}

#[tokio::test]
async fn an_upload_past_a_limit_is_refused_while_it_is_stored_and_the_limit_is_told() {
    let s = setup_with(Some(limits(3, 5, 1 << 20, 100))).await;

    // Too many files: the fourth is refused, and the first three are removed.
    let many = Form::to("l1", 1, None)
        .file("1.srt", SRT)
        .file("2.srt", SRT)
        .file("3.srt", SRT)
        .file("4.srt", SRT);
    let (status, refused) = upload(&s.router, &many).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        refused["message"],
        "한 번에 파일 3개까지 올릴 수 있어요. 나눠서 올려 주세요."
    );

    // Too many names in all, those left out before sending included.
    let named = Form::to("l2", 1, None)
        .text("skipped", "a")
        .text("skipped", "b")
        .text("skipped", "c")
        .file("1.srt", SRT)
        .file("2.srt", SRT)
        .file("3.srt", SRT);
    let (status, refused) = upload(&s.router, &named).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        refused["message"].as_str().unwrap().contains("모두 5개"),
        "{refused}"
    );

    // A file past the size of one file (100 bytes here).
    let big = vec![b'1'; 101];
    let (status, refused) = upload(&s.router, &Form::to("l3", 1, None).file("big.srt", &big)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        refused["message"],
        "파일 하나는 100바이트까지 올릴 수 있어요."
    );

    assert_eq!(job_count(&s).await, 0);
    assert_eq!(tree(&s.receive()), [".tmp".to_owned()]);
    // A file at the limit is taken.
    let exact = vec![b'1'; 100];
    let (status, _) = upload(
        &s.router,
        &Form::to("l4", 1, None).file("exact.srt", &exact),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "100 bytes of '1' are no subtitle"
    );
    let (status, made) = upload(
        &s.router,
        &Form::to("l5", 1, None)
            .file("a.srt", SRT)
            .file("b.srt", SRT)
            .file("c.srt", SRT),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{made}");
}

#[tokio::test]
async fn the_total_size_is_held_while_the_bytes_come_and_by_the_declared_length() {
    let s = setup_with(Some(limits(10, 20, 150, 100))).await;
    let a = vec![b'x'; 80];
    let form = Form::to("t1", 1, None).file("a.srt", &a).file("b.srt", &a);
    let (status, refused) = upload(&s.router, &form).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        refused["message"],
        "한 번에 모두 합쳐 150바이트까지 올릴 수 있어요. 나눠서 올려 주세요."
    );
    assert_eq!(job_count(&s).await, 0);
    assert_eq!(tree(&s.receive()), [".tmp".to_owned()]);

    // A length declared past the limit (and the framing allowance) is refused
    // before the body is read, whatever the body is.
    let mut declared = request(Body::from(Vec::new()));
    declared.headers_mut().insert(
        header::CONTENT_LENGTH,
        (150 + FRAMING_ALLOWANCE + 1).to_string().parse().unwrap(),
    );
    let (status, refused) = answer(&s.router, declared).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(refused["message"].as_str().unwrap().contains("150바이트"));
}

#[tokio::test]
async fn a_body_that_stalls_or_takes_too_long_is_dropped_and_leaves_nothing() {
    let limits = Limits {
        body_timeout: Duration::from_millis(400),
        idle_timeout: Duration::from_millis(100),
        ..Limits::default()
    };
    let s = setup_with(Some(limits)).await;
    // It sends the first part, then nothing.
    let stalled = futures::stream::unfold(0, |n| async move {
        match n {
            0 => Some((
                Ok::<_, std::io::Error>(bytes::Bytes::from(half_of(&start_form()))),
                1,
            )),
            _ => {
                tokio::time::sleep(Duration::from_secs(5)).await;
                None
            }
        }
    });
    let (status, refused) = answer(&s.router, request(Body::from_stream(stalled))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        refused["message"]
            .as_str()
            .unwrap()
            .contains("아무것도 오지 않아"),
        "{refused}"
    );

    // It sends a byte at a time, never stalling, and takes longer than all.
    let trickle = futures::stream::unfold(0usize, move |n| {
        let bytes = half_of(&start_form());
        async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            bytes.get(n).map(|b| {
                (
                    Ok::<_, std::io::Error>(bytes::Bytes::copy_from_slice(&[*b])),
                    n + 1,
                )
            })
        }
    });
    let (status, refused) = answer(&s.router, request(Body::from_stream(trickle))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        refused["message"].as_str().unwrap().contains("멈췄어요"),
        "{refused}"
    );

    assert_eq!(job_count(&s).await, 0);
    assert_eq!(tree(&s.receive()), [".tmp".to_owned()]);
}

fn start_form() -> Form {
    Form::to("w1", 1, None).file("a.ass", ASS)
}

/// The form without its closing line, as a body cut short has it.
fn half_of(form: &Form) -> Vec<u8> {
    let mut body = form.body.clone();
    body.extend(format!("--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"b.ass\"\r\n\r\n[Script").into_bytes());
    body
}

#[tokio::test]
async fn a_retry_with_the_same_id_makes_one_job() {
    let s = setup().await;
    let form = Form::to("same", 1, Some("s1"))
        .file("01.ass", ASS)
        .file("Font.ttf", &ttf());
    let (status, first) = upload(&s.router, &form).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{first}");
    let id = first["id"].as_str().unwrap().to_owned();

    // The answer was lost: the same upload again.
    let (status, again) = upload(&s.router, &form).await;
    assert_eq!(
        (status, again["id"].as_str()),
        (StatusCode::OK, Some(id.as_str()))
    );
    assert_eq!(job_count(&s).await, 1);
    // Its files were not kept a second time, and nothing staged is left.
    assert_eq!(
        tree(&s.receive()),
        [
            ".tmp".to_owned(),
            id.clone(),
            format!("{id}/01.ass"),
            format!("{id}/Font.ttf"),
        ]
    );

    // Another upload under the same ID is a conflict that names the job.
    for other in [
        Form::to("same", 1, Some("s1")).file("01.ass", ASS),
        Form::to("same", 1, Some("s2"))
            .file("01.ass", ASS)
            .file("Font.ttf", &ttf()),
        Form::to("same", 2, None)
            .file("01.ass", ASS)
            .file("Font.ttf", &ttf()),
        Form::to("same", 1, Some("s1"))
            .text("skipped", "x")
            .file("01.ass", ASS)
            .file("Font.ttf", &ttf()),
        Form::to("same", 1, Some("s1"))
            .file("01.ass", SRT)
            .file("Font.ttf", &ttf()),
    ] {
        let (status, conflict) = upload(&s.router, &other).await;
        assert_eq!(status, StatusCode::CONFLICT, "{conflict}");
        assert_eq!(conflict["current"]["id"].as_str(), Some(id.as_str()));
    }
    assert_eq!(job_count(&s).await, 1);
    assert_eq!(tree(&s.receive()).len(), 4);
}

#[tokio::test]
async fn two_deliveries_at_once_make_one_job() {
    let s = setup().await;
    let form = Form::to("race", 1, None)
        .file("01.ass", ASS)
        .file("02.srt", SRT);
    let (first, second) = tokio::join!(upload(&s.router, &form), upload(&s.router, &form));
    let mut statuses = [first.0, second.0];
    statuses.sort();
    assert_eq!(statuses, [StatusCode::OK, StatusCode::ACCEPTED]);
    assert_eq!(first.1["id"], second.1["id"]);
    assert_eq!(job_count(&s).await, 1);
    let id = first.1["id"].as_str().unwrap();
    assert_eq!(
        tree(&s.receive()),
        [
            ".tmp".to_owned(),
            id.to_owned(),
            format!("{id}/01.ass"),
            format!("{id}/02.srt"),
        ]
    );
}

// --- the framing and the sender are bounded ------------------------------------------

use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};

const CHUNK: usize = 64 * 1024;

/// `head`, then `count` chunks of `CHUNK` bytes of `fill` made as they are asked for, then `tail`.
/// `pulled` counts the bytes handed out.
fn counted(head: Vec<u8>, fill: u8, count: usize, tail: Vec<u8>, pulled: &Arc<AtomicU64>) -> Body {
    let pulled = Arc::clone(pulled);
    let pieces = std::iter::once(head)
        .chain((0..count).map(move |_| vec![fill; CHUNK]))
        .chain(std::iter::once(tail));
    Body::from_stream(futures::stream::iter(pieces.map(move |piece| {
        pulled.fetch_add(piece.len() as u64, AtomicOrdering::SeqCst);
        Ok::<_, std::io::Error>(bytes::Bytes::from(piece))
    })))
}

#[tokio::test]
async fn a_part_header_that_never_ends_is_refused_after_a_small_backlog() {
    let s = setup().await;
    let pulled = Arc::new(AtomicU64::new(0));
    // 512 MiB that could be sent: a header with no end. Before the backlog was
    // counted, the multipart reader held all of it.
    let head = format!("--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"").into_bytes();
    let body = counted(head, b'a', 8192, Vec::new(), &pulled);
    let (status, refused) = answer(&s.router, request(body)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{refused}");
    assert!(
        refused["message"].as_str().unwrap().contains("머리글"),
        "{refused}"
    );
    let taken = pulled.load(AtomicOrdering::SeqCst);
    assert!(taken < 2 * 1024 * 1024, "{taken} bytes were read");
    assert_eq!(job_count(&s).await, 0);
    assert_eq!(tree(&s.receive()), [".tmp".to_owned()]);
}

#[tokio::test]
async fn a_long_preamble_before_the_first_part_is_refused_the_same_way() {
    let s = setup().await;
    let pulled = Arc::new(AtomicU64::new(0));
    let body = counted(
        b"junk ".to_vec(),
        b'x',
        8192,
        Form::default().finish(),
        &pulled,
    );
    let (status, refused) = answer(&s.router, request(body)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{refused}");
    let taken = pulled.load(AtomicOrdering::SeqCst);
    assert!(taken < 2 * 1024 * 1024, "{taken} bytes were read");
    assert_eq!(job_count(&s).await, 0);
}

#[tokio::test]
async fn many_small_parts_never_trip_the_backlog() {
    // The framing between parts is counted as used when the next part opens,
    // so no number of parts adds up to a backlog.
    let s = setup().await;
    let mut form = Form::to("many", 1, None);
    for n in 0..1500 {
        form = form.text("skipped", &format!("{n:04}-{}.txt", "n".repeat(380)));
    }
    let form = form.file("01.ass", ASS);
    assert!(form.body.len() > 2 * BACKLOG as usize);
    let (status, made) = upload(&s.router, &form).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{made}");
    assert_eq!(made["dropped"].as_array().unwrap().len(), 1500);
}

#[tokio::test]
async fn a_body_with_no_length_is_cut_at_the_total_and_the_framing_allowance() {
    let limits = Limits {
        total_bytes: 1 << 20,
        ..Limits::default()
    };
    let s = setup_with(Some(limits)).await;
    let pulled = Arc::new(AtomicU64::new(0));
    // A chunked body (no `Content-Length`) of a file much past the limit, and
    // with a file's worth of padding it could be that never gets that far.
    let head = Form::to("chunked", 1, None).body;
    let mut head = head;
    head.extend(format!("--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"big.ass\"\r\n\r\n[Script Info]\n").into_bytes());
    let body = counted(head, b'x', 1024, Vec::new(), &pulled);
    let (status, refused) = answer(&s.router, request(body)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{refused}");
    assert!(
        refused["message"].as_str().unwrap().contains("1MiB"),
        "{refused}"
    );
    // The file went past the limit (a refusal of its own, then drained as
    // far as any refused body is).
    let taken = pulled.load(AtomicOrdering::SeqCst);
    assert!(
        taken < (1 << 20) + DRAIN_BYTES + 3 * CHUNK as u64,
        "{taken} bytes were read"
    );
    assert_eq!(tree(&s.receive()), [".tmp".to_owned()]);

    // Padding in the framing (text parts) is bounded by the same sum.
    let pulled = Arc::new(AtomicU64::new(0));
    let head = format!("--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"skipped\"\r\n\r\n")
        .into_bytes();
    let body = counted(head, b'x', 1024, Vec::new(), &pulled);
    let (status, _) = answer(&s.router, request(body)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    // The text part is too long (a refusal of its own, so the body is drained
    // as far as any refused body is, and no further).
    let taken = pulled.load(AtomicOrdering::SeqCst);
    assert!(
        taken < DRAIN_BYTES + 3 * CHUNK as u64,
        "{taken} bytes were read"
    );
}

#[tokio::test]
async fn names_are_bounded() {
    let s = setup().await;
    let long = "a".repeat(FILE_NAME_MAX + 1);
    let (status, refused) = upload(
        &s.router,
        &Form::to("long", 1, None).file(&format!("{long}.ass"), ASS),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(refused["message"]
        .as_str()
        .unwrap()
        .contains("이름이 너무 길어요"));
    let (status, _) = upload(
        &s.router,
        &Form::to("long", 1, None).text(&"n".repeat(PART_NAME_MAX + 1), "x"),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(job_count(&s).await, 0);
    assert_eq!(tree(&s.receive()), [".tmp".to_owned()]);
}

#[tokio::test]
async fn a_sender_that_trickles_is_cut_by_the_least_average_speed() {
    // Never stalls for the idle time and is well within the whole time, but
    // averages a few KiB/s once the grace is over.
    let limits = Limits {
        rate_grace: Duration::from_millis(300),
        min_rate: 64 * 1024,
        idle_timeout: Duration::from_secs(5),
        body_timeout: Duration::from_secs(30),
        ..Limits::default()
    };
    let s = setup_with(Some(limits)).await;
    let mut head = Form::to("slow", 1, None).body;
    head.extend(format!("--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"a.ass\"\r\n\r\n").into_bytes());
    let trickle = futures::stream::unfold(Some(head), |next| async move {
        let piece = match next {
            Some(head) => head,
            None => {
                tokio::time::sleep(Duration::from_millis(60)).await;
                vec![b'x'; 100]
            }
        };
        Some((Ok::<_, std::io::Error>(bytes::Bytes::from(piece)), None))
    });
    let started = std::time::Instant::now();
    let (status, refused) = answer(&s.router, request(Body::from_stream(trickle))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{refused}");
    assert!(
        refused["message"].as_str().unwrap().contains("너무 느려서"),
        "{refused}"
    );
    assert!(started.elapsed() < Duration::from_secs(3));
    assert_eq!(job_count(&s).await, 0);
    assert_eq!(tree(&s.receive()), [".tmp".to_owned()]);
}

#[tokio::test]
async fn bytes_sent_up_front_buy_no_time_for_a_trickle_after_them() {
    // 2 MiB at once would carry an average of 64 KiB/s for half a minute, but
    // only the last window counts: the trickle that follows is cut as soon as
    // the grace is over.
    let limits = Limits {
        rate_grace: Duration::from_millis(300),
        min_rate: 64 * 1024,
        idle_timeout: Duration::from_secs(5),
        body_timeout: Duration::from_secs(30),
        ..Limits::default()
    };
    let s = setup_with(Some(limits)).await;
    let mut head = Form::to("front", 1, None).body;
    head.extend(format!("--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"a.ass\"\r\n\r\n").into_bytes());
    head.extend(vec![b'x'; 2 * 1024 * 1024]);
    let body = futures::stream::unfold(Some(head), |next| async move {
        let piece = match next {
            Some(head) => head,
            None => {
                tokio::time::sleep(Duration::from_millis(60)).await;
                vec![b'x'; 100]
            }
        };
        Some((Ok::<_, std::io::Error>(bytes::Bytes::from(piece)), None))
    });
    let started = std::time::Instant::now();
    let (status, refused) = answer(&s.router, request(Body::from_stream(body))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{refused}");
    assert!(
        refused["message"].as_str().unwrap().contains("너무 느려서"),
        "{refused}"
    );
    assert!(started.elapsed() < Duration::from_secs(3));
    assert_eq!(job_count(&s).await, 0);
}

#[tokio::test]
async fn a_sender_that_keeps_the_least_speed_in_every_window_is_not_cut() {
    // 4 KiB every 20 ms is 200 KiB/s, over the 64 KiB/s, for longer than the
    // grace.
    let limits = Limits {
        rate_grace: Duration::from_millis(300),
        min_rate: 64 * 1024,
        ..Limits::default()
    };
    let s = setup_with(Some(limits)).await;
    let mut head = Form::to("steady", 1, None).body;
    head.extend(format!("--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"a.ass\"\r\n\r\n").into_bytes());
    let body = futures::stream::unfold(0u32, move |n| {
        let head = head.clone();
        async move {
            let piece = match n {
                0 => head,
                1..=40 => {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                    vec![b'x'; 4096]
                }
                41 => format!("\r\n--{BOUNDARY}--\r\n").into_bytes(),
                _ => return None,
            };
            Some((Ok::<_, std::io::Error>(bytes::Bytes::from(piece)), n + 1))
        }
    });
    // The body was read to its end (the answer is about the file's content,
    // which is no subtitle), not cut for its speed.
    let (status, answered) = answer(
        &s.router,
        request(Body::from_stream(futures::StreamExt::fuse(body))),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{answered}");
    let message = answered["message"].as_str().unwrap();
    assert!(
        message.contains("받을 자막이나 폰트가 없어요"),
        "{answered}"
    );
}

#[tokio::test]
async fn a_busy_server_says_so_at_once_when_the_queue_is_full() {
    let limits = Limits {
        queue: 0,
        ..Limits::default()
    };
    let s = setup_with(Some(limits)).await;
    let mut held = Vec::new();
    for _ in 0..trss_jobs::upload::UPLOAD_SLOTS {
        held.push(s.state.uploads.begin().await.unwrap());
    }
    let started = std::time::Instant::now();
    let response = s
        .router
        .clone()
        .oneshot(request(Body::from(
            Form::to("busy", 1, None).file("01.ass", ASS).finish(),
        )))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["error"], "unavailable");
    assert!(json["message"].as_str().unwrap().contains("잠시 뒤"));
    assert!(started.elapsed() < Duration::from_secs(2));
    // Nothing was made, and the turn that is free later serves the same upload.
    assert_eq!(job_count(&s).await, 0);
    drop(held);
    let (status, _) = upload(&s.router, &Form::to("busy", 1, None).file("01.ass", ASS)).await;
    assert_eq!(status, StatusCode::ACCEPTED);
}

#[tokio::test]
async fn a_refusal_in_the_middle_reads_the_rest_of_a_small_body_first() {
    let limits = limits(1, 5, 1 << 20, 1 << 20);
    let s = setup_with(Some(limits)).await;
    // The second file is one too many; a body of a few hundred KiB follows it.
    let pulled = Arc::new(AtomicU64::new(0));
    let mut head = Form::to("drain", 1, None).file("1.ass", ASS).body;
    head.extend(format!("--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"2.ass\"\r\n\r\n").into_bytes());
    let tail = Form::default().finish();
    let body = counted(head, b'x', 8, tail, &pulled);
    let total = {
        let before = pulled.load(AtomicOrdering::SeqCst);
        assert_eq!(before, 0);
        // What the stream will hand out in all.
        let probe = Arc::new(AtomicU64::new(0));
        let mut head = Form::to("drain", 1, None).file("1.ass", ASS).body;
        head.extend(format!("--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"2.ass\"\r\n\r\n").into_bytes());
        let probe_body = counted(head, b'x', 8, Form::default().finish(), &probe);
        let _ = probe_body.collect().await.unwrap();
        probe.load(AtomicOrdering::SeqCst)
    };
    let (status, refused) = answer(&s.router, request(body)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{refused}");
    assert!(refused["message"].as_str().unwrap().contains("1개까지"));
    assert_eq!(
        pulled.load(AtomicOrdering::SeqCst),
        total,
        "the body was read to its end"
    );

    // A body of any size is drained only so far.
    let pulled = Arc::new(AtomicU64::new(0));
    let mut head = Form::to("drain2", 1, None).file("1.ass", ASS).body;
    head.extend(format!("--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"2.ass\"\r\n\r\n").into_bytes());
    let body = counted(head, b'x', 100_000, Vec::new(), &pulled);
    let started = std::time::Instant::now();
    let (status, _) = answer(&s.router, request(body)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let taken = pulled.load(AtomicOrdering::SeqCst);
    assert!(
        taken < 10 * 1024 * 1024 + 2 * CHUNK as u64,
        "{taken} bytes were read"
    );
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[test]
fn the_body_itself_stops_past_the_total_and_the_framing_allowance() {
    let limits = Limits {
        total_bytes: 1 << 20,
        ..Limits::default()
    };
    let gauge = Arc::new(Gauge::default());
    let mut body = Resting::new(
        Arc::new(Mutex::new(futures::stream::empty::<
            Result<Bytes, axum::Error>,
        >())),
        Arc::clone(&gauge),
        limits,
    );
    let allowed = limits.total_bytes + FRAMING_ALLOWANCE;
    let mut sent = 0u64;
    let refusal = loop {
        // Everything that came was used, so only the sum is in question.
        gauge.caught_up();
        if let Some(why) = body.judge(CHUNK as u64) {
            break why;
        }
        sent += CHUNK as u64;
    };
    assert!(refusal == Refusal::Total);
    assert!(
        sent <= allowed && sent + CHUNK as u64 > allowed - CHUNK as u64,
        "{sent}"
    );
}
