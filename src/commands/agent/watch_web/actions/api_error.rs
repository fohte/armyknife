use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::json;

#[derive(Debug)]
pub struct ApiError {
    status: StatusCode,
    pub(super) message: String,
    code: Option<&'static str>,
}

impl ApiError {
    pub(super) fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: message.into(),
            code: None,
        }
    }

    pub(super) fn not_found() -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            message: "session not found".to_string(),
            code: None,
        }
    }

    pub(super) fn decision_not_found() -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            message: "decision not found".to_string(),
            code: None,
        }
    }

    pub(super) fn conflict(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::CONFLICT,
            message: message.into(),
            code: None,
        }
    }

    pub(super) fn operation(error: impl std::fmt::Display) -> Self {
        Self {
            status: StatusCode::BAD_GATEWAY,
            message: error.to_string(),
            code: None,
        }
    }

    pub(super) fn internal(error: impl std::fmt::Display) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: error.to_string(),
            code: None,
        }
    }

    pub(super) fn close(error: anyhow::Error) -> Self {
        let code = error
            .downcast_ref::<crate::shared::worktree_delete::UnmergedWorktreeError>()
            .map(|_| "unmerged_worktree");
        Self {
            status: StatusCode::CONFLICT,
            message: error.to_string(),
            code,
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = match self.code {
            Some(code) => json!({ "error": self.message, "code": code }),
            None => json!({ "error": self.message }),
        };
        (self.status, Json(body)).into_response()
    }
}
