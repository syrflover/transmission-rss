//! The one error shape of the JSON API.
//!
//! Every failed `/api` call answers
//! `{ "error": <code>, "message": <Korean sentence>, "current"?: <value> }`:
//!
//! | code        | status | meaning                                                   |
//! | ----------- | ------ | --------------------------------------------------------- |
//! | `invalid`   | 400    | the request cannot be applied as sent; `message` says why |
//! | `not_found` | 404    | the addressed item does not exist (any more)              |
//! | `conflict`  | 409    | someone saved first; `current` carries the server's value |
//! |             |        | when the handler has it, so the screen can compare        |
//! | `unavailable` | 502  | a service the call needs (Anissia) did not answer; `message` says why |
//! |             | 503    | this server cannot take the call now (an upload with no turn free); nothing was done |
//! | `internal`  | 500    | a server-side failure; details go to the log, not here    |
//! | `host_not_allowed` | 421 | the request's `Host` is not one the web answers to ([`crate::web::origin_guard`]) |
//! | `forbidden` | 403    | a change or a WebSocket from another origin ([`crate::web::origin_guard`]), a remote screen's socket among them |
//! | `gone`      | 410    | a remote screen's socket for a browser run that is no longer the job's |
//!
//! `message` is shown to the user as is, so it is a full Korean sentence and
//! never contains secret values (see `docs/requirements/collection.md`, 채널 URL의
//! 비밀 값).

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::Serialize;
#[cfg(test)]
use serde_json::json;
use serde_json::Value;

use trss_anilist::AnilistError;
use trss_collect::store::channels::ChannelError;

/// The `error` of a failed call's body.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    Invalid,
    NotFound,
    Conflict,
    Unavailable,
    Internal,
    HostNotAllowed,
    Forbidden,
    Gone,
}

/// The body of every failed `/api` call.
#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ErrorBody {
    pub error: ErrorCode,
    /// A full Korean sentence for the screen to show as is.
    pub message: String,
    /// The server's current value, on a `conflict` that has one; absent
    /// otherwise.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional, type = "unknown"))]
    pub current: Option<Value>,
}

/// The `current` of the `conflict` that a channel already has a rule following
/// the anime: the rule, for the screen to open.
#[derive(Debug, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct AlreadySubscribed {
    pub rule_id: String,
}

impl ErrorBody {
    pub fn new(error: ErrorCode, message: impl Into<String>) -> Self {
        ErrorBody {
            error,
            message: message.into(),
            current: None,
        }
    }
}

#[derive(Debug)]
pub enum ApiError {
    Invalid(String),
    NotFound(String),
    Conflict {
        message: String,
        current: Option<Value>,
    },
    /// A service outside the app that the call needs cannot be used now; the
    /// message says why, for the screen to show with a retry.
    Unavailable(String),
    /// This server cannot take the call now and did nothing; trying again
    /// later is safe.
    Busy(String),
    /// The detail is logged, never sent.
    Internal(String),
}

impl ApiError {
    pub fn invalid(message: impl Into<String>) -> Self {
        ApiError::Invalid(message.into())
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        ApiError::NotFound(message.into())
    }

    /// A conflict that carries the server's current value for comparison.
    pub fn conflict_with(current: impl serde::Serialize) -> Self {
        ApiError::Conflict {
            message: CONFLICT_MESSAGE.into(),
            current: serde_json::to_value(current).ok(),
        }
    }
}

const CONFLICT_MESSAGE: &str =
    "다른 곳에서 먼저 저장했어요. 지금 값을 확인하고 다시 저장해 주세요.";
const INTERNAL_MESSAGE: &str = "서버에서 처리하지 못했어요. 잠시 뒤 다시 시도해 주세요.";

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, body) = match self {
            ApiError::Invalid(message) => (
                StatusCode::BAD_REQUEST,
                ErrorBody::new(ErrorCode::Invalid, message),
            ),
            ApiError::NotFound(message) => (
                StatusCode::NOT_FOUND,
                ErrorBody::new(ErrorCode::NotFound, message),
            ),
            ApiError::Conflict { message, current } => (
                StatusCode::CONFLICT,
                ErrorBody {
                    current,
                    ..ErrorBody::new(ErrorCode::Conflict, message)
                },
            ),
            ApiError::Unavailable(message) => (
                StatusCode::BAD_GATEWAY,
                ErrorBody::new(ErrorCode::Unavailable, message),
            ),
            ApiError::Busy(message) => (
                StatusCode::SERVICE_UNAVAILABLE,
                ErrorBody::new(ErrorCode::Unavailable, message),
            ),
            ApiError::Internal(detail) => {
                eprintln!("trss: internal error: {detail}");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    ErrorBody::new(ErrorCode::Internal, INTERNAL_MESSAGE),
                )
            }
        };
        (status, Json(body)).into_response()
    }
}

/// Store errors map to the API codes. `ChannelError`'s own messages never
/// contain URLs, but they are English and technical, so user-facing text is
/// chosen here; handlers that can say more (or attach `current`) map the
/// error themselves before it reaches this conversion.
impl From<ChannelError> for ApiError {
    fn from(e: ChannelError) -> Self {
        match e {
            ChannelError::NotFound { kind, .. } => ApiError::NotFound(match kind {
                "rule" => "규칙을 찾지 못했어요. 이미 삭제됐을 수 있어요.".into(),
                _ => "채널을 찾지 못했어요. 이미 삭제됐을 수 있어요.".into(),
            }),
            ChannelError::Conflict { .. } | ChannelError::OrderMismatch { .. } => {
                ApiError::Conflict {
                    message: CONFLICT_MESSAGE.into(),
                    current: None,
                }
            }
            ChannelError::RuleChannelChange { .. } => ApiError::Invalid(
                "규칙의 채널은 바꿀 수 없어요. 다른 채널에는 새 규칙을 만들어 주세요.".into(),
            ),
            ChannelError::Invalid(reason) => ApiError::Invalid(reason.into()),
            // `current` names the rule that follows the anime, for the screen
            // to open ([`AlreadySubscribed`]).
            ChannelError::AlreadySubscribed { rule_id } => ApiError::Conflict {
                message:
                    "이 채널에서 이미 구독 중인 작품이에요. 구독 탭에서 그 구독을 열어 주세요."
                        .into(),
                current: serde_json::to_value(AlreadySubscribed { rule_id }).ok(),
            },
            ChannelError::Db(e) => ApiError::Internal(e.to_string()),
        }
    }
}

/// What a refused AniList call says on the API: a busy AniList asks the
/// person to try again after the wait it named (at least one second), a
/// failed store is internal, and any other failure is logged and told as
/// "could not connect".
impl From<AnilistError> for ApiError {
    fn from(e: AnilistError) -> Self {
        match e {
            AnilistError::Busy { retry_after } => ApiError::invalid(format!(
                "AniList 요청이 몰려 있어요. {}초 뒤에 다시 해 주세요.",
                retry_after.as_secs().max(1)
            )),
            AnilistError::Store(e) => ApiError::Internal(e.to_string()),
            e => {
                eprintln!("trss: AniList: {e}");
                ApiError::invalid("AniList에 연결하지 못했어요. 잠시 뒤 다시 해 주세요.")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use http_body_util::BodyExt;

    use super::*;

    async fn body(e: ApiError) -> (StatusCode, Value) {
        let response = e.into_response();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (status, serde_json::from_slice(&bytes).unwrap())
    }

    #[tokio::test]
    async fn conflict_carries_the_current_value() {
        let (status, json) = body(ApiError::conflict_with(json!({ "version": 4 }))).await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(json["error"], "conflict");
        assert_eq!(json["current"]["version"], 4);
        assert!(json["message"].as_str().unwrap().ends_with('.'));
    }

    #[tokio::test]
    async fn internal_detail_is_not_sent() {
        let (status, json) = body(ApiError::Internal("disk I/O error at /data".into())).await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(json["error"], "internal");
        assert!(!json.to_string().contains("/data"));
    }

    #[tokio::test]
    async fn store_errors_map_to_api_codes() {
        let conflict = ChannelError::Conflict {
            kind: "rule",
            id: "r1".into(),
            expected: 3,
            actual: 4,
        };
        assert_eq!(body(conflict.into()).await.0, StatusCode::CONFLICT);
        let missing = ChannelError::NotFound {
            kind: "channel",
            id: "c1".into(),
        };
        assert_eq!(body(missing.into()).await.0, StatusCode::NOT_FOUND);
        let moved = ChannelError::RuleChannelChange {
            rule_id: "r1".into(),
        };
        assert_eq!(body(moved.into()).await.0, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn anilist_failures_map_to_api_answers() {
        let busy = |retry_after| AnilistError::Busy { retry_after };
        let (status, json) = body(busy(std::time::Duration::from_secs(30)).into()).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(
            json["message"],
            "AniList 요청이 몰려 있어요. 30초 뒤에 다시 해 주세요."
        );
        // A wait under a second is told as one second.
        let (_, json) = body(busy(std::time::Duration::from_millis(200)).into()).await;
        assert_eq!(
            json["message"],
            "AniList 요청이 몰려 있어요. 1초 뒤에 다시 해 주세요."
        );
        let (status, json) = body(AnilistError::Status(500).into()).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(
            json["message"],
            "AniList에 연결하지 못했어요. 잠시 뒤 다시 해 주세요."
        );
        let (status, json) =
            body(AnilistError::Store(trss_core::DbError::PathNotConfigured).into()).await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(json["error"], "internal");
    }
}
