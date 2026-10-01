//! The resident memory a verify peaks at, for images whose headers hide part
//! of the cost. Linux only (it reads `/proc/self`), and the only test of this
//! file so nothing else allocates while it measures.
#![cfg(target_os = "linux")]

use image::{ExtendedColorType, ImageEncoder};
use transmission_rss::{
    artwork::{image::verify, DECODE_MAX_ALLOC},
    store::artwork::Format,
};

/// A `kB` field of `/proc/self/status`, in bytes.
fn status_bytes(field: &str) -> u64 {
    let status = std::fs::read_to_string("/proc/self/status").unwrap();
    let line = status.lines().find(|l| l.starts_with(field)).unwrap();
    let kib: u64 = line.split_whitespace().nth(1).unwrap().parse().unwrap();
    kib * 1024
}

/// Runs `f` and returns how far the resident set peaked above where it
/// started.
fn peak_of<T>(f: impl FnOnce() -> T) -> (T, u64) {
    // Resets the peak (`VmHWM`) to the current resident set.
    std::fs::write("/proc/self/clear_refs", "5").unwrap();
    let before = status_bytes("VmRSS:");
    let out = f();
    (out, status_bytes("VmHWM:").saturating_sub(before))
}

#[test]
fn a_png_profile_does_not_add_to_the_decoded_image_past_the_budget() {
    // 2000 × 2000 8-bit RGBA decodes to 16 MB. Its ICC profile inflates to
    // 62 MiB from a few kilobytes: the header's allowance and the output
    // together would be 78 MB against the 64 MiB budget.
    let (width, height) = (2000u32, 2000u32);
    let png = {
        let pixels = vec![0x80u8; width as usize * height as usize * 4];
        let mut out = Vec::new();
        let mut encoder = image::codecs::png::PngEncoder::new(&mut out);
        encoder.set_icc_profile(vec![0; 62 * 1024 * 1024]).unwrap();
        encoder
            .write_image(&pixels, width, height, ExtendedColorType::Rgba8)
            .unwrap();
        out
    };
    assert!(png.len() < 1024 * 1024, "{}", png.len());

    let (result, peak) = peak_of(|| verify(&png));
    assert_eq!(result, Ok(Format::Png));
    assert!(
        peak <= DECODE_MAX_ALLOC,
        "the decode peaked at {peak} bytes above where it started"
    );
}
