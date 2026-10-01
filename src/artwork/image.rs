//! Judging image bytes: the format from the bytes themselves (never a file
//! name, an extension or a declared MIME type), then a full decode under the
//! limits of [`super`], so a damaged file, a non-image or one too large to
//! decode safely is refused before it is stored.
//!
//! # Decode memory
//!
//! The pixel limit alone does not bound what a decode allocates: a 16-bit
//! RGBA PNG takes 8 bytes a pixel, and a progressive JPEG (or a sequential one
//! written a scan a component) keeps every DCT coefficient (2 bytes a sample)
//! besides its output. Before decoding,
//! [`decode_cost`] adds up from the headers what the decoders of the `image`
//! crate allocate: the output buffer ([`ImageDecoder::total_bytes`]), the JPEG
//! decoder's copy of the input and the coefficients of such a JPEG, and the
//! WebP decoder's intermediate frame. An image whose cost is over
//! [`DECODE_MAX_ALLOC`] is refused without being decoded
//! ([`Rejected::TooCostly`]).
//!
//! A PNG's embedded colour profile (iCCP) is inflated by the decoder while it
//! reads the header, before the output exists, and kept beside it. The header
//! is therefore read with a limit of what [`DECODE_MAX_ALLOC`] leaves after
//! the output the IHDR (and tRNS) announce ([`png_header_budget`]): a profile
//! that inflates past it is dropped by the decoder, not allocated. The app
//! keeps the file as it came and reads no profile.

use std::io::Cursor;

use image::{DynamicImage, ImageDecoder, ImageFormat, ImageReader, Limits};

use super::{DECODE_MAX_ALLOC, MAX_IMAGE_BYTES, MAX_IMAGE_PIXELS, MAX_IMAGE_SIDE};
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
    /// Within the pixel limit, but decoding it would take more memory than
    /// [`DECODE_MAX_ALLOC`] (16 bits a channel, a large progressive JPEG, ...).
    #[error("too costly to decode")]
    TooCostly,
    /// Starts like an image but does not decode (damaged, cut short, or too
    /// costly to decode).
    #[error("does not decode")]
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
            Rejected::TooCostly => {
                "이미지를 확인하는 데 메모리가 너무 많이 들어요. 크기를 줄이거나 8비트 일반 JPEG·PNG로 저장해 올려 주세요."
                    .to_owned()
            }
            Rejected::Damaged => {
                "이미지를 끝까지 읽지 못했어요. 손상된 파일일 수 있어요.".to_owned()
            }
        }
    }
}

/// The format the first bytes say, among the accepted ones.
pub fn sniff(bytes: &[u8]) -> Option<Format> {
    match image::guess_format(bytes).ok()? {
        ImageFormat::Jpeg => Some(Format::Jpeg),
        ImageFormat::Png => Some(Format::Png),
        ImageFormat::WebP => Some(Format::Webp),
        _ => None,
    }
}

/// Checks that `bytes` are a whole JPEG, PNG or WebP image within the limits,
/// and says which. Decodes the whole image, so it is for a blocking thread.
pub fn verify(bytes: &[u8]) -> Result<Format, Rejected> {
    verify_within(bytes, DECODE_MAX_ALLOC)
}

/// [`verify`] with `budget` bytes for the decode in place of
/// [`DECODE_MAX_ALLOC`] (tests use small images and a small budget).
pub(crate) fn verify_within(bytes: &[u8], budget: u64) -> Result<Format, Rejected> {
    if bytes.is_empty() {
        return Err(Rejected::Empty);
    }
    if bytes.len() > MAX_IMAGE_BYTES {
        return Err(Rejected::TooLarge);
    }
    let format = sniff(bytes).ok_or(Rejected::NotImage)?;
    let image_format = match format {
        Format::Jpeg => ImageFormat::Jpeg,
        Format::Png => ImageFormat::Png,
        Format::Webp => ImageFormat::WebP,
    };

    let mut reader = ImageReader::with_format(Cursor::new(bytes), image_format);
    // The PNG decoder inflates compressed ancillary chunks (an ICC profile,
    // compressed text) while it reads the header, bounded only by this limit,
    // and keeps them while it decodes: the header gets what the output leaves.
    let header_alloc = match format {
        Format::Png => png_header_budget(bytes, budget)?,
        _ => budget,
    };
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_IMAGE_SIDE);
    limits.max_image_height = Some(MAX_IMAGE_SIDE);
    limits.max_alloc = Some(header_alloc);
    reader.limits(limits);

    let decoder = reader.into_decoder().map_err(|e| match e {
        image::ImageError::Limits(l) => match l.kind() {
            image::error::LimitErrorKind::InsufficientMemory => Rejected::TooCostly,
            _ => Rejected::TooManyPixels,
        },
        _ => Rejected::Damaged,
    })?;
    let (width, height) = decoder.dimensions();
    if width == 0 || height == 0 {
        return Err(Rejected::Damaged);
    }
    if u64::from(width) * u64::from(height) > MAX_IMAGE_PIXELS {
        return Err(Rejected::TooManyPixels);
    }
    // `from_decoder` allocates the output without asking the limits, and the
    // JPEG decoder keeps its own buffers: the whole cost is checked first.
    if decode_cost(format, bytes, decoder.total_bytes(), width, height)? > budget {
        return Err(Rejected::TooCostly);
    }
    match DynamicImage::from_decoder(decoder) {
        Ok(_) => Ok(format),
        Err(image::ImageError::Limits(_)) => Err(Rejected::TooCostly),
        Err(_) => Err(Rejected::Damaged),
    }
}

/// The bytes a decode of `bytes` allocates at its peak, from the headers:
/// `output` (the decoded image), plus for JPEG the decoder's copy of the input
/// and, when it keeps them ([`JpegFrame::keeps_coefficients`]), the
/// coefficients of every component; for WebP the
/// frame the decoder fills before the output (and the canvas of an animation).
pub(crate) fn decode_cost(
    format: Format,
    bytes: &[u8],
    output: u64,
    width: u32,
    height: u32,
) -> Result<u64, Rejected> {
    let pixels = u64::from(width) * u64::from(height);
    let extra = match format {
        Format::Png => 0,
        Format::Jpeg => {
            let frame = jpeg_frame(bytes).ok_or(Rejected::Damaged)?;
            let coefficients = if frame.keeps_coefficients() {
                frame.coefficient_bytes()
            } else {
                0
            };
            bytes.len() as u64 + coefficients
        }
        Format::Webp => {
            let animated = bytes.len() > 20 && &bytes[12..16] == b"VP8X" && bytes[20] & 0x02 != 0;
            pixels * 4 + if animated { pixels * 8 } else { 0 }
        }
    };
    Ok(output.saturating_add(extra))
}

/// What the PNG decoder may allocate for the header's chunks, from the file's
/// IHDR (and tRNS): `budget` less the decoded image, which it allocates beside
/// them. A header that already claims an image too large or too costly is
/// refused here, before any chunk is read.
fn png_header_budget(bytes: &[u8], budget: u64) -> Result<u64, Rejected> {
    // Signature (8), then IHDR: length (4), "IHDR", 13 bytes of fields.
    let ihdr = bytes.get(8..33).ok_or(Rejected::Damaged)?;
    if &ihdr[4..8] != b"IHDR" {
        return Err(Rejected::Damaged);
    }
    let width = u32::from_be_bytes([ihdr[8], ihdr[9], ihdr[10], ihdr[11]]);
    let height = u32::from_be_bytes([ihdr[12], ihdr[13], ihdr[14], ihdr[15]]);
    let (depth, color) = (ihdr[16], ihdr[17]);
    if width == 0 || height == 0 {
        return Err(Rejected::Damaged);
    }
    if width > MAX_IMAGE_SIDE
        || height > MAX_IMAGE_SIDE
        || u64::from(width) * u64::from(height) > MAX_IMAGE_PIXELS
    {
        return Err(Rejected::TooManyPixels);
    }
    // The decoder expands palettes and low bit depths to 8 bits, and a tRNS
    // chunk adds an alpha channel (`image` reads them with `EXPAND`).
    let transparent = png_has_transparency(&bytes[33..]);
    let channels: u64 = match (color, transparent) {
        (0, false) => 1,
        (0, true) | (4, _) => 2,
        (2, false) | (3, false) => 3,
        (2, true) | (3, true) | (6, _) => 4,
        _ => return Err(Rejected::Damaged),
    };
    let sample = if depth == 16 { 2 } else { 1 };
    let output = u64::from(width) * u64::from(height) * channels * sample;
    budget.checked_sub(output).ok_or(Rejected::TooCostly)
}

/// Whether a tRNS chunk comes among the chunks `chunks` (the PNG after its
/// IHDR) before the first IDAT. Stops at anything that is not a chunk.
fn png_has_transparency(mut chunks: &[u8]) -> bool {
    while let Some(head) = chunks.get(..8) {
        let length = u32::from_be_bytes([head[0], head[1], head[2], head[3]]) as usize;
        match &head[4..8] {
            b"tRNS" => return true,
            b"IDAT" | b"IEND" => return false,
            _ => {}
        }
        // The data and the CRC after the head.
        match chunks.get(8usize.saturating_add(length).saturating_add(4)..) {
            Some(rest) => chunks = rest,
            None => return false,
        }
    }
    false
}

/// A JPEG frame header (SOF): what the decoder's buffers depend on.
#[derive(Debug, Clone, PartialEq, Eq)]
struct JpegFrame {
    progressive: bool,
    width: u32,
    height: u32,
    /// Each component's horizontal and vertical sampling factors.
    sampling: Vec<(u32, u32)>,
    /// How many components the first scan (SOS) lists.
    first_scan_components: usize,
}

impl JpegFrame {
    /// Whether the decoder keeps every coefficient of the image before it
    /// writes any output: a progressive file always does, and so does a
    /// sequential one whose first scan lists fewer components than the frame
    /// (one scan a component, `cjpeg -scans`), because the other components
    /// arrive only in the scans after it.
    fn keeps_coefficients(&self) -> bool {
        self.progressive || self.first_scan_components != self.sampling.len()
    }

    /// The coefficients such a decode keeps for the whole image: 64
    /// two-byte coefficients for every block of every component, blocks
    /// padded to whole MCUs.
    fn coefficient_bytes(&self) -> u64 {
        let h_max = self.sampling.iter().map(|s| s.0).max().unwrap_or(1).max(1);
        let v_max = self.sampling.iter().map(|s| s.1).max().unwrap_or(1).max(1);
        let mcus_x = u64::from(self.width.div_ceil(8 * h_max));
        let mcus_y = u64::from(self.height.div_ceil(8 * v_max));
        self.sampling
            .iter()
            .map(|&(h, v)| mcus_x * u64::from(h) * mcus_y * u64::from(v) * 64 * 2)
            .sum()
    }
}

/// The frame header of a JPEG and the component count of its first scan, read
/// from the markers before the first scan's data.
fn jpeg_frame(bytes: &[u8]) -> Option<JpegFrame> {
    if bytes.get(..2)? != [0xFF, 0xD8] {
        return None;
    }
    let mut at = 2;
    let mut frame = None;
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
            // Markers without a segment.
            0x01 | 0xD0..=0xD7 => continue,
            // The end before the first scan.
            0xD9 => return None,
            _ => {}
        }
        let length = usize::from(u16::from_be_bytes([*bytes.get(at)?, *bytes.get(at + 1)?]));
        if length < 2 {
            return None;
        }
        let segment = bytes.get(at + 2..at + length)?;
        if marker == 0xDA {
            // The first scan: a scan before any frame header is no JPEG.
            let mut frame: JpegFrame = frame?;
            frame.first_scan_components = usize::from(*segment.first()?);
            return Some(frame);
        }
        if frame.is_none() && matches!(marker, 0xC0..=0xCF) && !matches!(marker, 0xC4 | 0xC8 | 0xCC)
        {
            let height = u32::from(u16::from_be_bytes([*segment.get(1)?, *segment.get(2)?]));
            let width = u32::from(u16::from_be_bytes([*segment.get(3)?, *segment.get(4)?]));
            let count = usize::from(*segment.get(5)?);
            let mut sampling = Vec::with_capacity(count);
            for i in 0..count {
                let factors = *segment.get(6 + i * 3 + 1)?;
                sampling.push((u32::from(factors >> 4), u32::from(factors & 0x0F)));
            }
            frame = Some(JpegFrame {
                progressive: matches!(marker, 0xC2 | 0xC6 | 0xCA | 0xCE),
                width,
                height,
                sampling,
                first_scan_components: 0,
            });
        }
        at += length;
    }
}

#[cfg(test)]
pub(crate) mod samples {
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

    pub fn webp() -> Vec<u8> {
        encoded(46, 65, ImageFormat::WebP)
    }

    /// A PNG of `width` × `height` with 16 bits a channel and alpha (8 bytes
    /// a pixel decoded).
    pub fn png16(width: u32, height: u32) -> Vec<u8> {
        let image =
            image::ImageBuffer::<image::Rgba<u16>, Vec<u16>>::from_fn(width, height, |x, y| {
                image::Rgba([x as u16 * 997, y as u16 * 991, 4000, 65535])
            });
        let mut out = Cursor::new(Vec::new());
        image.write_to(&mut out, ImageFormat::Png).unwrap();
        out.into_inner()
    }

    /// A `width` × `height` 8-bit RGBA PNG with an ICC profile chunk (iCCP)
    /// that inflates to `profile_len` bytes of zeros. It compresses to a few
    /// kilobytes whatever its length.
    pub fn png_with_profile(width: u32, height: u32, profile_len: usize) -> Vec<u8> {
        use image::{ExtendedColorType, ImageEncoder};

        let pixels = vec![0x80u8; width as usize * height as usize * 4];
        let mut out = Vec::new();
        let mut encoder = image::codecs::png::PngEncoder::new(&mut out);
        encoder.set_icc_profile(vec![0; profile_len]).unwrap();
        encoder
            .write_image(&pixels, width, height, ExtendedColorType::Rgba8)
            .unwrap();
        out
    }

    /// A 64 × 64 progressive JPEG without chroma subsampling (made with
    /// `cjpeg -progressive -sample 1x1`; the `image` crate writes only
    /// baseline JPEG).
    pub fn progressive_jpeg() -> Vec<u8> {
        include_bytes!("../../tests/fixtures/progressive_444.jpg").to_vec()
    }

    /// The start of a JPEG whose frame header (`sof`, e.g. `0xC2` for
    /// progressive) claims `width` × `height` with the components' sampling
    /// factors `sampling` (`0x11` is 1×1), and a first scan listing every
    /// component, with no scan data after it.
    pub fn jpeg_header(sof: u8, width: u16, height: u16, sampling: &[u8]) -> Vec<u8> {
        jpeg_header_scanning(sof, width, height, sampling, sampling.len())
    }

    /// [`jpeg_header`] whose first scan lists `scan_components` components
    /// (one in a file written a scan a component).
    pub fn jpeg_header_scanning(
        sof: u8,
        width: u16,
        height: u16,
        sampling: &[u8],
        scan_components: usize,
    ) -> Vec<u8> {
        let mut out = vec![0xFF, 0xD8, 0xFF, sof];
        let length = 8 + 3 * sampling.len() as u16;
        out.extend_from_slice(&length.to_be_bytes());
        out.push(8);
        out.extend_from_slice(&height.to_be_bytes());
        out.extend_from_slice(&width.to_be_bytes());
        out.push(sampling.len() as u8);
        for (i, factors) in sampling.iter().enumerate() {
            out.extend_from_slice(&[i as u8 + 1, *factors, 0]);
        }
        // SOS: length, Ns, (component, tables) for each, Ss, Se, Ah/Al.
        out.extend_from_slice(&[0xFF, 0xDA]);
        out.extend_from_slice(&(6 + 2 * scan_components as u16).to_be_bytes());
        out.push(scan_components as u8);
        for i in 0..scan_components {
            out.extend_from_slice(&[i as u8 + 1, 0]);
        }
        out.extend_from_slice(&[0, 63, 0]);
        out
    }

    /// A 64 × 64 sequential JPEG without chroma subsampling written a scan a
    /// component (`cjpeg -baseline -sample 1x1 -scans`): its first scan lists
    /// one of the three components, so the decoder keeps every coefficient.
    pub fn sequential_scans_jpeg() -> Vec<u8> {
        include_bytes!("../../tests/fixtures/sequential_scans_444.jpg").to_vec()
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
    fn the_format_comes_from_the_bytes() {
        assert_eq!(verify(&samples::jpeg()), Ok(Format::Jpeg));
        assert_eq!(verify(&samples::png()), Ok(Format::Png));
        assert_eq!(verify(&samples::webp()), Ok(Format::Webp));
    }

    #[test]
    fn text_damage_and_other_formats_are_refused() {
        assert_eq!(verify(b""), Err(Rejected::Empty));
        assert_eq!(verify(b"hello, this is text\n"), Err(Rejected::NotImage));
        assert_eq!(verify(&samples::gif()), Err(Rejected::NotImage));
        // A JPEG cut short starts like one and does not decode.
        let jpeg = samples::jpeg();
        assert_eq!(verify(&jpeg[..jpeg.len() / 2]), Err(Rejected::Damaged));
        let png = samples::png();
        assert_eq!(verify(&png[..png.len() - 20]), Err(Rejected::Damaged));
    }

    #[test]
    fn the_limits_are_kept_at_their_bounds() {
        // Over the side limit, and over the pixel limit within the sides.
        assert_eq!(
            verify(&samples::encoded(MAX_IMAGE_SIDE + 1, 1, ImageFormat::Png)),
            Err(Rejected::TooManyPixels)
        );
        let side = (MAX_IMAGE_PIXELS as f64).sqrt() as u32 + 1;
        assert_eq!(
            verify(&samples::png_claiming(side, side)),
            Err(Rejected::TooManyPixels)
        );
        // The longest side allowed decodes.
        assert_eq!(
            verify(&samples::encoded(MAX_IMAGE_SIDE, 1, ImageFormat::Png)),
            Ok(Format::Png)
        );
        // Bytes over the limit are refused before decoding.
        let mut big = samples::png();
        big.resize(MAX_IMAGE_BYTES + 1, 0);
        assert_eq!(verify(&big), Err(Rejected::TooLarge));
    }

    #[test]
    fn what_a_decode_would_allocate_is_kept_under_the_budget() {
        // 64 × 64: 16 KiB decoded as 8-bit RGBA, 32 KiB as 16-bit RGBA; 12 KiB
        // as an RGB JPEG, plus 24 KiB of coefficients when progressive 4:4:4.
        let budget = 20_000;
        let rgba8 = samples::encoded(64, 64, ImageFormat::Png);
        assert_eq!(verify_within(&rgba8, budget), Ok(Format::Png));
        assert_eq!(
            verify_within(&samples::png16(64, 64), budget),
            Err(Rejected::TooCostly)
        );
        let baseline = samples::encoded(64, 64, ImageFormat::Jpeg);
        assert_eq!(verify_within(&baseline, budget), Ok(Format::Jpeg));
        let progressive = samples::progressive_jpeg();
        assert_eq!(
            verify_within(&progressive, budget),
            Err(Rejected::TooCostly)
        );
        // A sequential JPEG written a scan a component keeps the same
        // coefficients as a progressive one.
        let scans = samples::sequential_scans_jpeg();
        assert_eq!(verify_within(&scans, budget), Err(Rejected::TooCostly));
        // Each decodes within a budget that covers it.
        assert_eq!(
            verify_within(&samples::png16(64, 64), 40_000),
            Ok(Format::Png)
        );
        assert_eq!(verify_within(&progressive, 40_000), Ok(Format::Jpeg));
        assert_eq!(verify_within(&scans, 40_000), Ok(Format::Jpeg));
    }

    #[test]
    fn a_png_profile_over_what_the_output_leaves_is_dropped_not_refused() {
        // 512 × 512 8-bit RGBA is 1 MiB decoded: with a 2 MiB budget, 1 MiB
        // is left for the header's chunks. The profile is metadata, so an
        // image whose profile inflates past that is still taken, and the
        // decoder never holds the profile (`tests/artwork_decode_peak.rs`
        // measures the memory).
        let budget = 2 * 1024 * 1024;
        let small = samples::png_with_profile(512, 512, 3 * 1024);
        assert_eq!(verify_within(&small, budget), Ok(Format::Png));
        let big = samples::png_with_profile(512, 512, 1536 * 1024);
        // Its bytes are tiny; only what it inflates to is large.
        assert!(big.len() < 64 * 1024, "{}", big.len());
        assert_eq!(verify_within(&big, budget), Ok(Format::Png));
        let mut limits = Limits::default();
        limits.max_alloc = Some(png_header_budget(&big, budget).unwrap());
        let mut kept =
            image::codecs::png::PngDecoder::with_limits(Cursor::new(&big[..]), limits).unwrap();
        assert_eq!(kept.icc_profile().unwrap(), None);
        let mut limits = Limits::default();
        limits.max_alloc = Some(png_header_budget(&small, budget).unwrap());
        let mut kept =
            image::codecs::png::PngDecoder::with_limits(Cursor::new(&small[..]), limits).unwrap();
        assert_eq!(kept.icc_profile().unwrap().map(|p| p.len()), Some(3 * 1024));
    }

    #[test]
    fn the_png_header_budget_is_what_the_decoded_image_leaves() {
        let png = |depth: u8, color: u8| {
            let mut out = samples::png();
            out[16..20].copy_from_slice(&100u32.to_be_bytes());
            out[20..24].copy_from_slice(&10u32.to_be_bytes());
            out[24] = depth;
            out[25] = color;
            out
        };
        // 1,000 pixels: 8-bit gray, RGB, RGBA, and 16-bit RGBA.
        assert_eq!(png_header_budget(&png(8, 0), 10_000), Ok(9_000));
        assert_eq!(png_header_budget(&png(8, 2), 10_000), Ok(7_000));
        assert_eq!(png_header_budget(&png(8, 6), 10_000), Ok(6_000));
        assert_eq!(png_header_budget(&png(16, 6), 10_000), Ok(2_000));
        // Low bit depths and palettes are expanded to 8 bits a sample.
        assert_eq!(png_header_budget(&png(4, 0), 10_000), Ok(9_000));
        assert_eq!(png_header_budget(&png(8, 3), 10_000), Ok(7_000));
        assert_eq!(
            png_header_budget(&png(16, 6), 7_000),
            Err(Rejected::TooCostly)
        );
        assert_eq!(
            png_header_budget(&png(8, 9), 10_000),
            Err(Rejected::Damaged)
        );
        assert_eq!(
            png_header_budget(&[0x89, b'P'], 10_000),
            Err(Rejected::Damaged)
        );
    }

    #[test]
    fn a_png_transparency_chunk_adds_an_alpha_channel_to_the_estimate() {
        let chunk = |kind: &[u8; 4], data: &[u8]| {
            let mut out = (data.len() as u32).to_be_bytes().to_vec();
            out.extend_from_slice(kind);
            out.extend_from_slice(data);
            out.extend_from_slice(&[0; 4]);
            out
        };
        let mut with = chunk(b"gAMA", &[0; 4]);
        with.extend(chunk(b"tRNS", &[0, 0]));
        with.extend(chunk(b"IDAT", &[0]));
        assert!(png_has_transparency(&with));
        // After the image data, or cut short, it does not count.
        let mut after = chunk(b"IDAT", &[0]);
        after.extend(chunk(b"tRNS", &[0, 0]));
        assert!(!png_has_transparency(&after));
        assert!(!png_has_transparency(&with[..14]));
        // A chunk claiming more than the file has ends the walk.
        let mut long = u32::MAX.to_be_bytes().to_vec();
        long.extend_from_slice(b"zTXt");
        assert!(!png_has_transparency(&long));
    }

    #[test]
    fn the_cost_of_the_largest_images_follows_their_kind() {
        let (w, h) = (4000u16, 3000u16);
        let pixels = 12_000_000u64;
        // A 12 MP progressive JPEG keeps 2 bytes a sample besides its RGB
        // output: 4:4:4 is over the budget, a baseline one is not.
        let sof2 = samples::jpeg_header(0xC2, w, h, &[0x11, 0x11, 0x11]);
        let cost = decode_cost(Format::Jpeg, &sof2, pixels * 3, 4000, 3000).unwrap();
        assert_eq!(cost, pixels * 3 + sof2.len() as u64 + pixels * 3 * 2);
        assert!(cost > DECODE_MAX_ALLOC);
        let sof0 = samples::jpeg_header(0xC0, w, h, &[0x22, 0x11, 0x11]);
        let cost = decode_cost(Format::Jpeg, &sof0, pixels * 3, 4000, 3000).unwrap();
        assert!(cost <= DECODE_MAX_ALLOC);
        // A sequential file whose first scan lists one component of three
        // (a scan a component) keeps the coefficients like a progressive one,
        // whatever its size on disk (cjpeg wrote 3.2 MB for 4000 × 3000).
        let scans = samples::jpeg_header_scanning(0xC0, w, h, &[0x11, 0x11, 0x11], 1);
        let cost = decode_cost(Format::Jpeg, &scans, pixels * 3, 4000, 3000).unwrap();
        assert_eq!(cost, pixels * 3 + scans.len() as u64 + pixels * 3 * 2);
        assert!(cost > DECODE_MAX_ALLOC);
        // Every component in the first scan (interleaved) keeps none, also
        // in extended sequential (SOF1).
        let sof1 = samples::jpeg_header(0xC1, w, h, &[0x11, 0x11, 0x11]);
        let cost = decode_cost(Format::Jpeg, &sof1, pixels * 3, 4000, 3000).unwrap();
        assert_eq!(cost, pixels * 3 + sof1.len() as u64);
        // 4:2:0 progressive: the chroma planes are a quarter each, and the
        // 16-pixel MCUs pad 3000 rows to 3008.
        let frame = jpeg_frame(&samples::jpeg_header(0xC2, w, h, &[0x22, 0x11, 0x11])).unwrap();
        assert!(frame.progressive);
        assert_eq!(
            frame.coefficient_bytes(),
            (4000 * 3008 + 2 * 2000 * 1504) * 2
        );
        // 16-bit RGBA at 12 MP is 96 MB of output.
        assert!(decode_cost(Format::Png, &[], pixels * 8, 4000, 3000).unwrap() > DECODE_MAX_ALLOC);
        // A JPEG without a frame header before its scan does not decode, nor
        // does one that ends before its first scan.
        assert_eq!(
            decode_cost(Format::Jpeg, &[0xFF, 0xD8, 0xFF, 0xDA, 0, 3, 1], 3, 1, 1),
            Err(Rejected::Damaged)
        );
        let mut cut = samples::jpeg_header(0xC0, w, h, &[0x11, 0x11, 0x11]);
        cut.truncate(cut.len() - 14);
        assert_eq!(
            decode_cost(Format::Jpeg, &cut, 3, 1, 1),
            Err(Rejected::Damaged)
        );
    }
}
