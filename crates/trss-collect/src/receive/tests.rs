//! The add, record and rename of one item, against the fake Transmission
//! (`trss_transmission::fake`) and an app database in memory.

use super::*;

#[test]
fn each_kind_of_add_failure_has_its_own_sentence_before_transmissions_words() {
    let redactor = Redactor::none();
    assert_eq!(
        failure_reason(
            &AddError::Unreachable("connection refused".into()),
            &redactor
        ),
        "Transmission에 연결하지 못했어요: connection refused"
    );
    assert_eq!(
        failure_reason(&AddError::Rpc("operation timed out".into()), &redactor),
        "Transmission이 응답하지 않았어요: operation timed out"
    );
    assert_eq!(
        failure_reason(
            &AddError::Rejected("invalid or corrupt torrent file".into()),
            &redactor
        ),
        "Transmission이 토렌트를 받지 않았어요: invalid or corrupt torrent file"
    );
}

#[test]
fn an_add_failure_reason_hides_secrets_and_is_cut_to_the_kept_length() {
    let mut redactor = Redactor::none();
    redactor.add("SECRETTOKEN0123456789");
    let reason = failure_reason(
        &AddError::Rejected(format!(
            "cannot use SECRETTOKEN0123456789 {}",
            "x".repeat(400)
        )),
        &redactor,
    );
    assert!(!reason.contains("SECRETTOKEN0123456789"), "{reason}");
    assert!(reason.starts_with("Transmission이 토렌트를 받지 않았어요: cannot use ***"));
    assert_eq!(reason.chars().count(), MAX_REASON_CHARS);
}

#[test]
fn a_trname_name_is_told_apart_from_a_release_name() {
    let dir = std::path::Path::new("/media/anime/Slime/Season 04");
    for name in [
        "Slime S04E38.mkv",
        "SLIME S04E38.mkv",
        "Slime S04E105.mkv",
        "Slime S04E05.5.mp4",
    ] {
        assert!(looks_renamed(name, dir), "{name}");
    }
    for name in [
        "[SubsPlease] Tensei Shitara Slime Datta Ken - 62 (1080p) [AAAA0006].mkv",
        "Tensura S04E62.mkv",
        "S04E05.mkv",
        "Slime.S04E05.1080p.WEB.mkv",
        "Slime S04E05 (1080p).mkv",
        "SlimeS04E05.mkv",
        "Slime Special.mkv",
    ] {
        assert!(!looks_renamed(name, dir), "{name}");
    }
    assert!(!looks_renamed(
        "Slime S04E38.mkv",
        std::path::Path::new("/")
    ));
}
