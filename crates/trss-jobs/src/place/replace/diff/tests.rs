use std::path::Path;

use sha2::{Digest, Sha256};

use super::*;

/// An SRT of `n` cues, one second each, `text` ahead of the cue's number.
fn srt(n: u64, text: &str) -> Vec<u8> {
    let mut out = String::new();
    for i in 0..n {
        out.push_str(&format!(
            "{}\n00:00:{:02},000 --> 00:00:{:02},500\n{text} {}\n\n",
            i + 1,
            i,
            i,
            i + 1
        ));
    }
    out.into_bytes()
}

fn seen_of(bytes: &[u8]) -> FileSeen {
    FileSeen {
        size: bytes.len() as u64,
        sha256: hex(&Sha256::digest(bytes)),
        object: "1:1".to_owned(),
        mtime: 0,
        lines: None,
    }
}

fn file(dir: &Path, name: &str, bytes: &[u8]) -> std::path::PathBuf {
    let at = dir.join(name);
    std::fs::write(&at, bytes).unwrap();
    at
}

fn reason(compared: Compared) -> String {
    match compared {
        Compared::Unreadable(reason) => reason,
        other => panic!("compared: {other:?}"),
    }
}

#[test]
fn a_file_that_is_what_the_plan_saw_is_compared() {
    let dir = tempfile::tempdir().unwrap();
    let old = srt(3, "가");
    let at = file(dir.path(), "a.srt", &old);

    let compared = compare_files(&at, &seen_of(&old), &srt(3, "나"), "srt");

    let Compared::Diff(diff) = compared else {
        panic!("compared: {compared:?}");
    };
    assert_eq!(
        (
            diff.dialogue.added,
            diff.dialogue.changed,
            diff.dialogue.removed
        ),
        (0, 3, 0)
    );
    assert_eq!(diff.dialogue.lines.len(), 3);
}

#[test]
fn a_current_file_changed_after_the_plan_saw_it_is_not_compared() {
    let dir = tempfile::tempdir().unwrap();
    let seen = seen_of(&srt(3, "가"));
    // The same length, other bytes: only the SHA-256 tells.
    let at = file(dir.path(), "a.srt", &srt(3, "다"));

    let compared = compare_files(&at, &seen, &srt(3, "나"), "srt");

    assert_eq!(reason(compared), "비교하는 사이 현재 자막이 바뀌었어요");
}

#[test]
fn a_current_file_that_grew_past_the_limit_since_the_plan_is_changed_not_read_whole() {
    let dir = tempfile::tempdir().unwrap();
    let seen = seen_of(&srt(3, "가"));
    let at = file(dir.path(), "a.srt", &vec![b' '; MAX_BYTES as usize + 100]);

    assert_eq!(
        reason(compare_files(&at, &seen, &srt(3, "나"), "srt")),
        "비교하는 사이 현재 자막이 바뀌었어요"
    );
    // Whatever the file's size, no more than one byte past the limit is read.
    assert_eq!(read_current(&at).unwrap().len() as u64, MAX_BYTES + 1);
}

#[test]
fn a_file_larger_than_the_limit_is_not_compared_and_the_side_is_named() {
    let dir = tempfile::tempdir().unwrap();
    let old = srt(3, "가");
    let at = file(dir.path(), "a.srt", &old);
    // The size is what the plan recorded: the file is not read at all.
    let big = FileSeen {
        size: MAX_BYTES + 1,
        ..seen_of(&old)
    };
    assert_eq!(
        reason(compare_files(&at, &big, &srt(3, "나"), "srt")),
        "현재 자막: 파일이 커서 내용을 비교하지 않았어요"
    );
    let mut new = srt(3, "나");
    new.resize(MAX_BYTES as usize + 1, b' ');
    assert_eq!(
        reason(compare_files(&at, &seen_of(&old), &new, "srt")),
        "새 자막: 파일이 커서 내용을 비교하지 않았어요"
    );
}

#[test]
fn a_file_of_exactly_the_limit_is_compared() {
    let dir = tempfile::tempdir().unwrap();
    let mut old = srt(3, "가");
    old.resize(MAX_BYTES as usize, b'\n');
    let at = file(dir.path(), "a.srt", &old);
    let mut new = srt(3, "나");
    new.resize(MAX_BYTES as usize, b'\n');

    let compared = compare_files(&at, &seen_of(&old), &new, "srt");

    assert!(matches!(compared, Compared::Diff(_)), "{compared:?}");
}

#[test]
fn a_current_file_that_cannot_be_opened_is_not_compared() {
    let dir = tempfile::tempdir().unwrap();
    let seen = seen_of(&srt(3, "가"));
    let missing = dir.path().join("gone.srt");
    assert!(reason(compare_files(&missing, &seen, &srt(3, "나"), "srt"))
        .starts_with("현재 자막: 읽지 못했어요"));

    let target = file(dir.path(), "a.srt", &srt(3, "가"));
    let link = dir.path().join("link.srt");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    assert_eq!(
        reason(compare_files(&link, &seen, &srt(3, "나"), "srt")),
        "현재 자막: 일반 파일이 아니라 읽지 못했어요"
    );
}

#[test]
fn a_side_whose_content_cannot_be_read_is_named() {
    let srt = srt(3, "가");
    let picture = b"PG\x00\x01";

    assert!(reason(compare_bytes(picture, "sup", &srt, "srt")).starts_with("현재 자막: "));
    assert!(reason(compare_bytes(&srt, "srt", picture, "sup")).starts_with("새 자막: "));
    // Both: the current one is said first.
    assert!(reason(compare_bytes(picture, "sup", picture, "sup")).starts_with("현재 자막: "));
}

#[test]
fn an_engine_that_panics_leaves_the_plan_not_compared() {
    let compared = guarded(|| panic!("a bug the engine hit"));
    assert_eq!(reason(compared), FAILED);
    let fine = guarded(|| Compared::Unreadable("까닭".to_owned()));
    assert_eq!(reason(fine), "까닭");
}
