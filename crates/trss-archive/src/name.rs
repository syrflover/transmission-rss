//! Member names: decoded to text, then judged as paths (`docs/specs/subtitles.md`,
//! 압축 해제의 격리와 한도, the row for paths).

use crate::{Limits, Refusal};

/// A raw member name as text. A name that is valid UTF-8 is used as it is. A
/// name that is not is a Korean Windows tool's (CP949, which `encoding_rs`
/// names EUC-KR) when it decodes without errors, and `fallback`'s otherwise
/// (a ZIP's CP437, a tar's lossy UTF-8).
pub(crate) fn decode(raw: &[u8], fallback: impl FnOnce() -> String) -> String {
    if let Ok(text) = std::str::from_utf8(raw) {
        return text.to_owned();
    }
    match encoding_rs::EUC_KR.decode_without_bom_handling_and_without_replacement(raw) {
        Some(text) => text.into_owned(),
        None => fallback(),
    }
}

/// Like [`decode`], with the lossy UTF-8 as the fallback.
pub(crate) fn decode_lossy(raw: &[u8]) -> String {
    decode(raw, || String::from_utf8_lossy(raw).into_owned())
}

/// Whether `path` may be a member's path: the components it has when it is, or
/// [`Refusal::Path`] when it is not. A path is refused when it is empty, has
/// an empty, `.` or `..` component (an absolute path has an empty first
/// one), has a NUL or a backslash, begins with a drive letter, or is over the
/// limits of parts, bytes or bytes of one component.
pub(crate) fn check_path<'p>(path: &'p str, limits: &Limits) -> Result<Vec<&'p str>, Refusal> {
    let bad = || Refusal::Path {
        path: path.to_owned(),
    };
    if path.is_empty() || path.len() > limits.path_bytes {
        return Err(bad());
    }
    if path.contains(['\0', '\\']) {
        return Err(bad());
    }
    let parts: Vec<&str> = path.split('/').collect();
    if parts.len() > limits.path_parts {
        return Err(bad());
    }
    for part in &parts {
        if part.is_empty() || *part == "." || *part == ".." || part.len() > limits.name_bytes {
            return Err(bad());
        }
    }
    let first = parts[0].as_bytes();
    if first.len() >= 2 && first[0].is_ascii_alphabetic() && first[1] == b':' {
        return Err(bad());
    }
    Ok(parts)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(path: &str) -> bool {
        check_path(path, &Limits::default()).is_ok()
    }

    #[test]
    fn plain_paths_pass() {
        assert!(ok("a.ass"));
        assert!(ok("sub/a b.ass"));
        assert!(ok("자막/1화.smi"));
        assert!(ok("a..b/c"));
    }

    #[test]
    fn bad_paths_are_refused() {
        for path in [
            "",
            ".",
            "..",
            "../x.ass",
            "a/../x",
            "a/./x",
            "/etc/passwd",
            "a//b",
            "a/",
            "a\\b",
            "a\0b",
            "C:",
            "C:/x",
            "c:x",
        ] {
            assert!(!ok(path), "{path:?}");
        }
    }

    #[test]
    fn limits_apply_to_parts_bytes_and_names() {
        let limits = Limits::default();
        assert!(check_path(&vec!["a"; 16].join("/"), &limits).is_ok());
        assert!(check_path(&vec!["a"; 17].join("/"), &limits).is_err());
        assert!(check_path(&"a".repeat(255), &limits).is_ok());
        assert!(check_path(&"a".repeat(256), &limits).is_err());
        let long = vec!["a".repeat(200); 6].join("/");
        assert!(long.len() > 1024);
        assert!(check_path(&long, &limits).is_err());
    }

    #[test]
    fn names_that_are_not_utf8_decode_as_cp949() {
        let (bytes, _, _) = encoding_rs::EUC_KR.encode("자막.ass");
        assert!(std::str::from_utf8(&bytes).is_err());
        assert_eq!(decode_lossy(&bytes), "자막.ass");
        assert_eq!(decode_lossy("자막.ass".as_bytes()), "자막.ass");
        // Bytes CP949 cannot decode take the fallback.
        assert_eq!(decode(&[0xff, 0xff], || "fb".to_owned()), "fb");
    }
}
