//! The CRC32 of a received video is read as a stream: checking a file larger
//! than the worker container's 256M memory limit (`docs/specs/collection.md`,
//! 영상 수정본의 대체) holds one buffer, not the file. This binary counts what
//! it allocates, so it has a test of its own.

use std::{
    alloc::{GlobalAlloc, Layout, System},
    sync::atomic::{AtomicUsize, Ordering},
};

use trss_collect::revision::{file_crc32, CRC_BUFFER};

struct Counting;

static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            let live = LIVE.fetch_add(layout.size(), Ordering::SeqCst) + layout.size();
            PEAK.fetch_max(live, Ordering::SeqCst);
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) };
        LIVE.fetch_sub(layout.size(), Ordering::SeqCst);
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

#[test]
fn a_file_larger_than_the_memory_limit_is_checked_with_one_buffer() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("video.mkv");
    // 1 GiB of zeros, sparse, so the test writes almost nothing.
    std::fs::File::create(&path)
        .unwrap()
        .set_len(1 << 30)
        .unwrap();

    let before = LIVE.load(Ordering::SeqCst);
    PEAK.store(before, Ordering::SeqCst);
    let crc = file_crc32(&path).unwrap();
    let peak = PEAK.load(Ordering::SeqCst) - before;

    // Python's `zlib.crc32` over the same 1 GiB of zeros.
    assert_eq!(crc, 0x5B64C2B0);
    assert!(
        peak <= CRC_BUFFER + (64 << 10),
        "{peak} bytes held while reading"
    );
}
