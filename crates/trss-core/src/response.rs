//! Handling what an outside service answers: the pause a `429` asks for, and
//! reading a body up to a size limit.
//!
//! Nothing here knows an HTTP client. The callers hand over the header's text
//! and a way to ask for the next chunk, so this crate does not depend on
//! `reqwest` (the clients of different crates use different versions).

use std::time::Duration;

/// How long to wait when a `429` answer names no time.
pub const DEFAULT_RETRY_AFTER: Duration = Duration::from_secs(60);

/// The longest `Retry-After` honoured as given.
pub const MAX_RETRY_AFTER: Duration = Duration::from_secs(3600);

/// The most bytes of an answer of an outside service that are read: an AniList
/// or Anissia answer, an RSS feed. Real ones are far smaller (a tracker's RSS
/// of 75 to 100 items is 50 to 300 KB), so 2 MiB leaves a wide margin, and
/// several are held at once in a process of a few hundred MiB.
pub const MAX_ANSWER_BYTES: usize = 2 * 1024 * 1024;

/// The most bytes one downloaded file may have: a download of the server
/// browser past it is canceled while it comes, and a subtitle source stops
/// receiving past it. The two mean the same limit.
pub const MAX_DOWNLOAD_FILE_BYTES: u64 = 200 * 1024 * 1024;

/// The pause a `429` answer asks for, from its `Retry-After` header's text
/// (`None` when it has none or it is not text). Whole seconds only: a date, or
/// anything else that does not read as a number of seconds, gives
/// [`DEFAULT_RETRY_AFTER`], and no answer is honoured for longer than
/// [`MAX_RETRY_AFTER`].
pub fn retry_after(header: Option<&str>) -> Duration {
    header
        .and_then(|v| v.trim().parse::<u64>().ok())
        .map_or(DEFAULT_RETRY_AFTER, Duration::from_secs)
        .min(MAX_RETRY_AFTER)
}

/// A body that could not be read within its limit.
#[derive(Debug, PartialEq, Eq)]
pub enum BodyError<E> {
    /// The body is longer than the limit.
    TooLarge,
    /// The client failed to read it.
    Read(E),
}

/// The body of a response, refused once it passes `max` bytes: at once when
/// the response announces a longer body, otherwise as soon as the chunks read
/// so far do. (The announced length is not trusted to be true, so the chunks
/// are counted either way.) A body of exactly `max` bytes is read whole.
///
/// `next` gives the next chunk, `None` at the end. With `reserve`, the room
/// for the body is taken once, the announced length or else `max` (of which
/// only what arrives is touched), so the buffer is never grown by copying into
/// a larger one beside the old.
pub async fn read_or_refuse<R, E, C: AsRef<[u8]>>(
    max: usize,
    announced: Option<u64>,
    reserve: bool,
    mut source: R,
    mut next: impl AsyncFnMut(&mut R) -> Result<Option<C>, E>,
) -> Result<Vec<u8>, BodyError<E>> {
    if announced.is_some_and(|n| n > max as u64) {
        return Err(BodyError::TooLarge);
    }
    let mut body = if reserve {
        Vec::with_capacity(announced.map_or(max, |n| n as usize))
    } else {
        Vec::new()
    };
    while let Some(chunk) = next(&mut source).await.map_err(BodyError::Read)? {
        let chunk = chunk.as_ref();
        if body.len() + chunk.len() > max {
            return Err(BodyError::TooLarge);
        }
        body.extend_from_slice(chunk);
    }
    Ok(body)
}

/// The body of a response, cut at `max` bytes: what comes after is not read.
/// For a caller that needs only the beginning (a page to look into, the size
/// of an error answer up to a bound), where a longer body is no failure.
pub async fn read_cut<R, E, C: AsRef<[u8]>>(
    max: usize,
    mut source: R,
    mut next: impl AsyncFnMut(&mut R) -> Result<Option<C>, E>,
) -> Result<Vec<u8>, E> {
    let mut body = Vec::new();
    while let Some(chunk) = next(&mut source).await? {
        body.extend_from_slice(chunk.as_ref());
        if body.len() >= max {
            body.truncate(max);
            break;
        }
    }
    Ok(body)
}

#[cfg(test)]
mod tests {
    use std::convert::Infallible;

    use super::*;

    /// The chunks of a body not yet read.
    type Parts = std::vec::IntoIter<&'static [u8]>;

    fn parts(chunks: &[&'static [u8]]) -> Parts {
        Vec::from(chunks).into_iter()
    }

    async fn next(parts: &mut Parts) -> Result<Option<&'static [u8]>, Infallible> {
        Ok(parts.next())
    }

    #[test]
    fn retry_after_reads_seconds_and_falls_back_to_a_minute() {
        assert_eq!(retry_after(Some("30")), Duration::from_secs(30));
        assert_eq!(retry_after(Some(" 7 ")), Duration::from_secs(7));
        assert_eq!(retry_after(Some("0")), Duration::ZERO);
        // The most it honours is an hour.
        assert_eq!(retry_after(Some("3600")), Duration::from_secs(3600));
        assert_eq!(retry_after(Some("86400")), Duration::from_secs(3600));
        // No header, or one that is no number of seconds (a date, a fraction,
        // a negative number), names no time.
        for header in [
            None,
            Some(""),
            Some("soon"),
            Some("Wed, 21 Oct 2026 07:28:00 GMT"),
            Some("1.5"),
            Some("-5"),
        ] {
            assert_eq!(retry_after(header), DEFAULT_RETRY_AFTER, "{header:?}");
        }
    }

    #[tokio::test]
    async fn a_body_within_the_limit_is_read_whole_and_one_over_it_is_refused() {
        let body = read_or_refuse(6, None, false, parts(&[b"abc", b"def"]), next).await;
        assert_eq!(body, Ok(b"abcdef".to_vec()));
        let body = read_or_refuse(6, None, false, parts(&[b"abc", b"defg"]), next).await;
        assert_eq!(body, Err(BodyError::TooLarge));
        let body = read_or_refuse(6, None, false, parts(&[]), next).await;
        assert_eq!(body, Ok(Vec::new()));
    }

    #[tokio::test]
    async fn a_longer_announced_length_refuses_the_body_before_it_is_read() {
        let mut asked = false;
        let body = read_or_refuse(6, Some(7), false, (), async |_| {
            asked = true;
            Ok::<Option<&[u8]>, Infallible>(Some(b"a"))
        })
        .await;
        assert_eq!(body, Err(BodyError::TooLarge));
        assert!(!asked);
        // The announced length is not trusted: the chunks are counted too.
        let body = read_or_refuse(6, Some(2), false, parts(&[b"abcdefg"]), next).await;
        assert_eq!(body, Err(BodyError::TooLarge));
        let body = read_or_refuse(6, Some(6), true, parts(&[b"abcdef"]), next).await;
        assert_eq!(body, Ok(b"abcdef".to_vec()));
    }

    #[tokio::test]
    async fn a_failing_read_is_told_apart_from_a_body_that_is_too_large() {
        let body = read_or_refuse(6, None, false, (), async |_| {
            Err::<Option<&[u8]>, _>("reset")
        })
        .await;
        assert_eq!(body, Err(BodyError::Read("reset")));
    }

    #[tokio::test]
    async fn a_body_that_reserves_room_takes_it_once() {
        let body = read_or_refuse(1000, Some(10), true, parts(&[b"0123456789"]), next)
            .await
            .unwrap();
        assert_eq!(body.capacity(), 10);
        let body = read_or_refuse(1000, None, true, parts(&[b"0123456789"]), next)
            .await
            .unwrap();
        assert_eq!(body.capacity(), 1000);
    }

    #[tokio::test]
    async fn a_cut_body_ends_at_the_limit_and_the_rest_is_not_read() {
        let body = read_cut(4, parts(&[b"ab", b"cdef", b"gh"]), next).await;
        assert_eq!(body, Ok(b"abcd".to_vec()));
        // Not read: the chunk after the one that reached the limit.
        let mut asked = 0;
        let body = read_cut(4, (), async |_| {
            asked += 1;
            Ok::<Option<&[u8]>, Infallible>(Some(b"abcd"))
        })
        .await;
        assert_eq!(body, Ok(b"abcd".to_vec()));
        assert_eq!(asked, 1);
        // A shorter body is read whole.
        let body = read_cut(4, parts(&[b"ab"]), next).await;
        assert_eq!(body, Ok(b"ab".to_vec()));
        let body = read_cut(4, (), async |_| Err::<Option<&[u8]>, _>("reset")).await;
        assert_eq!(body, Err("reset"));
    }
}
