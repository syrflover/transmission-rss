//! The resident memory a subtitle upload takes while its body is read: a file
//! that arrives in many chunks is written to the disk as it comes, never held
//! whole (a test that fed the body unrested peaked at the size of the file),
//! and a body that is all one part header, or all a preamble, is refused after
//! a small backlog (the multipart reader alone held 511 MiB of a header that
//! never ended). Linux only (it reads `/proc/self`), and the only test of this
//! file so nothing else allocates while it measures.
#![cfg(target_os = "linux")]

use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};

use axum::{
    body::Body,
    http::{header, Method, Request, StatusCode},
};
use tower::ServiceExt;
use trss_core::{Db, DbError};
use trss_jobs::ReceiveArea;
use trss_web::{self as web, AppState};

/// A `kB` field of `/proc/self/status`, in bytes.
fn status_bytes(field: &str) -> u64 {
    let status = std::fs::read_to_string("/proc/self/status").unwrap();
    let line = status.lines().find(|l| l.starts_with(field)).unwrap();
    let kib: u64 = line.split_whitespace().nth(1).unwrap().parse().unwrap();
    kib * 1024
}

#[tokio::test]
async fn bodies_arriving_in_chunks_are_not_held_whatever_they_hold() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("index.html"), "<html></html>").unwrap();
    let db = Db::open(dir.path().join("trss.db")).await.unwrap();
    db.run::<_, DbError, _>(|c| {
        Ok(c.execute_batch(
            "INSERT INTO watch_folders (id, path, created_at) VALUES ('f1', '/media', 1);
             INSERT INTO works (id, watch_folder_id, dir_name) VALUES ('w1', 'f1', 'Show');
             INSERT INTO seasons (work_id, number) VALUES ('w1', 1);",
        )?)
    })
    .await
    .unwrap();
    let area = ReceiveArea::in_app_data(dir.path());
    let router = web::router(dir.path(), AppState::new(db).with_receive_area(&area));

    let boundary = "----peak";
    let request_of = |body: Body| {
        Request::builder()
            .method(Method::POST)
            .uri("/api/subtitle-jobs/upload")
            .header(
                header::CONTENT_TYPE,
                format!("multipart/form-data; boundary={boundary}"),
            )
            .body(body)
            .unwrap()
    };
    // `head`, then `count` chunks of 64 KiB made when they are asked for and
    // dropped once read, then `tail`. `pulled` counts what was handed out.
    let chunk = 64 * 1024;
    let stream = |head: String, count: usize, tail: String, pulled: &Arc<AtomicU64>| {
        let pulled = Arc::clone(pulled);
        let pieces = std::iter::once(bytes::Bytes::from(head))
            .chain((0..count).map(move |_| bytes::Bytes::from(vec![b'x'; chunk])))
            .chain(std::iter::once(bytes::Bytes::from(tail)));
        Body::from_stream(futures::stream::iter(pieces.map(move |piece| {
            pulled.fetch_add(piece.len() as u64, Ordering::SeqCst);
            Ok::<_, std::io::Error>(piece)
        })))
    };
    // The peak (`VmHWM`) above where the resident set was, for one request.
    let measure = |request: Request<Body>| {
        let router = router.clone();
        async move {
            std::fs::write("/proc/self/clear_refs", "5").unwrap();
            let before = status_bytes("VmRSS:");
            let response = router.oneshot(request).await.unwrap();
            (
                response.status(),
                status_bytes("VmHWM:").saturating_sub(before),
            )
        }
    };
    let limit = 16 * 1024 * 1024;

    // A subtitle of 128 MiB (the largest a file may be is 200 MiB).
    let pulled = Arc::new(AtomicU64::new(0));
    let head = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"id\"\r\n\r\nu1\r\n\
         --{boundary}\r\nContent-Disposition: form-data; name=\"work_id\"\r\n\r\nw1\r\n\
         --{boundary}\r\nContent-Disposition: form-data; name=\"season\"\r\n\r\n1\r\n\
         --{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"big.ass\"\r\n\r\n\
         [Script Info]\n"
    );
    let (status, peak) = measure(request_of(stream(
        head,
        2048,
        format!("\r\n--{boundary}--\r\n"),
        &pulled,
    )))
    .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    assert!(
        peak < limit,
        "reading a file of 128 MiB peaked at {peak} bytes above where it started"
    );

    // 512 MiB of one part header that never ends.
    let pulled = Arc::new(AtomicU64::new(0));
    let head = format!("--{boundary}\r\nContent-Disposition: form-data; name=\"");
    let (status, peak) = measure(request_of(stream(head, 8192, String::new(), &pulled))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        peak < limit,
        "a header that never ends peaked at {peak} bytes above where it started"
    );
    assert!(pulled.load(Ordering::SeqCst) < 2 * 1024 * 1024);

    // 512 MiB before the first part.
    let pulled = Arc::new(AtomicU64::new(0));
    let (status, peak) = measure(request_of(stream(
        "preamble ".to_owned(),
        8192,
        format!("--{boundary}--\r\n"),
        &pulled,
    )))
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        peak < limit,
        "a preamble peaked at {peak} bytes above where it started"
    );
    assert!(pulled.load(Ordering::SeqCst) < 2 * 1024 * 1024);
}
