//! The runner with the Blogger source, and Tistory's Drive links, against a
//! local server shaped like Blogger and Google Drive
//! ([`trss_subtitles::testing`]), and once against the real sites (ignored).

use crate::{world::Base, Handles};
use std::time::Duration;

use sha2::{Digest, Sha256};
use tokio_util::sync::CancellationToken;
use trss_core::{Db, DbError};
use trss_jobs::{
    area::{self, ReceiveArea},
    store::JobDetail,
    Created, FailureKind, FileState, Format, ItemState, JobState, NewItem, NewJob, Runner,
    StepKind, StepState, Wait,
};
use trss_subtitles::{
    blogger::BloggerSource,
    drive::Drive,
    fake,
    testing::{
        blogger_page, drive_link, drive_page, DriveAnswer, PostAnswer, SourceServer,
        BLOGGER_MODIFIED, DRIVE_MODIFIED,
    },
    tistory::TistorySource,
    verify, Sources,
};

struct Setup {
    _dir: tempfile::TempDir,
    store: Handles,
    runner: Runner,
    area: ReceiveArea,
    server: SourceServer,
}

async fn setup() -> Setup {
    let server = SourceServer::start().await;
    let base = Base::new().await;
    let runner = base
        .runner(
            Sources::none()
                .with_tistory(server.source())
                .with_blogger(server.blogger()),
        )
        .with_retry_waits(vec![Duration::from_millis(10), Duration::from_millis(10)]);
    Setup {
        _dir: base.dir,
        store: base.store,
        runner,
        area: base.area,
        server,
    }
}

/// Makes a job of `posts` (episode, address).
async fn make(store: &Handles, command: &str, posts: &[(&str, String)]) -> String {
    let job = NewJob {
        command_id: command.to_owned(),
        request: "{}".to_owned(),
        origin: "pick".to_owned(),
        work_id: None,
        season: Some(1),
        anime_no: None,
        source_id: None,
        creator: Some("C소라".to_owned()),
        revision_of: None,
        revises_attributed: false,
        items: posts
            .iter()
            .map(|(episode, post)| NewItem {
                observation_id: None,
                episode: (*episode).to_owned(),
                post_url: post.clone(),
                found_at: 500,
            })
            .collect(),
    };
    match store.requests.create(job, 900).await.unwrap() {
        Created::Created(id) => id,
        other => panic!("created: {other:?}"),
    }
}

async fn run(s: &Setup) {
    s.runner.run_ready(&CancellationToken::new()).await.unwrap();
}

async fn detail(s: &Setup, id: &str) -> JobDetail {
    s.store.views.detail(id).await.unwrap().unwrap()
}

fn step(d: &JobDetail, kind: StepKind) -> Option<StepState> {
    d.steps.iter().find(|s| s.step == kind).map(|s| s.state)
}

fn files_in(dir: &std::path::Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

/// A post of these links (words, Drive ID) on C소라's blog at `path`.
fn post(s: &Setup, path: &str, links: &[(&str, &str)]) -> String {
    let links: Vec<(String, String)> = links
        .iter()
        .map(|(words, id)| ((*words).to_owned(), drive_link(id)))
        .collect();
    let links: Vec<(&str, &str)> = links
        .iter()
        .map(|(w, h)| (w.as_str(), h.as_str()))
        .collect();
    s.server.blogger_post(
        "csora556",
        path,
        vec![PostAnswer::Page(blogger_page(&links))],
    );
    s.server.blogger_url("csora556", path)
}

fn file(name: &str, bytes: Vec<u8>) -> Vec<DriveAnswer> {
    vec![DriveAnswer::File {
        name: name.to_owned(),
        bytes,
    }]
}

/// A post of fonts and one link per episode (C소라 2026/07/2): each item
/// receives its own episode's file only, and the fonts once for the job.
#[tokio::test]
async fn individual_links_give_only_the_chosen_episodes_file_and_the_fonts_once() {
    let s = setup().await;
    let mut links = vec![("폰트".to_owned(), "1fontsOfThePost".to_owned())];
    links.extend((13..=24).map(|n| (format!("{n}화"), format!("1episodeNumber{n}"))));
    let links: Vec<(&str, &str)> = links
        .iter()
        .map(|(w, id)| (w.as_str(), id.as_str()))
        .collect();
    let url = post(&s, "2026/07/2.html", &links);
    let fonts = verify::zip_of(&[("얼음폰트.ttf", b"font")]);
    s.server
        .drive("1fontsOfThePost", file("얼음폰트.zip", fonts.clone()));
    for n in 13..=24 {
        s.server.drive(
            &format!("1episodeNumber{n}"),
            file(
                &format!("Seihantai {n}.ass"),
                fake::ass(&format!("Seihantai {n}")),
            ),
        );
    }
    let id = make(&s.store, "c1", &[("23", url.clone()), ("24", url)]).await;
    run(&s).await;

    let d = detail(&s, &id).await;
    assert_eq!(d.row.state, JobState::Done);
    for kind in [StepKind::Open, StepKind::Receive] {
        assert_eq!(step(&d, kind), Some(StepState::Done), "{kind:?}");
    }
    // The names Drive gave, and no other episode's file.
    assert_eq!(
        files_in(&s.area.at(&id)),
        ["Seihantai 23.ass", "Seihantai 24.ass", "얼음폰트.zip"]
    );
    for n in 13..=22 {
        assert_eq!(s.server.drive_asked(&format!("1episodeNumber{n}")), 0);
    }
    assert_eq!(s.server.drive_asked("1fontsOfThePost"), 1);

    let ep24 = &d.items[1];
    assert_eq!(ep24.state, ItemState::Done);
    let names: Vec<&str> = ep24.files.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, ["얼음폰트.zip", "Seihantai 24.ass"]);
    let font = &ep24.files[0];
    assert!(font.same_as.is_some(), "the fonts are the first item's");
    let ass = &ep24.files[1];
    assert_eq!(ass.state, FileState::Done);
    assert_eq!(ass.format, Some(Format::Ass));
    let bytes = fake::ass("Seihantai 24");
    assert_eq!(ass.size, Some(bytes.len() as u64));
    assert_eq!(ass.expected_size, Some(bytes.len() as u64));
    assert_eq!(
        ass.sha256.as_deref(),
        Some(area::hex(&Sha256::digest(&bytes)).as_str())
    );
    assert_eq!(ass.http_status, Some(200));
    let snapshot: Vec<[String; 2]> =
        serde_json::from_str(ass.snapshot.as_deref().unwrap()).unwrap();
    assert_eq!(
        snapshot,
        [
            ["dateModified".to_owned(), BLOGGER_MODIFIED.to_owned()],
            ["last_modified".to_owned(), DRIVE_MODIFIED.to_owned()],
            ["content_length".to_owned(), bytes.len().to_string()],
        ]
    );
    assert!(s.server.seen().iter().all(|r| !r.cookie && !r.referer));
}

/// A post of fonts and a ZIP of a range (C소라 season-3): the ZIP is received
/// once for the episodes it holds, as a ZIP.
#[tokio::test]
async fn a_zip_of_a_range_is_received_once_for_its_episodes_as_a_zip() {
    let s = setup().await;
    let url = post(
        &s,
        "2026/07/season-3.html",
        &[
            ("폰트", "1yiOdJ6YbwrbGmJ4gVMGeLfhdk4w0dAwM"),
            ("1 ~ 12화", "10EFK_9-G9VPVFy08Zuit88H0ciLJMFKq"),
        ],
    );
    let members: Vec<(String, Vec<u8>)> = (1..=12)
        .map(|n| {
            (
                format!("Grand Blue S3 {n:02}.ass"),
                fake::ass(&format!("{n}")),
            )
        })
        .collect();
    let members: Vec<(&str, &[u8])> = members
        .iter()
        .map(|(n, b)| (n.as_str(), b.as_slice()))
        .collect();
    let zip = verify::zip_of(&members);
    s.server.drive(
        "10EFK_9-G9VPVFy08Zuit88H0ciLJMFKq",
        file("그랑블루3 1-12.zip", zip.clone()),
    );
    s.server.drive(
        "1yiOdJ6YbwrbGmJ4gVMGeLfhdk4w0dAwM",
        file("그랑블루폰트.zip", verify::zip_of(&[("a.ttf", b"font")])),
    );
    let id = make(&s.store, "c1", &[("11", url.clone()), ("12", url.clone())]).await;
    // An episode the range does not hold: the post has changed.
    let past = make(&s.store, "c2", &[("13", url)]).await;
    run(&s).await;

    let d = detail(&s, &id).await;
    assert_eq!(d.row.state, JobState::Done);
    assert_eq!(
        files_in(&s.area.at(&id)),
        ["그랑블루3 1-12.zip", "그랑블루폰트.zip"]
    );
    assert_eq!(s.server.drive_asked("10EFK_9-G9VPVFy08Zuit88H0ciLJMFKq"), 1);
    let received = &d.items[0].files[1];
    assert_eq!(received.name, "그랑블루3 1-12.zip");
    assert_eq!(received.format, Some(Format::Zip));
    assert_eq!(received.size, Some(zip.len() as u64));
    assert!(d.items[1].files[1].same_as.is_some());

    let d = detail(&s, &past).await;
    assert_eq!(d.row.state, JobState::Failed);
    assert_eq!(d.items[0].failure, Some(FailureKind::Changed));
    assert_eq!(
        d.items[0].reason.as_deref(),
        Some("게시물에 13화 파일이 없어요")
    );
    assert_eq!(step(&d, StepKind::Open), Some(StepState::Failed));
}

/// Drive gives a page instead of the file: the receipt fails by its class,
/// with what answered, and leaves no bytes.
#[tokio::test]
async fn a_confirmation_or_error_page_from_drive_is_a_classified_failure() {
    let s = setup().await;
    let url = post(
        &s,
        "2026/10/2.html",
        &[("15화", "1W0LRBy-gxCLtmDpQBd8kGYb79oh19YRe")],
    );
    let url16 = post(
        &s,
        "2026/10/3.html",
        &[("16화", "1W0LRBy-gxCLtmDpQBd8kGYb79oh19Y16")],
    );
    s.server.drive(
        "1W0LRBy-gxCLtmDpQBd8kGYb79oh19YRe",
        vec![DriveAnswer::Confirm],
    );
    s.server.drive(
        "1W0LRBy-gxCLtmDpQBd8kGYb79oh19Y16",
        vec![DriveAnswer::Missing],
    );
    let confirm = make(&s.store, "c1", &[("15", url)]).await;
    let missing = make(&s.store, "c2", &[("16", url16)]).await;
    run(&s).await;

    let d = detail(&s, &confirm).await;
    assert_eq!(d.row.state, JobState::Failed);
    assert_eq!(d.row.failure, Some(FailureKind::NotAFile));
    let f = &d.items[0].files[0];
    assert_eq!(f.state, FileState::Failed);
    assert_eq!(f.failure, Some(FailureKind::NotAFile));
    assert_eq!(f.name, "15화");
    assert!(f.reason.as_deref().unwrap().contains("확인 페이지"));
    assert_eq!(f.http_status, Some(200));
    assert_eq!(f.content_type.as_deref(), Some("text/html"));
    assert!(f.size.is_none() && f.path.is_none());
    // One request: a page is not tried again, and its confirmation is not
    // passed.
    assert_eq!(s.server.drive_asked("1W0LRBy-gxCLtmDpQBd8kGYb79oh19YRe"), 1);
    assert!(s.server.seen().iter().all(|r| !r.query.contains("confirm")));
    assert!(files_in(&s.area.at(&confirm)).is_empty());
    assert!(files_in(&s.area.at(".tmp")).is_empty());

    let d = detail(&s, &missing).await;
    assert_eq!(d.row.failure, Some(FailureKind::Missing));
    let f = &d.items[0].files[0];
    assert_eq!(
        (f.http_status, f.content_type.as_deref(), f.response_size),
        (Some(404), Some("text/html"), Some(1652))
    );
}

/// A Tistory post whose subtitle is a Drive link in its body (felia 1187) is
/// received from Drive now, not left waiting.
#[tokio::test]
async fn a_tistory_posts_drive_link_is_received_from_drive() {
    let s = setup().await;
    s.server.post(
        "felia",
        1187,
        vec![PostAnswer::Page(drive_page(
            "1zqWESZSANz8Uj2aaW9u1oSHKurLuTqQw",
        ))],
    );
    s.server.drive(
        "1zqWESZSANz8Uj2aaW9u1oSHKurLuTqQw",
        file("FX 전사 쿠루미 01.ass", fake::ass("Kurumi 01")),
    );
    let id = make(&s.store, "c1", &[("1", s.server.post_url("felia", 1187))]).await;
    run(&s).await;

    let d = detail(&s, &id).await;
    assert_eq!(d.row.state, JobState::Done);
    assert_eq!(files_in(&s.area.at(&id)), ["FX 전사 쿠루미 01.ass"]);
    let f = &d.items[0].files[0];
    assert_eq!(f.format, Some(Format::Ass));
    let snapshot: Vec<[String; 2]> = serde_json::from_str(f.snapshot.as_deref().unwrap()).unwrap();
    assert_eq!(
        snapshot[0],
        [
            "article:modified_time".to_owned(),
            "2026-10-01T23:10:30+09:00".to_owned()
        ]
    );
}

/// A post that links only a Drive folder waits for a source with its
/// reason; one that links nothing has changed.
#[tokio::test]
async fn a_folder_link_waits_and_a_post_of_no_link_has_changed() {
    let s = setup().await;
    let folder = r#"<html><body><div class='post-body entry-content'><a href="https://drive.google.com/drive/folders/10YFO-jkkgsybQnPpl5TVAwPdx2P0SE-y">자막 모음(작업 중)</a></div></body></html>"#;
    s.server.blogger_post(
        "bluewater91",
        "2026/09/folder.html",
        vec![PostAnswer::Page(folder.to_owned())],
    );
    // The blog's folder link sits beside the body: not the post's.
    s.server.blogger_post(
        "bluewater91",
        "2026/09/none.html",
        vec![PostAnswer::Page(blogger_page(&[]))],
    );
    let waits = make(
        &s.store,
        "c1",
        &[(
            "1",
            s.server.blogger_url("bluewater91", "2026/09/folder.html"),
        )],
    )
    .await;
    let changed = make(
        &s.store,
        "c2",
        &[(
            "1",
            s.server.blogger_url("bluewater91", "2026/09/none.html"),
        )],
    )
    .await;
    let gone = make(
        &s.store,
        "c3",
        &[(
            "1",
            s.server.blogger_url("bluewater91", "2026/09/gone.html"),
        )],
    )
    .await;
    run(&s).await;

    let d = detail(&s, &waits).await;
    assert_eq!(
        (d.row.state, d.row.wait),
        (JobState::Waiting, Some(Wait::Subtitle))
    );
    assert!(d.items[0]
        .reason
        .as_deref()
        .unwrap()
        .contains("Google Drive 폴더"));
    let d = detail(&s, &changed).await;
    assert_eq!(d.row.failure, Some(FailureKind::Changed));
    let d = detail(&s, &gone).await;
    assert_eq!(d.row.failure, Some(FailureKind::Missing));
}

/// What a Drive receipt keeps: the post's address, the file's key and the
/// names; no download address, cookie or query.
#[tokio::test]
async fn no_download_address_reaches_the_records() {
    let s = setup().await;
    let url = post(
        &s,
        "2026/10/2.html",
        &[
            ("폰트", "18dW2jKcf-1VPj0f739afgBVD-gWKO0Db"),
            ("15화", "1W0LRBy-gxCLtmDpQBd8kGYb79oh19YRe"),
        ],
    );
    s.server.drive(
        "18dW2jKcf-1VPj0f739afgBVD-gWKO0Db",
        vec![DriveAnswer::Quota],
    );
    s.server.drive(
        "1W0LRBy-gxCLtmDpQBd8kGYb79oh19YRe",
        file("Koori 15.ass", fake::ass("15")),
    );
    let id = make(&s.store, "c1", &[("15", url)]).await;
    run(&s).await;
    assert_eq!(detail(&s, &id).await.row.state, JobState::Failed);

    let dump: String = s
        .store
        .run
        .db()
        .run::<_, DbError, _>(|c| {
            let mut all = String::new();
            for table in [
                "subtitle_jobs",
                "subtitle_job_items",
                "subtitle_job_steps",
                "subtitle_job_events",
                "subtitle_job_files",
            ] {
                let mut stmt = c.prepare(&format!("SELECT * FROM {table}"))?;
                let columns = stmt.column_count();
                let mut rows = stmt.query([])?;
                while let Some(row) = rows.next()? {
                    for i in 0..columns {
                        if let Ok(Some(text)) = row.get::<_, Option<String>>(i) {
                            all.push_str(&text);
                            all.push('\n');
                        }
                    }
                }
            }
            Ok(all)
        })
        .await
        .unwrap();
    assert!(
        dump.contains("drive:1W0LRBy-gxCLtmDpQBd8kGYb79oh19YRe") && dump.contains("Koori 15.ass")
    );
    for secret in ["usercontent", "export=", "?", "NID"] {
        let hits: Vec<&str> = dump.lines().filter(|l| l.contains(secret)).collect();
        assert!(hits.is_empty(), "{secret}: {hits:?}");
    }
}

/// Receives one real individual file (C소라 얼음 성벽 2기 15화, with the
/// post's fonts) and one real ZIP (별명따위 전생슬 4기 24화). Run by hand:
/// `cargo test -p trss-jobs --test it blogger:: -- --ignored --nocapture`.
#[tokio::test]
#[ignore = "reaches the real Blogger and Google Drive"]
async fn real_blogger_posts_are_received_from_drive() {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(dir.path().join("app.db")).await.unwrap();
    let store = Handles::new(db);
    let area = ReceiveArea::in_app_data(dir.path());
    let drive = Drive::new();
    let runner = Runner::new(
        store.run.clone(),
        Sources::none()
            .with_tistory(TistorySource::new(drive.clone()))
            .with_blogger(BloggerSource::new(drive)),
        area.clone(),
        trss_core::system_clock(),
    );
    let id = make(
        &store,
        "real",
        &[
            (
                "15",
                "https://csora556.blogspot.com/2026/10/2.html".to_owned(),
            ),
            (
                "24",
                "https://bluewater91.blogspot.com/2026/09/4-2496.html".to_owned(),
            ),
        ],
    )
    .await;
    runner.run_ready(&CancellationToken::new()).await.unwrap();

    let d = store.views.detail(&id).await.unwrap().unwrap();
    for e in d.events.iter().rev() {
        println!("{} {}", e.message, e.detail.as_deref().unwrap_or_default());
    }
    for f in d.items.iter().flat_map(|i| &i.files) {
        println!(
            "{} state={:?} size={:?} expected={:?} format={:?} sha256={:?} status={:?} type={:?} snapshot={:?}",
            f.name, f.state, f.size, f.expected_size, f.format, f.sha256, f.http_status,
            f.content_type, f.snapshot
        );
    }
    assert_eq!(d.row.state, JobState::Done);
    let formats: Vec<Option<Format>> = d
        .items
        .iter()
        .flat_map(|i| &i.files)
        .map(|f| f.format)
        .collect();
    assert!(formats.contains(&Some(Format::Ass)) && formats.contains(&Some(Format::Zip)));
}
