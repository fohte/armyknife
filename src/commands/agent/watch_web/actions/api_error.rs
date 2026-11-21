use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::json;

use super::close_checks::CloseChecks;

#[derive(Debug)]
pub struct ApiError {
    status: StatusCode,
    pub(super) message: String,
    code: Option<&'static str>,
    checks: Option<Box<CloseChecks>>,
}

impl ApiError {
    pub(super) fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: message.into(),
            code: None,
            checks: None,
        }
    }

    pub(super) fn not_found() -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            message: "session not found".to_string(),
            code: None,
            checks: None,
        }
    }

    pub(super) fn decision_not_found() -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            message: "decision not found".to_string(),
            code: None,
            checks: None,
        }
    }

    pub(super) fn conflict(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::CONFLICT,
            message: message.into(),
            code: None,
            checks: None,
        }
    }

    pub(super) fn operation(error: impl std::fmt::Display) -> Self {
        Self {
            status: StatusCode::BAD_GATEWAY,
            message: error.to_string(),
            code: None,
            checks: None,
        }
    }

    pub(super) fn internal(error: impl std::fmt::Display) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: error.to_string(),
            code: None,
            checks: None,
        }
    }

    pub(super) fn requires_close_checks(error: &anyhow::Error) -> bool {
        if error
            .downcast_ref::<crate::commands::agent::SessionLifecyclePersistenceError>()
            .is_some()
        {
            return false;
        }

        error
            .downcast_ref::<crate::shared::worktree::DirtyWorktreeError>()
            .is_some()
            || error
                .downcast_ref::<crate::shared::cleanup::WorktreeProcessGuardError>()
                .is_some()
            || error
                .downcast_ref::<crate::shared::worktree_delete::UnmergedWorktreeError>()
                .is_some()
    }

    pub(super) fn close(error: anyhow::Error, checks: Option<CloseChecks>) -> Self {
        if error
            .downcast_ref::<crate::commands::agent::SessionLifecyclePersistenceError>()
            .is_some()
        {
            return Self::internal(error);
        }

        if let Some(dirty_worktree) =
            error.downcast_ref::<crate::shared::worktree::DirtyWorktreeError>()
        {
            return Self {
                status: StatusCode::CONFLICT,
                message: format!(
                    "Worktree contains uncommitted changes or untracked files: {}; commit or move them before closing",
                    dirty_worktree.worktree_path().display(),
                ),
                code: Some("dirty_worktree"),
                checks: checks.map(Box::new),
            };
        }

        if let Some(process_guard) =
            error.downcast_ref::<crate::shared::cleanup::WorktreeProcessGuardError>()
        {
            let message = match process_guard {
                crate::shared::cleanup::WorktreeProcessGuardError::ActiveProcesses {
                    worktree_path,
                    process_list,
                } => format!(
                    "Worktree '{}' contains running processes: {process_list}. Stop them and retry, or close the session while keeping the worktree.",
                    worktree_path.display(),
                ),
                crate::shared::cleanup::WorktreeProcessGuardError::InspectionUnavailable {
                    worktree_path,
                } => format!(
                    "Could not inspect processes in worktree '{}'. Close the session while keeping the worktree, then retry.",
                    worktree_path.display(),
                ),
            };
            return Self {
                status: StatusCode::CONFLICT,
                message,
                code: Some("worktree_process_guard"),
                checks: checks.map(Box::new),
            };
        }

        let code = error
            .downcast_ref::<crate::shared::worktree_delete::UnmergedWorktreeError>()
            .map(|_| "unmerged_worktree");
        Self {
            status: StatusCode::CONFLICT,
            message: error.to_string(),
            code,
            checks: checks.map(Box::new),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = match (self.code, self.checks) {
            (Some(code), Some(checks)) => json!({
                "error": self.message,
                "code": code,
                "checks": checks,
            }),
            (Some(code), None) => json!({ "error": self.message, "code": code }),
            (None, _) => json!({ "error": self.message }),
        };
        (self.status, Json(body)).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn close_lifecycle_persistence_errors_are_internal_server_errors() {
        let error = ApiError::close(
            anyhow::Error::new(
                crate::commands::agent::SessionLifecyclePersistenceError::new(anyhow::anyhow!(
                    "database unavailable"
                )),
            ),
            None,
        );

        assert_eq!(
            (error.status, error.message, error.code),
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "the session closed, but its lifecycle storage cleanup failed; retry Close when storage is available: database unavailable".to_string(),
                None,
            ),
        );
    }

    #[test]
    fn close_dirty_worktree_errors_have_a_code_and_no_unavailable_force_option() {
        let error = ApiError::close(
            anyhow::Error::new(crate::shared::worktree::DirtyWorktreeError::new(
                std::path::PathBuf::from("/worktrees/dirty"),
            )),
            None,
        );

        assert_eq!(
            (error.status, error.message, error.code),
            (
                StatusCode::CONFLICT,
                "Worktree contains uncommitted changes or untracked files: /worktrees/dirty; commit or move them before closing".to_string(),
                Some("dirty_worktree"),
            ),
        );
    }

    #[test]
    fn close_process_guard_errors_offer_a_keep_worktree_retry() {
        let error = ApiError::close(
            anyhow::Error::new(
                crate::shared::cleanup::WorktreeProcessGuardError::ActiveProcesses {
                    worktree_path: std::path::PathBuf::from("/worktrees/busy"),
                    process_list: "42 (editor)".to_string(),
                },
            ),
            None,
        );

        assert_eq!(
            (error.status, error.message, error.code),
            (
                StatusCode::CONFLICT,
                "Worktree '/worktrees/busy' contains running processes: 42 (editor). Stop them and retry, or close the session while keeping the worktree.".to_string(),
                Some("worktree_process_guard"),
            ),
        );
    }

    #[test]
    fn close_process_inspection_errors_offer_a_keep_worktree_retry() {
        let error = ApiError::close(
            anyhow::Error::new(
                crate::shared::cleanup::WorktreeProcessGuardError::InspectionUnavailable {
                    worktree_path: std::path::PathBuf::from("/worktrees/uncheckable"),
                },
            ),
            None,
        );

        assert_eq!(
            (error.status, error.message, error.code),
            (
                StatusCode::CONFLICT,
                "Could not inspect processes in worktree '/worktrees/uncheckable'. Close the session while keeping the worktree, then retry.".to_string(),
                Some("worktree_process_guard"),
            ),
        );
    }
}
