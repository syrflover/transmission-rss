use std::{collections::BTreeSet, time::Duration};

use axum::{
    body::Body,
    http::{header, Method, Request, StatusCode},
};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tempfile::TempDir;
use tower::ServiceExt;

use super::*;
use crate::{
    artwork::{fake::Fake, image::samples, AppData, MAX_IMAGE_BYTES},
    discovery::{Scan, ScannedWork, WorkRead},
    store::Db,
    web::api,
};

struct Env {
    _dir: TempDir,
    state: AppState,
    fake: Fake,
    work: String,
}

async fn env() -> Env {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(dir.path().join("trss.db")).await.unwrap();
    let fake = Fake::start().await;
    let state = AppState::new(db.clone()).with_artwork(
        Artwork::new(db, Some(AppData::new(dir.path())), fake.config())
            .with_spacing(Duration::ZERO),
    );
    let scan = Scan {
        works: vec![WorkRead::Read(ScannedWork {
            dir_name: "Lycoris Recoil".into(),
            seasons: BTreeSet::new(),
            files: Vec::new(),
            unrecognized: Vec::new(),
        })],
    };
    let (folder, _) = state
        .library
        .add_folder("/c".into(), scan, 1, &[])
        .await
        .unwrap();
    let work = state.library.works(&folder.id).await.unwrap()[0].id.clone();
    Env {
        _dir: dir,
        state,
        fake,
        work,
    }
}

async fn call(
    state: &AppState,
    method: Method,
    uri: &str,
    body: Body,
    headers: &[(header::HeaderName, &str)],
) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
    let mut request = Request::builder().method(method).uri(uri);
    for (name, value) in headers {
        request = request.header(name, *value);
    }
    let response = api::router()
        .with_state(state.clone())
        .oneshot(request.body(body).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, headers, bytes.to_vec())
}

async fn json_call(
    state: &AppState,
    method: Method,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let (body, headers): (Body, &[(header::HeaderName, &str)]) = match body {
        Some(v) => (
            Body::from(v.to_string()),
            &[(header::CONTENT_TYPE, "application/json")],
        ),
        None => (Body::empty(), &[]),
    };
    let (status, _, bytes) = call(state, method, uri, body, headers).await;
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

fn base(env: &Env) -> String {
    format!("/library/works/{}/artwork", env.work)
}

async fn upload(
    env: &Env,
    version: i64,
    bytes: Vec<u8>,
    content_type: &str,
) -> (StatusCode, Value) {
    let (status, _, body) = call(
        &env.state,
        Method::POST,
        &format!("{}/upload?version={version}", base(env)),
        Body::from(bytes),
        &[(header::CONTENT_TYPE, content_type)],
    )
    .await;
    (status, serde_json::from_slice(&body).unwrap())
}

#[tokio::test]
async fn the_state_of_a_new_work_is_auto_with_a_search_pending() {
    let env = env().await;
    let (status, body) = json_call(&env.state, Method::GET, &base(&env), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["mode"], "auto");
    assert_eq!(body["source"], Value::Null);
    assert_eq!(body["image"], Value::Null);
    assert_eq!(body["pending"], "search");
    let (status, _) = json_call(&env.state, Method::GET, "/library/works/nope/artwork", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    // No image: the image answers 404, the list and the detail have no URL.
    let (status, _, _) = call(
        &env.state,
        Method::GET,
        &format!("{}/image", base(&env)),
        Body::empty(),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (_, list) = json_call(&env.state, Method::GET, "/library/works", None).await;
    assert_eq!(list["items"][0]["cover_url"], Value::Null);
}

#[tokio::test]
async fn an_upload_is_judged_by_its_bytes_not_its_name_or_type() {
    let env = env().await;
    // A JPEG sent as a PNG.
    let (status, body) = upload(&env, 1, samples::jpeg(), "image/png").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["mode"], "manual");
    assert_eq!(body["source"], "upload");
    assert_eq!(body["image"]["format"], "jpeg");
    assert_eq!(body["image"]["status"], "available");
    let version = body["version"].as_i64().unwrap();
    let url = body["image"]["url"].as_str().unwrap().to_owned();
    let path = url.strip_prefix("/api").unwrap().to_owned();

    // Text sent as a JPEG, and a file over the limit: refused with the reason,
    // the cover stays.
    let (status, refused) = upload(&env, version, b"hello".to_vec(), "image/jpeg").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(refused["message"]
        .as_str()
        .unwrap()
        .contains("JPEG·PNG·WebP"));
    let mut big = samples::png();
    big.resize(MAX_IMAGE_BYTES + 1, 0);
    let (status, refused) = upload(&env, version, big, "application/octet-stream").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(refused["message"].as_str().unwrap().contains("10MB"));
    let (_, now) = json_call(&env.state, Method::GET, &base(&env), None).await;
    assert_eq!(now, body);

    // The image is served checked, revalidated each time.
    let (status, headers, bytes) = call(&env.state, Method::GET, &path, Body::empty(), &[]).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(bytes, samples::jpeg());
    assert_eq!(headers[header::CONTENT_TYPE], "image/jpeg");
    assert_eq!(headers[header::CACHE_CONTROL], "no-cache");
    assert_eq!(headers[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
    let etag = headers[header::ETAG].to_str().unwrap().to_owned();
    let (status, _, bytes) = call(
        &env.state,
        Method::GET,
        &path,
        Body::empty(),
        &[(header::IF_NONE_MATCH, &etag)],
    )
    .await;
    assert_eq!(status, StatusCode::NOT_MODIFIED);
    assert!(bytes.is_empty());

    // The list and the detail carry the URL.
    let (_, list) = json_call(&env.state, Method::GET, "/library/works", None).await;
    assert_eq!(list["items"][0]["cover_url"], url.as_str());
    let (_, detail) = json_call(
        &env.state,
        Method::GET,
        &format!("/library/works/{}", env.work),
        None,
    )
    .await;
    assert_eq!(detail["cover_url"], url.as_str());
}

#[tokio::test]
async fn an_image_whose_file_changed_is_not_served_and_its_state_says_why() {
    let env = env().await;
    let (_, body) = upload(&env, 1, samples::png(), "image/png").await;
    let url = body["image"]["url"]
        .as_str()
        .unwrap()
        .strip_prefix("/api")
        .unwrap()
        .to_owned();
    let selection = env.state.artwork.store.selection(&env.work).await.unwrap();
    let path = env
        .state
        .artwork
        .app_data()
        .unwrap()
        .root()
        .join(&selection.image.unwrap().relative_path);
    std::fs::write(&path, b"not the image").unwrap();
    let (status, _, _) = call(&env.state, Method::GET, &url, Body::empty(), &[]).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (_, now) = json_call(&env.state, Method::GET, &base(&env), None).await;
    assert_eq!(now["image"]["status"], "mismatch");
    assert_eq!(now["mode"], "manual", "the reference stays");
    std::fs::remove_file(&path).unwrap();
    let (_, now) = json_call(&env.state, Method::GET, &base(&env), None).await;
    assert_eq!(now["image"]["status"], "missing");
}

#[tokio::test]
async fn a_change_from_an_old_version_is_a_conflict_with_the_current_state() {
    let env = env().await;
    let (_, first) = upload(&env, 1, samples::png(), "image/png").await;
    let (status, body) = upload(&env, 1, samples::jpeg(), "image/jpeg").await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["current"], first);
    let (status, body) = json_call(
        &env.state,
        Method::POST,
        &format!("{}/clear", base(&env)),
        Some(json!({ "version": 1 })),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["current"]["mode"], "manual");

    // Clear, then back to auto with a new search.
    let v = first["version"].as_i64().unwrap();
    let (status, cleared) = json_call(
        &env.state,
        Method::POST,
        &format!("{}/clear", base(&env)),
        Some(json!({ "version": v })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        (&cleared["mode"], &cleared["image"], &cleared["pending"]),
        (&json!("disabled"), &Value::Null, &Value::Null)
    );
    let (status, auto) = json_call(
        &env.state,
        Method::POST,
        &format!("{}/auto", base(&env)),
        Some(json!({ "version": cleared["version"] })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        (&auto["mode"], &auto["pending"]),
        (&json!("auto"), &json!("search"))
    );
    // An upload cannot be repaired.
    let (status, body) = json_call(
        &env.state,
        Method::POST,
        &format!("{}/repair", base(&env)),
        Some(json!({ "version": auto["version"] })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["message"].as_str().unwrap().contains("AniList"));
}

#[tokio::test]
async fn search_and_pick_go_through_anilist_and_its_image_host_only() {
    let env = env().await;
    env.fake.add_search(
        "Lycoris Recoil",
        vec![
            env.fake.entry(1, "Lycoris Recoil", &[]),
            env.fake.entry(2, "Lycoris Recoil Season 2", &[]),
        ],
        &samples::webp(),
    );
    // Without a query: the folder name.
    let (status, found) = json_call(
        &env.state,
        Method::POST,
        &format!("{}/search", base(&env)),
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{found}");
    assert_eq!(found["has_next"], false);
    assert_eq!(found["items"][0]["id"], 1);
    assert_eq!(found["items"][0]["title"], "Lycoris Recoil");
    assert_eq!(
        found["items"][1]["thumb_url"],
        format!("{}/img/2.jpg", env.fake.origin)
    );

    let (status, picked) = json_call(
        &env.state,
        Method::POST,
        &format!("{}/pick", base(&env)),
        Some(json!({ "version": 1, "anilist_media_id": 2 })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{picked}");
    assert_eq!(
        (
            &picked["mode"],
            &picked["source"],
            &picked["anilist_media_id"]
        ),
        (&json!("manual"), &json!("anilist"), &json!(2))
    );
    assert_eq!(picked["image"]["format"], "webp");
    assert_eq!(
        picked["pending"],
        Value::Null,
        "the user's choice ends the search"
    );

    // An entry whose cover is on another host is not fetched.
    env.fake.state.lock().unwrap().media.insert(
        3,
        json!({ "id": 3, "title": { "romaji": "X" }, "synonyms": [],
                "coverImage": { "extraLarge": "https://evil.example/x.jpg" } }),
    );
    let (status, body) = json_call(
        &env.state,
        Method::POST,
        &format!("{}/pick", base(&env)),
        Some(json!({ "version": picked["version"], "anilist_media_id": 3 })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["message"]
        .as_str()
        .unwrap()
        .contains("표지 이미지가 없어요"));
    // An ID AniList does not have.
    let (status, _) = json_call(
        &env.state,
        Method::POST,
        &format!("{}/pick", base(&env)),
        Some(json!({ "version": picked["version"], "anilist_media_id": 999 })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (_, now) = json_call(&env.state, Method::GET, &base(&env), None).await;
    assert_eq!(now, picked);

    // AniList busy: the user is told to wait, nothing changes.
    {
        let mut fake = env.fake.state.lock().unwrap();
        fake.rate_limited = 1;
        fake.retry_after = 45;
    }
    let (status, body) = json_call(
        &env.state,
        Method::POST,
        &format!("{}/search", base(&env)),
        Some(json!({ "q": "Lycoris" })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        body["message"].as_str().unwrap().contains("초 뒤에"),
        "{body}"
    );
}

#[tokio::test]
async fn an_upload_waits_for_a_slot_before_it_reads_its_body() {
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };

    let env = env().await;
    let held: Vec<_> = futures::future::join_all(
        (0..crate::artwork::UPLOAD_SLOTS).map(|_| env.state.artwork.upload_slot()),
    )
    .await;
    let read = Arc::new(AtomicBool::new(false));
    let flag = read.clone();
    let body = Body::from_stream(futures::stream::once(async move {
        flag.store(true, Ordering::SeqCst);
        Ok::<_, std::io::Error>(bytes::Bytes::from(samples::png()))
    }));
    let uri = format!("{}/upload?version=1", base(&env));
    let request = call(
        &env.state,
        Method::POST,
        &uri,
        body,
        &[(header::CONTENT_TYPE, "image/png")],
    );
    tokio::pin!(request);
    // Every slot is taken: the body is not read, nothing is stored.
    assert!(
        tokio::time::timeout(Duration::from_millis(300), &mut request)
            .await
            .is_err()
    );
    assert!(!read.load(Ordering::SeqCst));
    drop(held);
    let (status, _, body) = request.await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    assert!(read.load(Ordering::SeqCst));
}
