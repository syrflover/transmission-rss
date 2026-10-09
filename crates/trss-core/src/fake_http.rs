//! The parts the fake HTTP services of the tests (AniList, Anissia, the feed
//! and the nyaa search) have in common that need no HTTP framework: whether a
//! request is refused, the padding of an answer and its split into chunks.
//! Each fake turns them into its framework's response and keeps its routes,
//! its data and the order of its own checks.

use serde_json::Value;

/// How many bytes one chunk of a chunked answer has.
pub const CHUNK_BYTES: usize = 64 * 1024;

/// A refused request: the status and the `Retry-After` seconds, if sent.
#[derive(Debug, PartialEq, Eq)]
pub struct Refusal {
    pub status: u16,
    pub retry_after: Option<u64>,
}

/// Whether the next request is refused, with the counters a test sets
/// (`rate_limited` and `failing` count the requests still to refuse): `429`
/// first, with `retry_after` as its `Retry-After` (none when `None`), then
/// `500`. A refusal counts its counter down.
pub fn refuse(
    rate_limited: &mut u32,
    retry_after: Option<u64>,
    failing: &mut u32,
) -> Option<Refusal> {
    if *rate_limited > 0 {
        *rate_limited -= 1;
        return Some(Refusal {
            status: 429,
            retry_after,
        });
    }
    if *failing > 0 {
        *failing -= 1;
        return Some(Refusal {
            status: 500,
            retry_after: None,
        });
    }
    None
}

/// `value` with `bytes` bytes of filler (a string of `x`) at `path`, as JSON
/// text: `["padding"]` sets `value["padding"]`, `["extensions", "padding"]`
/// sets `value["extensions"]` to an object holding it. Whatever was at the
/// first key is replaced. The filler stands for a field the client does not
/// know, which it must read past.
pub fn padded(mut value: Value, path: &[&str], bytes: usize) -> String {
    let (first, rest) = path.split_first().expect("a key to pad under");
    let mut filler = Value::String("x".repeat(bytes));
    for key in rest.iter().rev() {
        filler = Value::Object([((*key).to_owned(), filler)].into_iter().collect());
    }
    value[*first] = filler;
    value.to_string()
}

/// `body` in pieces of [`CHUNK_BYTES`], for an answer sent without a
/// `Content-Length`.
pub fn chunks(body: &[u8]) -> std::slice::Chunks<'_, u8> {
    body.chunks(CHUNK_BYTES)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn a_rate_limit_comes_before_a_failure_and_each_counts_its_own_counter_down() {
        let (mut limited, mut failing) = (2, 1);
        let mut seen = Vec::new();
        for _ in 0..5 {
            seen.push(refuse(&mut limited, Some(7), &mut failing));
        }
        let limit = || {
            Some(Refusal {
                status: 429,
                retry_after: Some(7),
            })
        };
        let failure = Some(Refusal {
            status: 500,
            retry_after: None,
        });
        assert_eq!(seen, [limit(), limit(), failure, None, None]);
        assert_eq!((limited, failing), (0, 0));
    }

    #[test]
    fn a_rate_limit_without_seconds_sends_no_retry_after() {
        let (mut limited, mut failing) = (1, 0);
        assert_eq!(
            refuse(&mut limited, None, &mut failing),
            Some(Refusal {
                status: 429,
                retry_after: None
            })
        );
    }

    #[test]
    fn nothing_is_refused_while_both_counters_are_zero() {
        let (mut limited, mut failing) = (0, 0);
        assert_eq!(refuse(&mut limited, Some(1), &mut failing), None);
    }

    #[test]
    fn padding_goes_under_the_key_and_leaves_the_rest_of_the_value() {
        let text = padded(json!({ "code": "ok" }), &["padding"], 10);
        let value: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value, json!({ "code": "ok", "padding": "xxxxxxxxxx" }));
    }

    #[test]
    fn padding_under_a_path_nests_the_filler_and_replaces_the_first_key() {
        let text = padded(
            json!({ "data": 1, "extensions": "old" }),
            &["extensions", "padding"],
            3,
        );
        let value: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(
            value,
            json!({ "data": 1, "extensions": { "padding": "xxx" } })
        );
    }

    #[test]
    fn the_padding_makes_the_text_that_much_longer() {
        let bare = padded(json!({}), &["padding"], 0).len();
        assert_eq!(
            padded(json!({}), &["padding"], 100_000).len(),
            bare + 100_000
        );
    }

    #[test]
    fn chunks_are_64_kib_and_the_last_is_what_is_left() {
        let body = vec![1u8; CHUNK_BYTES * 2 + 5];
        let sizes: Vec<usize> = chunks(&body).map(<[u8]>::len).collect();
        assert_eq!(sizes, [CHUNK_BYTES, CHUNK_BYTES, 5]);
        assert_eq!(CHUNK_BYTES, 65_536);
        assert_eq!(chunks(&[]).count(), 0);
    }
}
