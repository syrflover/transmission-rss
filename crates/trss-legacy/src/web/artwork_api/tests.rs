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
    artwork::{fake::Fake, image::samples, AppData, MAX_IMAGE_BYTES, SERVING_BUDGET},
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
    let (status, headers, _) = call(&env.state, Method::GET, &url, Body::empty(), &[]).await;
    assert_eq!(status, StatusCode::OK);
    let etag = headers[header::ETAG].to_str().unwrap().to_owned();
    std::fs::write(&path, b"not the image").unwrap();
    let (status, _, _) = call(&env.state, Method::GET, &url, Body::empty(), &[]).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    // A browser holding the old image is not told it is still good.
    let (status, _, _) = call(
        &env.state,
        Method::GET,
        &url,
        Body::empty(),
        &[(header::IF_NONE_MATCH, &etag)],
    )
    .await;
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

#[tokio::test]
async fn a_stalled_upload_body_times_out_and_gives_its_slot_back() {
    let mut env = env().await;
    env.state.artwork = env
        .state
        .artwork
        .clone()
        .with_body_timeout(Duration::from_millis(200));
    let uri = format!("{}/upload?version=1", base(&env));
    let stalled = || {
        // One chunk, then nothing: the body never ends.
        let body = Body::from_stream(
            futures::stream::once(async {
                Ok::<_, std::io::Error>(bytes::Bytes::from_static(b"\x89PNG"))
            })
            .chain(futures::stream::pending()),
        );
        call(
            &env.state,
            Method::POST,
            &uri,
            body,
            &[(header::CONTENT_TYPE, "image/png")],
        )
    };
    // Every slot is held by a stalled body, and one more upload waits behind
    // them: it is read after they give their slots back.
    let waiting = async {
        tokio::time::sleep(Duration::from_millis(50)).await;
        call(
            &env.state,
            Method::POST,
            &uri,
            Body::from(samples::png()),
            &[(header::CONTENT_TYPE, "image/png")],
        )
        .await
    };
    let (first, second, third) = tokio::time::timeout(Duration::from_secs(10), async {
        futures::join!(stalled(), stalled(), waiting)
    })
    .await
    .expect("the stalled bodies must not hold their slots for ever");
    for (status, _, body) in [first, second] {
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let body: Value = serde_json::from_slice(&body).unwrap();
        assert!(
            body["message"].as_str().unwrap().contains("오래 걸려요"),
            "{body}"
        );
    }
    assert_eq!(
        third.0,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&third.2)
    );
}

mod reading_a_body {
    use futures::StreamExt;

    use super::*;
    use crate::web::artwork_api::{read_body, BodyError};

    fn chunks(parts: &[&'static [u8]]) -> Body {
        Body::from_stream(futures::stream::iter(
            parts
                .iter()
                .map(|p| Ok::<_, std::io::Error>(bytes::Bytes::from_static(p)))
                .collect::<Vec<_>>(),
        ))
    }

    #[tokio::test]
    async fn chunks_are_joined_in_order_into_one_buffer() {
        let body = chunks(&[b"abc", b"", b"defg", b"h"]);
        let bytes = read_body(body, Some(8), 8).await.unwrap();
        assert_eq!(&bytes[..], b"abcdefgh");
        // Without a length, or with a wrong one, the buffer still grows to fit.
        let bytes = read_body(chunks(&[b"abc", b"def"]), None, 10)
            .await
            .unwrap();
        assert_eq!(&bytes[..], b"abcdef");
        let bytes = read_body(chunks(&[b"abc", b"def"]), Some(1), 10)
            .await
            .unwrap();
        assert_eq!(&bytes[..], b"abcdef");
        assert!(read_body(Body::empty(), Some(0), 10)
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn the_limit_holds_for_what_is_declared_and_for_what_arrives() {
        // Exactly the limit is taken.
        assert!(read_body(chunks(&[b"abcd", b"efgh"]), Some(8), 8)
            .await
            .is_ok());
        // A declared length over the limit is refused before the body is read.
        let read = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = read.clone();
        let body = Body::from_stream(futures::stream::once(async move {
            flag.store(true, std::sync::atomic::Ordering::SeqCst);
            Ok::<_, std::io::Error>(bytes::Bytes::from_static(b"x"))
        }));
        assert_eq!(read_body(body, Some(9), 8).await, Err(BodyError::TooLarge));
        assert!(!read.load(std::sync::atomic::Ordering::SeqCst));
        // A body longer than it declared, or with no declaration, is cut at
        // the limit.
        assert_eq!(
            read_body(chunks(&[b"abcd", b"efghi"]), Some(2), 8).await,
            Err(BodyError::TooLarge)
        );
        assert_eq!(
            read_body(chunks(&[b"abcd", b"efghi"]), None, 8).await,
            Err(BodyError::TooLarge)
        );
    }

    #[tokio::test]
    async fn a_body_that_fails_midway_is_not_taken() {
        let body = Body::from_stream(
            futures::stream::iter([Ok(bytes::Bytes::from_static(b"abc"))])
                .chain(futures::stream::iter([Err(std::io::Error::other("reset"))])),
        );
        assert_eq!(read_body(body, None, 8).await, Err(BodyError::Broken));
    }
}

/// Makes the work's cover a PNG-looking file of `size` bytes (serving looks at
/// the size, the hash and the first bytes only) and returns the image URL.
async fn plant_cover(env: &Env, size: usize) -> String {
    use crate::{
        artwork::files::{image_ref, ARTWORK_DIR},
        store::artwork::{Format, Source},
    };
    let mut bytes = samples::png();
    bytes.resize(size, 0);
    let root = env.state.artwork.app_data().unwrap().root().to_owned();
    std::fs::create_dir_all(root.join(ARTWORK_DIR)).unwrap();
    let relative = format!("{ARTWORK_DIR}/planted.png");
    std::fs::write(root.join(&relative), &bytes).unwrap();
    let image = image_ref(Source::Upload, relative, &bytes, Format::Png);
    let url = image_url(&env.work, &image.id);
    let store = &env.state.artwork.store;
    store
        .reserve_file(&image.relative_path, "artwork/.staging/planted.tmp", 1)
        .await
        .unwrap();
    store
        .select_manual(&env.work, 1, None, image)
        .await
        .unwrap();
    url.strip_prefix("/api").unwrap().to_owned()
}

fn get(uri: &str) -> Request<Body> {
    Request::builder()
        .method(Method::GET)
        .uri(uri)
        .body(Body::empty())
        .unwrap()
}

#[tokio::test]
async fn a_response_holds_its_room_until_its_body_is_sent_or_dropped() {
    let env = env().await;
    let size = 3 * 1024 * 1024;
    let url = plant_cover(&env, size).await;
    let taken = || SERVING_BUDGET - env.state.artwork.serving_free();
    let router = || api::router().with_state(env.state.clone());

    // The answer is ready but its body is not sent: the bytes are still held.
    let response = router().oneshot(get(&url)).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(taken(), size);
    // Dropped unsent (what the server does when the client went away).
    drop(response);
    assert_eq!(taken(), 0);

    // Sent: read to its end, and let go with the last of the bytes.
    let response = router().oneshot(get(&url)).await.unwrap();
    assert_eq!(taken(), size);
    let mut body = response.into_body().into_data_stream();
    let mut sent = 0;
    while let Some(chunk) = body.next().await {
        sent += chunk.unwrap().len();
    }
    assert_eq!(sent, size);
    drop(body);
    assert_eq!(taken(), 0);
}

#[tokio::test]
async fn a_slow_client_holds_its_room_and_a_client_that_leaves_gives_it_back() {
    use tokio::{io::AsyncWriteExt, net::TcpSocket};

    let env = env().await;
    let size = MAX_IMAGE_BYTES;
    let url = plant_cover(&env, size).await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = api::router().with_state(env.state.clone());
    let server = tokio::spawn(async move { axum::serve(listener, app).await });

    // A client that asks and never reads: its small receive window leaves most
    // of the 10 MiB with the server.
    let socket = TcpSocket::new_v4().unwrap();
    socket.set_recv_buffer_size(4096).unwrap();
    let mut client = socket.connect(addr).await.unwrap();
    client
        .write_all(format!("GET {url} HTTP/1.1\r\nHost: x\r\n\r\n").as_bytes())
        .await
        .unwrap();
    let taken = || SERVING_BUDGET - env.state.artwork.serving_free();
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(taken(), size, "the bytes are still with the server");

    // Gone: the connection fails, the server drops the response unsent, and
    // the room comes back.
    drop(client);
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while taken() != 0 {
        assert!(
            std::time::Instant::now() < deadline,
            "a client that left kept {} bytes",
            taken()
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    server.abort();
}
