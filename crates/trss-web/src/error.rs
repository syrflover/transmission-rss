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
//! | `internal`  | 500    | a server-side failure; details go to the log, not here    |
//!
//! `message` is shown to the user as is, so it is a full Korean sentence and
//! never contains secret values (see `docs/specs/collection.md`, 채널 URL의
//! 비밀 값).

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::{json, Value};

use trss_legacy::store::channels::ChannelError;

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
                json!({ "error": "invalid", "message": message }),
            ),
            ApiError::NotFound(message) => (
                StatusCode::NOT_FOUND,
                json!({ "error": "not_found", "message": message }),
            ),
            ApiError::Conflict { message, current } => {
                let mut body = json!({ "error": "conflict", "message": message });
                if let Some(current) = current {
                    body["current"] = current;
                }
                (StatusCode::CONFLICT, body)
            }
            ApiError::Unavailable(message) => (
                StatusCode::BAD_GATEWAY,
                json!({ "error": "unavailable", "message": message }),
            ),
            ApiError::Internal(detail) => {
                eprintln!("trss-web: internal error: {detail}");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    json!({ "error": "internal", "message": INTERNAL_MESSAGE }),
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
            // to open (`{ "rule_id": … }`).
            ChannelError::AlreadySubscribed { rule_id } => ApiError::Conflict {
                message:
                    "이 채널에서 이미 구독 중인 작품이에요. 구독 탭에서 그 구독을 열어 주세요."
                        .into(),
                current: Some(json!({ "rule_id": rule_id })),
            },
            ChannelError::Db(e) => ApiError::Internal(e.to_string()),
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
}
