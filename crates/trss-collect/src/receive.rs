//! How one item's torrent goes to Transmission, the same for the rule cycle
//! ([`crate::cycle`]) and the `다시 받기` command
//! ([`crate::commands::receive_once`]).

use trss_transmission::{AddError, Redactor};

use crate::context::MAX_REASON_CHARS;

/// Why the cycle recorded an item as failed whose task ended in a panic
/// before its result was written.
pub const ADD_PANICKED: &str = "토렌트를 추가하다 내부 오류가 났어요.";

/// Why an add failed, as history keeps it: one sentence per kind of failure,
/// with Transmission's own words after it, secret values replaced and cut to
/// [`MAX_REASON_CHARS`].
pub fn failure_reason(err: &AddError, redactor: &Redactor) -> String {
    let text = match err {
        AddError::Unreachable(err) => format!("Transmission에 연결하지 못했어요: {err}"),
        AddError::Rpc(err) => format!("Transmission이 응답하지 않았어요: {err}"),
        AddError::Rejected(result) => format!("Transmission이 토렌트를 받지 않았어요: {result}"),
    };
    redactor
        .apply(&text)
        .chars()
        .take(MAX_REASON_CHARS)
        .collect()
}

#[cfg(test)]
mod tests {
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
}
