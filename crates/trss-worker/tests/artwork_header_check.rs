//! What judging an image allocates: nothing that grows with the pixels or the
//! file. The only test of this file so nothing else allocates while it counts.

use std::{
    alloc::{GlobalAlloc, Layout, System},
    sync::atomic::{AtomicUsize, Ordering},
};

use trss_anilist::MAX_IMAGE_BYTES;
use trss_legacy::{artwork::image::verify, store::artwork::Format};

/// Every byte ever allocated, freed or not.
static ALLOCATED: AtomicUsize = AtomicUsize::new(0);

struct Counting;

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCATED.fetch_add(layout.size(), Ordering::Relaxed);
        System.alloc(layout)
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        System.dealloc(ptr, layout)
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOCATED.fetch_add(new_size, Ordering::Relaxed);
        System.realloc(ptr, layout, new_size)
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

fn chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut out = (data.len() as u32).to_be_bytes().to_vec();
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    out.extend_from_slice(&[0; 4]);
    out
}

#[test]
fn judging_an_image_allocates_nothing_that_grows_with_its_pixels_or_bytes() {
    // 4000 × 3000 at 16 bits a channel with alpha: 96 MB once decoded. Its
    // colour profile is 9 MiB of bytes that are not even zlib, after which the
    // file ends.
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&4000u32.to_be_bytes());
    ihdr.extend_from_slice(&3000u32.to_be_bytes());
    ihdr.extend_from_slice(&[16, 6, 0, 0, 0]);
    let mut png = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    png.extend(chunk(b"IHDR", &ihdr));
    png.extend(chunk(b"iCCP", &vec![0xA5; 9 * 1024 * 1024]));

    // A JPEG behind 150 segments of 64 KiB, a progressive frame of 12 MP.
    let mut jpeg = vec![0xFF, 0xD8];
    for _ in 0..150 {
        jpeg.extend_from_slice(&[0xFF, 0xE1, 0xFF, 0xFF]);
        jpeg.extend(std::iter::repeat_n(0, 0xFFFD));
    }
    jpeg.extend_from_slice(&[0xFF, 0xC2, 0, 17, 8]);
    jpeg.extend_from_slice(&3000u16.to_be_bytes());
    jpeg.extend_from_slice(&4000u16.to_be_bytes());
    jpeg.extend_from_slice(&[3, 1, 0x11, 0, 2, 0x11, 0, 3, 0x11, 0]);
    assert!(jpeg.len() < MAX_IMAGE_BYTES);

    // A lossless WebP of 12 MP followed by 9 MiB that is not image data.
    let mut webp = b"RIFF".to_vec();
    webp.extend_from_slice(&(12 + 5u32).to_le_bytes());
    webp.extend_from_slice(b"WEBPVP8L");
    webp.extend_from_slice(&5u32.to_le_bytes());
    webp.push(0x2F);
    webp.extend_from_slice(&(3999u32 | (2999 << 14)).to_le_bytes());
    webp.extend(std::iter::repeat_n(0x5A, 9 * 1024 * 1024));

    for (bytes, format) in [
        (png, Format::Png),
        (jpeg, Format::Jpeg),
        (webp, Format::Webp),
    ] {
        let before = ALLOCATED.load(Ordering::Relaxed);
        let result = verify(&bytes);
        let allocated = ALLOCATED.load(Ordering::Relaxed) - before;
        assert_eq!(result, Ok(format));
        assert!(
            allocated < 1024,
            "judging a {format:?} of {} bytes allocated {allocated} bytes",
            bytes.len()
        );
    }
}
