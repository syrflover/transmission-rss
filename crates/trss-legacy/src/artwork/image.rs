//! Judging image bytes: the format from the bytes themselves (never a file
//! name, an extension or a declared MIME type), then the size the header
//! states, under the limits of [`super`], so a non-image, a file whose header
//! cannot be read and one too large to show is refused before it is stored
//! (`docs/specs/library.md`, 이미지 파일의 수명).
//!
//! Nothing is decoded (user decision, 2026-10-01). The app stores and serves
//! the bytes as they came, so decoding would only have found files damaged
//! after the header, at the cost of tens of MiB of memory for one image (the
//! output, the decoders' copies, a PNG's inflated colour profile). A file
//! damaged after its header is accepted and shows as an empty place on the
//! screen; the user uploads it again.
//!
//! The header is read by hand, from the first bytes only: the PNG's IHDR, the
//! JPEG's first frame header (SOFn, found by stepping over the segments before
//! it), the WebP's first chunk (`VP8 `, `VP8L` or `VP8X`). No chunk or segment
//! is read or inflated, so what this costs in memory is a few locals whatever
//! the file or the pixels it claims.

use super::{MAX_IMAGE_BYTES, MAX_IMAGE_PIXELS, MAX_IMAGE_SIDE};
use crate::store::artwork::Format;

/// Why bytes are not an image the app keeps. Each has a sentence for the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum Rejected {
    #[error("empty")]
    Empty,
    #[error("larger than the byte limit")]
    TooLarge,
    /// Not JPEG, PNG or WebP by its first bytes (text, another image format, ...).
    #[error("not a JPEG, PNG or WebP image")]
    NotImage,
    #[error("larger than the pixel limit")]
    TooManyPixels,
    /// Starts like an image but its header cannot be read (cut short, or not
    /// the header its format has).
    #[error("the header cannot be read")]
    Damaged,
}

impl Rejected {
    pub fn message(self) -> String {
        match self {
            Rejected::Empty => "빈 파일이에요.".to_owned(),
            Rejected::TooLarge => format!(
                "{}MB보다 큰 이미지는 받지 않아요.",
                MAX_IMAGE_BYTES / (1024 * 1024)
            ),
            Rejected::NotImage => {
                "JPEG·PNG·WebP 이미지가 아니에요. 파일 이름과 상관없이 내용으로 판단해요."
                    .to_owned()
            }
            Rejected::TooManyPixels => format!(
                "이미지가 너무 커요. 가로·세로 {MAX_IMAGE_SIDE}픽셀, 전체 {}만 픽셀까지 받아요.",
                MAX_IMAGE_PIXELS / 10_000
            ),
            Rejected::Damaged => {
                "이미지의 머리 부분을 읽지 못했어요. 손상된 파일일 수 있어요.".to_owned()
            }
        }
    }
}

const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];

/// The format the first bytes say, among the accepted ones.
pub fn sniff(bytes: &[u8]) -> Option<Format> {
    if bytes.starts_with(&PNG_SIGNATURE) {
        Some(Format::Png)
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some(Format::Jpeg)
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some(Format::Webp)
    } else {
        None
    }
}

/// Checks that `bytes` are a JPEG, PNG or WebP whose header can be read and
/// states a size within the limits, and says which format. Reads the header
/// only (see the module docs).
pub fn verify(bytes: &[u8]) -> Result<Format, Rejected> {
    if bytes.is_empty() {
        return Err(Rejected::Empty);
    }
    if bytes.len() > MAX_IMAGE_BYTES {
        return Err(Rejected::TooLarge);
    }
    let format = sniff(bytes).ok_or(Rejected::NotImage)?;
    let (width, height) = match format {
        Format::Png => png_size(bytes),
        Format::Jpeg => jpeg_size(bytes),
        Format::Webp => webp_size(bytes),
    }
    .ok_or(Rejected::Damaged)?;
    if width == 0 || height == 0 {
        return Err(Rejected::Damaged);
    }
    if width > MAX_IMAGE_SIDE
        || height > MAX_IMAGE_SIDE
        || u64::from(width) * u64::from(height) > MAX_IMAGE_PIXELS
    {
        return Err(Rejected::TooManyPixels);
    }
    Ok(format)
}

/// The size in the PNG's IHDR, which must be the first chunk: signature (8),
/// then length (4, 13), `IHDR` and its 13 bytes of fields.
fn png_size(bytes: &[u8]) -> Option<(u32, u32)> {
    let ihdr = bytes.get(8..33)?;
    if ihdr[..8] != *b"\0\0\0\x0dIHDR" {
        return None;
    }
    let width = u32::from_be_bytes(ihdr[8..12].try_into().ok()?);
    let height = u32::from_be_bytes(ihdr[12..16].try_into().ok()?);
    let (depth, color) = (ihdr[16], ihdr[17]);
    let depths: &[u8] = match color {
        0 => &[1, 2, 4, 8, 16],
        3 => &[1, 2, 4, 8],
        2 | 4 | 6 => &[8, 16],
        _ => return None,
    };
    // Compression and filter methods have one value each; interlace two.
    (depths.contains(&depth) && ihdr[18] == 0 && ihdr[19] == 0 && ihdr[20] <= 1)
        .then_some((width, height))
}

/// The size in the first frame header (SOFn) of a JPEG, found by stepping over
/// the segments before it by their lengths. Gives up at the first scan or the
/// end of the image (no frame came), and at anything that is not a marker.
fn jpeg_size(bytes: &[u8]) -> Option<(u32, u32)> {
    let mut at = 2; // After SOI.
    loop {
        if *bytes.get(at)? != 0xFF {
            return None;
        }
        // Fill bytes before a marker.
        while *bytes.get(at + 1)? == 0xFF {
            at += 1;
        }
        let marker = *bytes.get(at + 1)?;
        at += 2;
        match marker {
            // Not a marker (a stuffed byte), a second SOI, the end, the scan.
            0x00 | 0xD8 | 0xD9 | 0xDA => return None,
            // Markers without a segment.
            0x01 | 0xD0..=0xD7 => continue,
            _ => {}
        }
        let length = usize::from(u16::from_be_bytes([*bytes.get(at)?, *bytes.get(at + 1)?]));
        if length < 2 {
            return None;
        }
        let segment = bytes.get(at + 2..at + length)?;
        // SOF0..SOF15 except DHT (C4), JPG (C8) and DAC (CC).
        if matches!(marker, 0xC0..=0xCF) && !matches!(marker, 0xC4 | 0xC8 | 0xCC) {
            // Precision (1), height (2), width (2), components (1), and each
            // component's three bytes.
            let components = usize::from(*segment.get(5)?);
            if components == 0 || segment.len() < 6 + 3 * components {
                return None;
            }
            let height = u32::from(u16::from_be_bytes([segment[1], segment[2]]));
            let width = u32::from(u16::from_be_bytes([segment[3], segment[4]]));
            return Some((width, height));
        }
        at += length;
    }
}

/// The size in the first chunk of a WebP: the frame header of a lossy
/// `VP8 `, the 14-bit fields of a lossless `VP8L`, or the canvas of an
/// extended `VP8X` (an animation too). After `RIFF`, the file size and `WEBP`
/// (12 bytes) comes the chunk's tag (4) and size (4).
fn webp_size(bytes: &[u8]) -> Option<(u32, u32)> {
    let size = u32::from_le_bytes(bytes.get(16..20)?.try_into().ok()?) as usize;
    match bytes.get(12..16)? {
        b"VP8 " => {
            // Frame tag (3, bit 0 clear for a key frame), start code, then
            // width and height with a 2-bit scale each above 14 bits.
            let head = bytes.get(20..30)?;
            if size < 10 || head[0] & 1 != 0 || head[3..6] != [0x9D, 0x01, 0x2A] {
                return None;
            }
            let width = u32::from(u16::from_le_bytes([head[6], head[7]]) & 0x3FFF);
            let height = u32::from(u16::from_le_bytes([head[8], head[9]]) & 0x3FFF);
            Some((width, height))
        }
        b"VP8L" => {
            // Signature byte 0x2F, then 14 bits of width − 1, 14 of height − 1,
            // an alpha flag and a 3-bit version (0).
            let head = bytes.get(20..25)?;
            if size < 5 || head[0] != 0x2F {
                return None;
            }
            let fields = u32::from_le_bytes(head[1..5].try_into().ok()?);
            if fields >> 29 != 0 {
                return None;
            }
            Some(((fields & 0x3FFF) + 1, ((fields >> 14) & 0x3FFF) + 1))
        }
        b"VP8X" => {
            // Flags (1), reserved (3), then canvas width − 1 and height − 1
            // in 24 bits each.
            let head = bytes.get(20..30)?;
            if size < 10 {
                return None;
            }
            let width = u32::from_le_bytes([head[4], head[5], head[6], 0]) + 1;
            let height = u32::from_le_bytes([head[7], head[8], head[9], 0]) + 1;
            Some((width, height))
        }
        _ => None,
    }
}

#[cfg(any(test, feature = "test-support"))]
pub mod samples {
    //! Images made on the spot for the tests.

    use std::io::Cursor;

    use image::{ImageFormat, RgbImage};

    pub fn encoded(width: u32, height: u32, format: ImageFormat) -> Vec<u8> {
        let image = RgbImage::from_fn(width, height, |x, y| {
            image::Rgb([(x % 256) as u8, (y % 256) as u8, 128])
        });
        let mut out = Cursor::new(Vec::new());
        image.write_to(&mut out, format).unwrap();
        out.into_inner()
    }

    pub fn jpeg() -> Vec<u8> {
        encoded(46, 65, ImageFormat::Jpeg)
    }

    pub fn png() -> Vec<u8> {
        encoded(46, 65, ImageFormat::Png)
    }

    /// A lossless WebP (`VP8L`), which is what the `image` crate writes.
    pub fn webp() -> Vec<u8> {
        encoded(46, 65, ImageFormat::WebP)
    }

    /// A 64 × 64 progressive JPEG (SOF2) without chroma subsampling (made with
    /// `cjpeg -progressive -sample 1x1`; the `image` crate writes only
    /// baseline JPEG).
    pub fn progressive_jpeg() -> Vec<u8> {
        include_bytes!("../../tests/fixtures/progressive_444.jpg").to_vec()
    }

    /// The start of a JPEG whose frame header (`sof`, e.g. `0xC2` for
    /// progressive) claims `width` × `height` with `components` components,
    /// then the start of a scan and no scan data.
    pub fn jpeg_header(sof: u8, width: u16, height: u16, components: u8) -> Vec<u8> {
        let mut out = vec![0xFF, 0xD8, 0xFF, sof];
        out.extend_from_slice(&(8 + 3 * u16::from(components)).to_be_bytes());
        out.push(8);
        out.extend_from_slice(&height.to_be_bytes());
        out.extend_from_slice(&width.to_be_bytes());
        out.push(components);
        for i in 0..components {
            out.extend_from_slice(&[i + 1, 0x11, 0]);
        }
        out.extend_from_slice(&[0xFF, 0xDA, 0, 3, 1, 0]);
        out
    }

    /// The start of a WebP whose first chunk is `kind` (`VP8 `, `VP8L` or
    /// `VP8X`) and claims `width` × `height`, with no image data.
    pub fn webp_header(kind: &[u8; 4], width: u32, height: u32) -> Vec<u8> {
        let mut data = Vec::new();
        match kind {
            b"VP8 " => {
                // Frame tag of a key frame, the start code, 14-bit sizes.
                data.extend_from_slice(&[0, 0, 0, 0x9D, 0x01, 0x2A]);
                data.extend_from_slice(&(width as u16).to_le_bytes());
                data.extend_from_slice(&(height as u16).to_le_bytes());
            }
            b"VP8L" => {
                data.push(0x2F);
                data.extend_from_slice(&((width - 1) | ((height - 1) << 14)).to_le_bytes());
            }
            _ => {
                // VP8X: flags and reserved bytes, then the canvas − 1.
                data.extend_from_slice(&[0, 0, 0, 0]);
                data.extend_from_slice(&(width - 1).to_le_bytes()[..3]);
                data.extend_from_slice(&(height - 1).to_le_bytes()[..3]);
            }
        }
        let mut out = b"RIFF".to_vec();
        out.extend_from_slice(&(4 + 8 + data.len() as u32).to_le_bytes());
        out.extend_from_slice(b"WEBP");
        out.extend_from_slice(kind);
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&data);
        out
    }

    /// The start of a GIF (a format the app does not take).
    pub fn gif() -> Vec<u8> {
        b"GIF89a\x01\x00\x01\x00\x00\x00\x00;".to_vec()
    }

    /// A small PNG whose header claims `width` × `height` (its CRC fixed up), so
    /// a limit on the header is tested without encoding a huge image.
    pub fn png_claiming(width: u32, height: u32) -> Vec<u8> {
        let mut png = png();
        // Signature (8), IHDR length (4), "IHDR" (4), width, height, ...
        png[16..20].copy_from_slice(&width.to_be_bytes());
        png[20..24].copy_from_slice(&height.to_be_bytes());
        let crc = crc32(&png[12..29]);
        png[29..33].copy_from_slice(&crc.to_be_bytes());
        png
    }

    fn crc32(bytes: &[u8]) -> u32 {
        let mut crc = 0xffff_ffffu32;
        for &b in bytes {
            crc ^= u32::from(b);
            for _ in 0..8 {
                crc = if crc & 1 != 0 {
                    (crc >> 1) ^ 0xedb8_8320
                } else {
                    crc >> 1
                };
            }
        }
        !crc
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jpeg_png_and_webp_are_known_by_their_bytes() {
        assert_eq!(verify(&samples::jpeg()), Ok(Format::Jpeg));
        assert_eq!(verify(&samples::png()), Ok(Format::Png));
        assert_eq!(verify(&samples::webp()), Ok(Format::Webp));
        assert_eq!(verify(&samples::progressive_jpeg()), Ok(Format::Jpeg));
    }

    #[test]
    fn every_frame_marker_of_a_jpeg_gives_its_size() {
        for sof in [
            0xC0, 0xC1, 0xC2, 0xC3, 0xC5, 0xC6, 0xC7, 0xC9, 0xCA, 0xCB, 0xCD, 0xCE, 0xCF,
        ] {
            assert_eq!(
                verify(&samples::jpeg_header(sof, 4000, 3000, 3)),
                Ok(Format::Jpeg),
                "{sof:#x}"
            );
            assert_eq!(
                verify(&samples::jpeg_header(sof, 4001, 3000, 3)),
                Err(Rejected::TooManyPixels),
                "{sof:#x}"
            );
        }
    }

    #[test]
    fn segments_before_the_frame_header_are_stepped_over() {
        // APP1 (EXIF-sized), a comment, a fill byte and a restart marker
        // before the frame header, which may hold anything.
        let jpeg = samples::jpeg();
        let mut with = vec![0xFF, 0xD8];
        with.extend_from_slice(&[0xFF, 0xE1]);
        with.extend_from_slice(&(60_000u16).to_be_bytes());
        with.extend(std::iter::repeat_n(0xFF, 60_000 - 2));
        with.extend_from_slice(&[0xFF, 0xFE, 0, 6, b'h', b'i', b'!', b'!']);
        with.extend_from_slice(&[0xFF, 0xFF, 0xFF, 0x01]);
        with.extend_from_slice(&jpeg[2..]);
        assert_eq!(verify(&with), Ok(Format::Jpeg));
        // The same file with the frame header gone reaches the scan first.
        let mut no_frame = vec![0xFF, 0xD8, 0xFF, 0xDA, 0, 3, 1, 0];
        no_frame.extend_from_slice(&[0; 100]);
        assert_eq!(verify(&no_frame), Err(Rejected::Damaged));
    }

    #[test]
    fn what_is_not_an_image_is_refused_by_its_first_bytes() {
        assert_eq!(verify(b""), Err(Rejected::Empty));
        assert_eq!(verify(b"hello, this is text\n"), Err(Rejected::NotImage));
        assert_eq!(verify(&samples::gif()), Err(Rejected::NotImage));
        // A RIFF file that is not WebP, and a PNG signature cut short.
        assert_eq!(verify(b"RIFF\x04\0\0\0WAVEfmt "), Err(Rejected::NotImage));
        assert_eq!(verify(&samples::png()[..7]), Err(Rejected::NotImage));
        // Bytes over the limit are refused before anything is read.
        let mut big = samples::png();
        big.resize(MAX_IMAGE_BYTES + 1, 0);
        assert_eq!(verify(&big), Err(Rejected::TooLarge));
        big.truncate(MAX_IMAGE_BYTES);
        assert_eq!(verify(&big), Ok(Format::Png));
    }

    #[test]
    fn a_header_that_cannot_be_read_is_damaged() {
        let png = samples::png();
        let jpeg = samples::jpeg();
        let webp = samples::webp();
        // Cut inside the header.
        assert_eq!(verify(&png[..20]), Err(Rejected::Damaged));
        assert_eq!(verify(&png[..8]), Err(Rejected::Damaged));
        assert_eq!(verify(&jpeg[..10]), Err(Rejected::Damaged));
        assert_eq!(verify(&jpeg[..4]), Err(Rejected::Damaged));
        assert_eq!(verify(&webp[..14]), Err(Rejected::Damaged));
        assert_eq!(verify(&webp[..24]), Err(Rejected::Damaged));
        assert_eq!(
            verify(&samples::jpeg_header(0xC0, 10, 10, 3)[..14]),
            Err(Rejected::Damaged)
        );

        // Not the header the format has: IHDR not first, a colour type or
        // depth that does not exist, a frame without components, a first
        // chunk that is not an image's.
        let mut not_first = png.clone();
        not_first[12..16].copy_from_slice(b"iCCP");
        assert_eq!(verify(&not_first), Err(Rejected::Damaged));
        let mut bad_color = png.clone();
        bad_color[25] = 5;
        assert_eq!(verify(&bad_color), Err(Rejected::Damaged));
        let mut bad_depth = png.clone();
        bad_depth[24] = 3;
        assert_eq!(verify(&bad_depth), Err(Rejected::Damaged));
        assert_eq!(
            verify(&samples::jpeg_header(0xC0, 10, 10, 0)),
            Err(Rejected::Damaged)
        );
        let mut other_chunk = samples::webp_header(b"VP8L", 10, 10);
        other_chunk[12..16].copy_from_slice(b"ANIM");
        assert_eq!(verify(&other_chunk), Err(Rejected::Damaged));
        let mut bad_signature = samples::webp_header(b"VP8L", 10, 10);
        bad_signature[20] = 0;
        assert_eq!(verify(&bad_signature), Err(Rejected::Damaged));
        let mut delta_frame = samples::webp_header(b"VP8 ", 10, 10);
        delta_frame[20] |= 1;
        assert_eq!(verify(&delta_frame), Err(Rejected::Damaged));

        // A size of nothing.
        assert_eq!(verify(&samples::png_claiming(0, 5)), Err(Rejected::Damaged));
        assert_eq!(verify(&samples::png_claiming(5, 0)), Err(Rejected::Damaged));
        assert_eq!(
            verify(&samples::jpeg_header(0xC0, 0, 5, 3)),
            Err(Rejected::Damaged)
        );
        assert_eq!(
            verify(&samples::jpeg_header(0xC0, 5, 0, 3)),
            Err(Rejected::Damaged)
        );
        assert_eq!(
            verify(&samples::webp_header(b"VP8 ", 0, 5)),
            Err(Rejected::Damaged)
        );
    }

    #[test]
    fn the_side_and_pixel_limits_hold_at_their_edges_in_every_format() {
        let side = MAX_IMAGE_SIDE;
        let png = |w, h| verify(&samples::png_claiming(w, h));
        assert_eq!(png(side, 1), Ok(Format::Png));
        assert_eq!(png(side + 1, 1), Err(Rejected::TooManyPixels));
        assert_eq!(png(1, side + 1), Err(Rejected::TooManyPixels));
        assert_eq!(png(4000, 3000), Ok(Format::Png));
        assert_eq!(png(4001, 3000), Err(Rejected::TooManyPixels));
        assert_eq!(png(u32::MAX, u32::MAX), Err(Rejected::TooManyPixels));

        let jpeg = |w, h| verify(&samples::jpeg_header(0xC0, w, h, 3));
        assert_eq!(jpeg(side as u16, 1), Ok(Format::Jpeg));
        assert_eq!(jpeg(side as u16 + 1, 1), Err(Rejected::TooManyPixels));
        assert_eq!(jpeg(u16::MAX, u16::MAX), Err(Rejected::TooManyPixels));

        for kind in [b"VP8L", b"VP8X"] {
            let webp = |w, h| verify(&samples::webp_header(kind, w, h));
            assert_eq!(webp(side, 1), Ok(Format::Webp), "{kind:?}");
            assert_eq!(webp(side + 1, 1), Err(Rejected::TooManyPixels), "{kind:?}");
            assert_eq!(webp(4000, 3000), Ok(Format::Webp), "{kind:?}");
            assert_eq!(webp(4001, 3000), Err(Rejected::TooManyPixels), "{kind:?}");
        }
        // The widest a canvas or a lossless image can say.
        assert_eq!(
            verify(&samples::webp_header(b"VP8X", 1 << 24, 1 << 24)),
            Err(Rejected::TooManyPixels)
        );
        assert_eq!(
            verify(&samples::webp_header(b"VP8L", 1 << 14, 1 << 14)),
            Err(Rejected::TooManyPixels)
        );
        let lossy = |w, h| verify(&samples::webp_header(b"VP8 ", w, h));
        assert_eq!(lossy(side, 1), Ok(Format::Webp));
        assert_eq!(lossy(side + 1, 1), Err(Rejected::TooManyPixels));
    }

    #[test]
    fn a_file_broken_after_its_header_is_accepted() {
        // Cut in the middle of the data, and the data overwritten.
        for bytes in [samples::png(), samples::jpeg(), samples::webp()] {
            let format = sniff(&bytes).unwrap();
            let cut = bytes.len() / 2;
            assert!(cut > 40);
            assert_eq!(verify(&bytes[..cut]), Ok(format));
            let mut garbage = bytes.clone();
            garbage[40..].fill(0x5A);
            assert_eq!(verify(&garbage), Ok(format));
        }
        // A PNG with nothing but its header, and with a colour profile chunk
        // whose bytes are not even zlib: no chunk is read.
        let mut png = samples::png()[..33].to_vec();
        assert_eq!(verify(&png), Ok(Format::Png));
        png.extend_from_slice(&(2_000_000u32).to_be_bytes());
        png.extend_from_slice(b"iCCP");
        png.extend(std::iter::repeat_n(0xA5, 2_000_000));
        assert_eq!(verify(&png), Ok(Format::Png));
        // A JPEG with its frame header and a scan with no data, a WebP with
        // its first chunk header and nothing after.
        assert_eq!(
            verify(&samples::jpeg_header(0xC2, 64, 64, 3)),
            Ok(Format::Jpeg)
        );
        assert_eq!(
            verify(&samples::webp_header(b"VP8L", 64, 64)),
            Ok(Format::Webp)
        );
    }

    #[test]
    fn sniffing_agrees_with_what_verify_accepts() {
        for bytes in [samples::jpeg(), samples::png(), samples::webp()] {
            assert_eq!(sniff(&bytes), verify(&bytes).ok());
        }
        assert_eq!(sniff(&samples::gif()), None);
        assert_eq!(sniff(b""), None);
    }
}
