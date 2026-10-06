use std::collections::BTreeSet;

use axum::{
    body::Body,
    http::{Method, Request, StatusCode},
};
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;

use crate::{api, AppState};
use trss_core::Db;
use trss_library::discovery::{
    EpisodeFile, FileKind, Reason, Scan, ScannedWork, Unrecognized, WorkRead,
};

async fn get(state: &AppState, uri: &str) -> (StatusCode, Value) {
    let request = Request::builder()
        .method(Method::GET)
        .uri(uri)
        .body(Body::empty())
        .unwrap();
    let response = api::router()
        .with_state(state.clone())
        .oneshot(request)
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap())
}

fn state() -> AppState {
    AppState::new(Db::open_blocking(":memory:").unwrap())
}

fn file(episode: &str, kind: FileKind) -> EpisodeFile {
    let ext = if kind == FileKind::Video {
        "mkv"
    } else {
        "ass"
    };
    EpisodeFile {
        path: format!("Season 01/S01E{episode}.{ext}"),
        kind,
        season: 1,
        episode: episode.to_owned(),
    }
}

fn work(name: &str, files: Vec<EpisodeFile>, unrecognized: Vec<Unrecognized>) -> WorkRead {
    WorkRead::Read(ScannedWork {
        dir_name: name.to_owned(),
        seasons: BTreeSet::from([1]),
        files,
        unrecognized,
    })
}

async fn add_folder(state: &AppState, path: &str, works: Vec<WorkRead>) {
    let registered = state.library.folders().await.unwrap();
    state
        .library
        .add_folder(path.into(), Scan { works }, 100, &registered)
        .await
        .unwrap();
}

/// `count` works named `Work 000`, `Work 001`, … of a plain video each.
fn plain(count: usize) -> Vec<WorkRead> {
    (0..count)
        .map(|n| {
            work(
                &format!("Work {n:03}"),
                vec![file("01", FileKind::Video)],
                vec![],
            )
        })
        .collect()
}

fn names(body: &Value) -> Vec<&str> {
    body["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w["name"].as_str().unwrap())
        .collect()
}

#[tokio::test]
async fn the_first_page_has_sixty_works_a_cursor_and_the_counts() {
    let state = state();
    add_folder(&state, "/a", plain(130)).await;

    let (status, body) = get(&state, "/library/works").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["items"].as_array().unwrap().len(), 60);
    assert_eq!(body["total"], 130);
    assert_eq!(body["library_count"], 130);
    assert!(body["next"].is_string());
    let first = &body["items"][0];
    assert_eq!(first["name"], "Work 000");
    assert_eq!(first["watch_folder"]["path"], "/a");
    assert_eq!(first["video"][0]["first"], "01");
    assert_eq!(first["subtitle_coverage"], "none");
}

#[tokio::test]
async fn following_next_reaches_every_work_once_for_every_sort_and_the_last_page_has_no_next() {
    let state = state();
    add_folder(&state, "/a", plain(130)).await;
    for sort in ["title", "year", "added", "video", "subtitle"] {
        let mut seen: Vec<String> = Vec::new();
        let mut after = String::new();
        let mut pages = 0;
        loop {
            let (status, body) = get(
                &state,
                &format!("/library/works?sort={sort}&limit=50&after={after}"),
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{body}");
            pages += 1;
            seen.extend(names(&body).iter().map(|s| s.to_string()));
            match body["next"].as_str() {
                Some(next) => after = next.to_owned(),
                None => break,
            }
        }
        assert_eq!(pages, 3, "{sort}");
        let unique: BTreeSet<&String> = seen.iter().collect();
        assert_eq!((seen.len(), unique.len()), (130, 130), "{sort}");
    }
}

#[tokio::test]
async fn a_work_added_while_paging_does_not_repeat_what_was_seen() {
    let state = state();
    add_folder(&state, "/a", plain(30)).await;
    let (_, first) = get(&state, "/library/works?sort=title&limit=10").await;
    let next = first["next"].as_str().unwrap().to_owned();
    // A work that sorts before and one that sorts after the cursor.
    add_folder(
        &state,
        "/b",
        vec![
            work("A first", vec![file("01", FileKind::Video)], vec![]),
            work("Zzz last", vec![file("01", FileKind::Video)], vec![]),
        ],
    )
    .await;
    let (_, rest) = get(
        &state,
        &format!("/library/works?sort=title&limit=100&after={next}"),
    )
    .await;
    let later = names(&rest);
    let seen = names(&first);
    assert!(later.iter().all(|n| !seen.contains(n)));
    assert!(!later.contains(&"A first"));
    assert_eq!(later.last(), Some(&"Zzz last"));
    assert_eq!(seen.len() + later.len(), 31);
    assert_eq!(rest["total"], 32);
}

#[tokio::test]
async fn filter_and_search_narrow_the_pages_and_the_total() {
    let state = state();
    let mut works = plain(12);
    works.push(work(
        "Complete One",
        vec![file("01", FileKind::Video), file("01", FileKind::Subtitle)],
        vec![],
    ));
    works.push(work(
        "Partial Two",
        vec![
            file("01", FileKind::Video),
            file("02", FileKind::Video),
            file("01", FileKind::Subtitle),
        ],
        vec![],
    ));
    works.push(work(
        "Checking",
        vec![file("01", FileKind::Video)],
        vec![Unrecognized {
            path: "Season 01/x.srt".into(),
            reason: Reason::NoEpisode,
            check: None,
        }],
    ));
    add_folder(&state, "/a", works).await;

    let (_, body) = get(&state, "/library/works?filter=complete").await;
    assert_eq!(names(&body), ["Complete One"]);
    assert_eq!(
        (body["total"].as_u64(), body["library_count"].as_u64()),
        (Some(1), Some(15))
    );
    let (_, body) = get(&state, "/library/works?filter=partial").await;
    assert_eq!(names(&body), ["Partial Two"]);
    let (_, body) = get(&state, "/library/works?filter=check").await;
    assert_eq!(names(&body), ["Checking"]);
    let (_, body) = get(&state, "/library/works?filter=none").await;
    assert_eq!(body["total"], 13);
    let (_, body) = get(&state, "/library/works?filter=airing").await;
    assert_eq!(names(&body), Vec::<&str>::new());
    assert_eq!(body["next"], Value::Null);
    assert_eq!(body["library_count"], 15);

    // Case does not matter and the search and the filter combine; the text is URL-encoded.
    let (_, body) = get(&state, "/library/works?q=TWO&filter=partial").await;
    assert_eq!(names(&body), ["Partial Two"]);
    let (_, body) = get(&state, "/library/works?q=%20two%20&filter=complete").await;
    assert_eq!(body["total"], 0);
    let (_, body) = get(&state, "/library/works?q=work%20001").await;
    assert_eq!(names(&body), ["Work 001"]);
}

#[tokio::test]
async fn a_bad_parameter_is_a_400_with_a_sentence() {
    let state = state();
    add_folder(&state, "/a", plain(3)).await;
    let (_, page) = get(&state, "/library/works?sort=added&limit=1").await;
    let cursor = page["next"].as_str().unwrap().to_owned();

    for uri in [
        "/library/works?sort=newest".to_owned(),
        "/library/works?filter=everything".to_owned(),
        "/library/works?limit=0".to_owned(),
        "/library/works?limit=201".to_owned(),
        "/library/works?limit=-1".to_owned(),
        "/library/works?limit=many".to_owned(),
        "/library/works?after=not-a-cursor".to_owned(),
        // A cursor belongs to the sort that made it.
        format!("/library/works?sort=title&after={cursor}"),
        format!("/library/works?after={cursor}"),
    ] {
        let (status, body) = get(&state, &uri).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{uri}: {body}");
        assert_eq!(body["error"], "invalid", "{uri}");
        assert!(
            body["message"].as_str().unwrap().ends_with("요."),
            "{uri}: {body}"
        );
    }
    let (status, _) = get(&state, &format!("/library/works?sort=added&after={cursor}")).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = get(&state, "/library/works?limit=200").await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn an_empty_library_answers_an_empty_page() {
    let state = state();
    let (status, body) = get(&state, "/library/works").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(names(&body), Vec::<&str>::new());
    assert_eq!(
        (&body["next"], &body["total"], &body["library_count"]),
        (&Value::Null, &Value::from(0), &Value::from(0))
    );
}

/// A pick's job of `work` that waits for `wait` (`auth`, `approval`, `placement`).
async fn waiting(state: &AppState, n: usize, work: Option<&str>, wait: &'static str) {
    use trss_jobs::{Created, NewItem, NewJob};
    let made = state
        .jobs
        .create(
            NewJob {
                command_id: format!("c{n}"),
                request: "{}".into(),
                origin: "pick".into(),
                work_id: work.map(str::to_owned),
                season: Some(1),
                anime_no: None,
                source_id: None,
                creator: None,
                revision_of: None,
                revises_attributed: false,
                items: vec![NewItem {
                    observation_id: None,
                    episode: "1".into(),
                    post_url: format!("https://post.test/{n}"),
                    found_at: 1,
                }],
            },
            100 + n as i64,
        )
        .await
        .unwrap();
    let Created::Created(id) = made else { panic!() };
    state
        .jobs
        .db()
        .run::<_, trss_core::DbError, _>(move |c| {
            c.execute(
                "UPDATE subtitle_jobs SET state = 'waiting', wait = ?1 WHERE id = ?2",
                [wait, id.as_str()],
            )?;
            Ok(())
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn a_page_badges_each_work_with_its_to_dos_in_the_to_do_order() {
    let state = state();
    add_folder(&state, "/media/anime", plain(3)).await;
    let (_, first) = get(&state, "/library/works?sort=title").await;
    let id = |n: usize| first["items"][n]["id"].as_str().unwrap().to_owned();
    let (a, b) = (id(0), id(1));
    // The replacement waits from before the check, which still comes first.
    waiting(&state, 1, Some(&a), "approval").await;
    waiting(&state, 2, Some(&a), "auth").await;
    waiting(&state, 3, Some(&a), "auth").await;
    waiting(&state, 4, Some(&b), "placement").await;
    // A job of no work badges nothing.
    waiting(&state, 5, None, "auth").await;

    let (status, page) = get(&state, "/library/works?sort=title").await;

    assert_eq!(status, StatusCode::OK);
    let todos: Vec<(&str, &Value)> = page["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| (w["name"].as_str().unwrap(), &w["todos"]))
        .collect();
    assert_eq!(
        todos,
        [
            ("Work 000", &serde_json::json!(["auth", "replacement"])),
            ("Work 001", &serde_json::json!(["episode_check"])),
            ("Work 002", &serde_json::json!([])),
        ]
    );
}
