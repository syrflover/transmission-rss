//! The resident memory an upload's body takes while it is read, for a body
//! that arrives in many chunks. Linux only (it reads `/proc/self`), and the
//! only test of this file so nothing else allocates while it measures.
#![cfg(target_os = "linux")]

use axum::{
    body::Body,
    http::{header, Method, Request},
};
use tower::ServiceExt;
use transmission_rss::{
    artwork::{AnilistConfig, AppData, Artwork, MAX_IMAGE_BYTES},
    store::Db,
    web::{self, AppState},
};

/// A `kB` field of `/proc/self/status`, in bytes.
fn status_bytes(field: &str) -> u64 {
    let status = std::fs::read_to_string("/proc/self/status").unwrap();
    let line = status.lines().find(|l| l.starts_with(field)).unwrap();
    let kib: u64 = line.split_whitespace().nth(1).unwrap().parse().unwrap();
    kib * 1024
}

#[tokio::test]
async fn an_upload_arriving_in_chunks_is_held_once() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("index.html"), "<html></html>").unwrap();
    let db = Db::open(dir.path().join("trss.db")).await.unwrap();
    let artwork = Artwork::new(
        db.clone(),
        Some(AppData::new(dir.path())),
        AnilistConfig::default(),
    );
    let router = web::router(dir.path(), AppState::new(db).with_artwork(artwork));

    // A body of the byte limit in 10 KiB chunks, each made when it is asked
    // for and dropped once read. It is no image and no work's: the answer is
    // an error, after the body was read.
    let chunk = 10 * 1024;
    let count = MAX_IMAGE_BYTES / chunk;
    let body =
        Body::from_stream(futures::stream::iter((0..count).map(move |i| {
            Ok::<_, std::io::Error>(bytes::Bytes::from(vec![i as u8; chunk]))
        })));
    let request = Request::builder()
        .method(Method::POST)
        .uri("/api/library/works/none/artwork/upload?version=1")
        .header(header::CONTENT_LENGTH, count * chunk)
        .body(body)
        .unwrap();

    // Resets the peak (`VmHWM`) to the current resident set.
    std::fs::write("/proc/self/clear_refs", "5").unwrap();
    let before = status_bytes("VmRSS:");
    let response = router.oneshot(request).await.unwrap();
    let peak = status_bytes("VmHWM:").saturating_sub(before);

    assert_ne!(response.status(), axum::http::StatusCode::OK);
    let size = (count * chunk) as u64;
    assert!(
        peak < size + size / 2,
        "reading a body of {size} bytes peaked at {peak} bytes above where it started"
    );
}
