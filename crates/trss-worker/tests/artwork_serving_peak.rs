//! The memory a crowd of cover requests holds while their clients are slow to
//! read, counted as the bytes allocated and not yet freed (the peak of them),
//! which does not depend on how the allocator gives memory back: glibc keeps
//! freed 10 MiB buffers in per-thread arenas, so the resident set of the same
//! run reads far higher on a development machine than in the musl image.
//! It is the only test of this file so nothing else allocates while it counts.

use std::{
    alloc::{GlobalAlloc, Layout, System},
    sync::atomic::{AtomicUsize, Ordering},
    time::Duration,
};

use axum::{
    body::Body,
    http::{Method, Request, StatusCode},
};
use futures::StreamExt;
use tower::ServiceExt;
use trss_legacy::{
    artwork::{
        files::{image_ref, ARTWORK_DIR},
        AnilistConfig, AppData, Artwork, MAX_IMAGE_BYTES, SERVING_BUDGET,
    },
    discovery::{Scan, ScannedWork, WorkRead},
    store::{
        artwork::{Format, Source},
        library::LibraryStore,
        Db,
    },
};
use trss_web::{self as web, AppState};

static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

/// The system allocator, counting the live bytes and their peak.
struct Counting;

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let live = LIVE.fetch_add(layout.size(), Ordering::Relaxed) + layout.size();
        PEAK.fetch_max(live, Ordering::Relaxed);
        System.alloc(layout)
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
        System.dealloc(ptr, layout)
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let live = LIVE.fetch_add(new_size, Ordering::Relaxed) + new_size;
        PEAK.fetch_max(live, Ordering::Relaxed);
        LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
        System.realloc(ptr, layout, new_size)
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// A `kB` field of `/proc/self/status`, in bytes (reported, not asserted).
fn status_bytes(field: &str) -> u64 {
    let status = std::fs::read_to_string("/proc/self/status").unwrap();
    let line = status.lines().find(|l| l.starts_with(field)).unwrap();
    let kib: u64 = line.split_whitespace().nth(1).unwrap().parse().unwrap();
    kib * 1024
}

#[tokio::test]
async fn covers_requested_by_slow_clients_are_held_within_the_serving_budget() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("index.html"), "<html></html>").unwrap();
    let db = Db::open(dir.path().join("trss.db")).await.unwrap();
    let artwork = Artwork::new(
        db.clone(),
        Some(AppData::new(dir.path())),
        AnilistConfig::default(),
    );
    let library = LibraryStore::new(db.clone());
    let scan = Scan {
        works: vec![WorkRead::Read(ScannedWork {
            dir_name: "A".into(),
            seasons: Default::default(),
            files: Vec::new(),
            unrecognized: Vec::new(),
        })],
    };
    let (folder, _) = library.add_folder("/c".into(), scan, 1, &[]).await.unwrap();
    let work = library.works(&folder.id).await.unwrap()[0].id.clone();

    // The work's cover is a file of the largest size (serving checks its size,
    // hash and first bytes, and does not decode it).
    let mut bytes = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    bytes.resize(MAX_IMAGE_BYTES, 0);
    std::fs::create_dir_all(dir.path().join(ARTWORK_DIR)).unwrap();
    let relative = format!("{ARTWORK_DIR}/cover.png");
    std::fs::write(dir.path().join(&relative), &bytes).unwrap();
    let image = image_ref(Source::Upload, relative, &bytes, Format::Png);
    let id = image.id.clone();
    artwork
        .store
        .reserve_file(&image.relative_path, "artwork/.staging/cover.tmp", 1)
        .await
        .unwrap();
    artwork
        .store
        .select_manual(&work, 1, None, image)
        .await
        .unwrap();
    drop(bytes);

    let router = web::router(dir.path(), AppState::new(db).with_artwork(artwork));
    let uri = format!("/api/library/works/{work}/artwork/image?v={id}");

    // Eight requests at once, each client taking a moment before it reads.
    // Counting only the responses held at once, the peak is the budget; eight
    // whole copies would be 80 MiB.
    std::fs::write("/proc/self/clear_refs", "5").unwrap();
    let rss_before = status_bytes("VmRSS:");
    let live_before = LIVE.load(Ordering::Relaxed);
    PEAK.store(live_before, Ordering::Relaxed);
    let clients: Vec<_> = (0..8)
        .map(|_| {
            let router = router.clone();
            let uri = uri.clone();
            tokio::spawn(async move {
                let request = Request::builder()
                    .method(Method::GET)
                    .uri(uri)
                    .body(Body::empty())
                    .unwrap();
                let response = router.oneshot(request).await.unwrap();
                assert_eq!(response.status(), StatusCode::OK);
                tokio::time::sleep(Duration::from_millis(300)).await;
                let mut body = response.into_body().into_data_stream();
                let mut sent = 0;
                while let Some(chunk) = body.next().await {
                    sent += chunk.unwrap().len();
                }
                sent
            })
        })
        .collect();
    for client in clients {
        assert_eq!(client.await.unwrap(), MAX_IMAGE_BYTES);
    }
    let peak = PEAK.load(Ordering::Relaxed).saturating_sub(live_before);
    let rss = status_bytes("VmHWM:").saturating_sub(rss_before);
    eprintln!("held at most {peak} bytes; the resident set peaked {rss} bytes above its start");

    // Three files of 10 MiB fit the budget of 32 MiB; the rest is the
    // requests themselves.
    let allowed = SERVING_BUDGET + 8 * 1024 * 1024;
    assert!(
        peak <= allowed,
        "eight responses of {MAX_IMAGE_BYTES} bytes held {peak} bytes at once (budget {SERVING_BUDGET})"
    );
}
