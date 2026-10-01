//! Judging image bytes: the format from the bytes themselves (never a file
//! name, an extension or a declared MIME type), then a full decode under the
//! limits of [`super`], so a damaged file, a non-image or one too large to
//! decode safely is refused before it is stored.

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
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_IMAGE_SIDE);
    limits.max_image_height = Some(MAX_IMAGE_SIDE);
    limits.max_alloc = Some(DECODE_MAX_ALLOC);
    reader.limits(limits);

    let decoder = reader.into_decoder().map_err(|e| match e {
        image::ImageError::Limits(_) => Rejected::TooManyPixels,
        _ => Rejected::Damaged,
    })?;
    let (width, height) = decoder.dimensions();
    if width == 0 || height == 0 {
        return Err(Rejected::Damaged);
    }
    if u64::from(width) * u64::from(height) > MAX_IMAGE_PIXELS {
        return Err(Rejected::TooManyPixels);
    }
    match DynamicImage::from_decoder(decoder) {
        Ok(_) => Ok(format),
        Err(image::ImageError::Limits(_)) => Err(Rejected::TooManyPixels),
        Err(_) => Err(Rejected::Damaged),
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
}
