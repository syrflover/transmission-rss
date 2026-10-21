//! The wire shape of `/api/save-folder` (`super`). The suggestion's rule is
//! tested in `trss-collect` (`save_folder/tests.rs`).

use std::collections::BTreeSet;

use axum::{
    http::{Method, StatusCode},
    Router,
};
use serde_json::{json, Value};
use trss_core::Db;
use trss_library::discovery::{Scan, ScannedWork, WorkRead};

use crate::web::{state::AppState, testing};

/// An app whose collect folder holds `Show` (seasons 1 and 2) and whose
/// archive folder holds `Old Show` (season 1) and `Show` again.
async fn app() -> Router {
    let db = Db::open_blocking(":memory:").unwrap();
    let state = AppState::new(db);
    state
        .settings
        .put_collection(
            0,
            "/media/current/".to_owned(),
            Some("/media/shows".to_owned()),
        )
        .await
        .unwrap();
    for (folder, works) in [
        ("/media/current", vec![("Show", vec![1, 2])]),
        (
            "/media/shows",
            vec![("Old Show", vec![1]), ("Show", vec![1])],
        ),
    ] {
        let scan = Scan {
            works: works
                .into_iter()
                .map(|(name, seasons)| {
                    WorkRead::Read(ScannedWork {
                        dir_name: name.to_owned(),
                        seasons: seasons.into_iter().collect::<BTreeSet<_>>(),
                        files: Vec::new(),
                        unrecognized: Vec::new(),
                    })
                })
                .collect(),
        };
        let registered = state.library.folders().await.unwrap();
        state
            .library
            .add_folder(folder.to_owned(), scan, 100, &registered)
            .await
            .unwrap();
    }
    testing::api(&state)
}

async fn get(app: &Router, uri: &str) -> (StatusCode, String, Value) {
    testing::call_text(app, Method::GET, uri, None).await
}

#[tokio::test]
async fn the_works_of_the_two_folders_are_listed_with_their_titles() {
    let app = app().await;
    let (status, text, answer) = get(&app, "/api/save-folder/works").await;
    assert_eq!(status, StatusCode::OK, "{text}");
    assert_eq!(
        answer,
        json!({ "works": [
            { "name": "Show", "title": "Show", "archived": false },
            { "name": "Old Show", "title": "Old Show", "archived": true },
        ] })
    );
}

#[tokio::test]
async fn a_suggestion_names_the_folder_its_library_work_and_why() {
    let app = app().await;
    let (status, text, answer) = get(
        &app,
        "/api/save-folder/suggestion?release=old%20show%20S2&anime_no=7&start=2026-01-10",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{text}");
    assert_eq!(
        answer,
        json!({ "suggestion": {
            "work": "Old Show",
            "season": 2,
            "directory": "Old Show/Season 02",
            "library": { "name": "Old Show", "title": "Old Show", "archived": true },
            "reason": "릴리스 제목의 ‘S2’ 표시를 시즌 번호로 읽었어요.",
        } })
    );

    // A picked work, no anime: which season it gets is trss-collect's.
    let (status, text, answer) = get(&app, "/api/save-folder/suggestion?work=Show").await;
    assert_eq!(status, StatusCode::OK, "{text}");
    assert_eq!(answer["suggestion"]["work"], "Show");
    assert_eq!(answer["suggestion"]["library"]["archived"], false);

    let (status, text, answer) = get(&app, "/api/save-folder/suggestion?release=%3A%2F").await;
    assert_eq!(status, StatusCode::OK, "{text}");
    assert_eq!(answer, json!({ "suggestion": null }));
}

#[tokio::test]
async fn a_suggestion_asks_for_a_release_or_a_work_not_both() {
    let app = app().await;
    for query in [
        "",
        "?release=A&work=B",
        "?release=%20",
        "?work=A&anime_no=x",
    ] {
        let (status, text, answer) =
            get(&app, &format!("/api/save-folder/suggestion{query}")).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{query}: {text}");
        assert_eq!(answer["error"], "invalid", "{query}");
    }
}
