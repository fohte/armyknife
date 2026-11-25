use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use axum::Json;
use axum::extract::{Path as RoutePath, State};
use axum::http::StatusCode;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::metadata::ReviewDocumentKind;
use super::{
    ReviewDocumentActionRegistry as ActionRegistry, ReviewDocumentEditLock,
    ReviewDocumentEditRequest, ReviewDocumentRoots, ReviewDocumentSummary, ReviewQueueLoader,
    SessionLoader, active_review_document, save,
};
pub(super) use crate::shared::human_in_the_loop::ReviewDocumentAction;
use crate::shared::human_in_the_loop::{
    LockGuard, ReviewDocumentActionResult, clear_action_result, document_hash, save_action_result,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ActionStatus {
    Running,
    Failed,
    Succeeded,
}

#[derive(Debug, Clone)]
pub(super) struct ActionRecord {
    action: ReviewDocumentAction,
    path: PathBuf,
    session_cwd: PathBuf,
    summary: ReviewDocumentSummary,
    frontmatter: serde_json::Value,
    body: String,
    status: ActionStatus,
    revision: String,
    retry_started_at: Option<DateTime<Utc>>,
    document_hash: String,
}

pub(super) type ActionExecutor = Arc<
    dyn Fn(ActionRecord) -> Pin<Box<dyn Future<Output = anyhow::Result<String>> + Send>>
        + Send
        + Sync,
>;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ReviewDocumentActionRequest {
    id: String,
    revision: String,
    frontmatter: serde_json::Value,
    body: String,
    action: ReviewDocumentAction,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ReviewDocumentActionResponse {
    success: bool,
    revision: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

pub(super) async fn execute_review_document(
    State((load_session, load_review_queue, edit_lock, roots, actions)): State<(
        SessionLoader,
        ReviewQueueLoader,
        ReviewDocumentEditLock,
        ReviewDocumentRoots,
        ActionRegistry,
    )>,
    RoutePath(session_id): RoutePath<String>,
    Json(request): Json<ReviewDocumentActionRequest>,
) -> Result<Json<ReviewDocumentActionResponse>, StatusCode> {
    let edit_guard = edit_lock.lock().await;
    let key = (session_id.clone(), request.id.clone());
    let existing = actions.lock().await.records.get(&key).cloned();

    let (mut record, revision, retrying) = match existing {
        Some(record) => {
            if record.action != request.action {
                return Err(StatusCode::CONFLICT);
            }
            if !should_run_again(record.status)? {
                return Ok(Json(ReviewDocumentActionResponse {
                    success: true,
                    revision: record.revision,
                    error: None,
                }));
            }
            let retrying = record.status == ActionStatus::Failed;
            let frontmatter =
                approved_frontmatter(record.summary.kind, request.action, request.frontmatter)?;
            clear_previous_action_result(&record.path)?;
            if same_action_content(
                &record.frontmatter,
                &record.body,
                &frontmatter,
                &request.body,
            ) {
                (record.clone(), record.revision.clone(), retrying)
            } else {
                let saved_revision = save::save_document_for_session(
                    load_session.clone(),
                    load_review_queue.clone(),
                    session_id.clone(),
                    ReviewDocumentEditRequest {
                        id: request.id.clone(),
                        revision: request.revision,
                        frontmatter: frontmatter.clone(),
                        body: request.body.clone(),
                    },
                    Some(b'1'),
                )
                .await?;
                let updated = ActionRecord {
                    frontmatter,
                    body: request.body,
                    status: ActionStatus::Running,
                    revision: saved_revision.clone(),
                    retry_started_at: None,
                    document_hash: document_hash(&record.path).map_err(|error| {
                        tracing::warn!(%error, "failed to hash approved review document");
                        StatusCode::INTERNAL_SERVER_ERROR
                    })?,
                    ..record
                };
                (updated, saved_revision, retrying)
            }
        }
        None => {
            let session = load_session(&session_id)
                .map_err(|error| {
                    tracing::warn!(%error, "failed to load session for review document action");
                    StatusCode::INTERNAL_SERVER_ERROR
                })?
                .ok_or(StatusCode::NOT_FOUND)?;
            let queue = load_review_queue(&session);
            let marker = active_review_document(&session, queue.clone())
                .filter(|marker| marker.id == request.id)
                .ok_or(StatusCode::NOT_FOUND)?;
            let summary = super::summary_for_marker(&session, &marker, queue, &roots);
            validate_action(summary.kind, request.action)?;
            let frontmatter =
                approved_frontmatter(summary.kind, request.action, request.frontmatter)?;
            clear_previous_action_result(&marker.path)?;
            let saved_revision = save::save_document_for_session(
                load_session.clone(),
                load_review_queue.clone(),
                session_id.clone(),
                ReviewDocumentEditRequest {
                    id: request.id.clone(),
                    revision: request.revision,
                    frontmatter: frontmatter.clone(),
                    body: request.body.clone(),
                },
                Some(b'1'),
            )
            .await?;
            let document_hash = document_hash(&marker.path).map_err(|error| {
                tracing::warn!(%error, "failed to hash approved review document");
                StatusCode::INTERNAL_SERVER_ERROR
            })?;
            let record = ActionRecord {
                action: request.action,
                path: marker.path,
                session_cwd: session.cwd.clone(),
                summary,
                frontmatter,
                body: request.body,
                status: ActionStatus::Running,
                revision: saved_revision.clone(),
                retry_started_at: None,
                document_hash,
            };
            (record, saved_revision, false)
        }
    };

    if retrying {
        crate::shared::human_in_the_loop::ApprovalManager::new(&record.path)
            .save()
            .map_err(|error| {
                tracing::warn!(%error, "failed to restore review document approval");
                StatusCode::INTERNAL_SERVER_ERROR
            })?;
    }
    record.status = ActionStatus::Running;
    actions
        .lock()
        .await
        .records
        .insert(key.clone(), record.clone());
    let executor = actions.lock().await.executor.clone();
    drop(edit_guard);

    let result = async {
        wait_for_review_child_exit(&record.path).await?;
        let mut action_record = record.clone();
        action_record.retry_started_at.get_or_insert_with(Utc::now);
        actions
            .lock()
            .await
            .records
            .insert(key.clone(), action_record.clone());
        executor(action_record).await
    }
    .await;
    let (succeeded, message) = match &result {
        Ok(message) => (true, message.clone()),
        Err(error) => (false, format!("{error:#}")),
    };
    let action_result = ReviewDocumentActionResult::new(
        record.action,
        succeeded,
        message.clone(),
        record.document_hash.clone(),
        document_hash(&record.path).ok(),
    );
    if let Err(error) = save_action_result(&record.path, &action_result) {
        tracing::error!(%error, "failed to save review document action result");
    }
    let mut records = actions.lock().await;
    if let Some(current) = records.records.get_mut(&key) {
        current.status = if result.is_ok() {
            ActionStatus::Succeeded
        } else {
            ActionStatus::Failed
        };
    }
    match result {
        Ok(_) => Ok(Json(ReviewDocumentActionResponse {
            success: true,
            revision,
            error: None,
        })),
        Err(error) => {
            tracing::warn!(%error, "review document action failed");
            Ok(Json(ReviewDocumentActionResponse {
                success: false,
                revision,
                error: Some(error.to_string()),
            }))
        }
    }
}

fn clear_previous_action_result(document_path: &Path) -> Result<(), StatusCode> {
    clear_action_result(document_path).map_err(|error| {
        tracing::warn!(%error, "failed to clear previous review action result");
        StatusCode::INTERNAL_SERVER_ERROR
    })
}

fn same_action_content(
    saved_frontmatter: &serde_json::Value,
    saved_body: &str,
    request_frontmatter: &serde_json::Value,
    request_body: &str,
) -> bool {
    saved_frontmatter == request_frontmatter && saved_body == request_body
}

async fn wait_for_review_child_exit(path: &Path) -> anyhow::Result<()> {
    tokio::time::timeout(Duration::from_secs(5), async {
        while LockGuard::is_review_child_running(path) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .map_err(|_| anyhow::anyhow!("The review editor did not close in time"))
}

fn validate_action(
    kind: ReviewDocumentKind,
    action: ReviewDocumentAction,
) -> Result<(), StatusCode> {
    let valid = matches!(
        (kind, action),
        (
            ReviewDocumentKind::IssueComment,
            ReviewDocumentAction::PostComment
        ) | (ReviewDocumentKind::PrDraft, ReviewDocumentAction::CreatePr)
            | (
                ReviewDocumentKind::ReviewReplies,
                ReviewDocumentAction::PostReplies
            )
            | (
                ReviewDocumentKind::IssueBody,
                ReviewDocumentAction::UpdateIssue
            )
    );
    if valid {
        Ok(())
    } else {
        Err(StatusCode::BAD_REQUEST)
    }
}

fn should_run_again(status: ActionStatus) -> Result<bool, StatusCode> {
    match status {
        ActionStatus::Running => Err(StatusCode::CONFLICT),
        ActionStatus::Failed => Ok(true),
        ActionStatus::Succeeded => Ok(false),
    }
}

fn approved_frontmatter(
    kind: ReviewDocumentKind,
    action: ReviewDocumentAction,
    mut frontmatter: serde_json::Value,
) -> Result<serde_json::Value, StatusCode> {
    if !frontmatter.is_object() {
        return Err(StatusCode::BAD_REQUEST);
    }
    let root = frontmatter.as_object_mut().ok_or(StatusCode::BAD_REQUEST)?;
    if kind == ReviewDocumentKind::PrDraft {
        let steps = root
            .get_mut("steps")
            .and_then(serde_json::Value::as_object_mut)
            .ok_or(StatusCode::BAD_REQUEST)?;
        steps.insert("submit".to_owned(), serde_json::Value::Bool(true));
    } else if matches!(
        action,
        ReviewDocumentAction::PostComment
            | ReviewDocumentAction::PostReplies
            | ReviewDocumentAction::UpdateIssue
    ) {
        root.insert("submit".to_owned(), serde_json::Value::Bool(true));
    }
    Ok(frontmatter)
}

pub(super) async fn run_action(record: &ActionRecord) -> anyhow::Result<String> {
    match record.action {
        ReviewDocumentAction::CreatePr => {
            let target_repo_id = record
                .summary
                .target
                .repository
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("Repository is unavailable"))?;
            crate::commands::ai::pr_draft::submit_draft_for_review(
                record.path.clone(),
                &record.session_cwd,
                target_repo_id,
            )
            .await
        }
        ReviewDocumentAction::PostComment => {
            let (issue_number, repo) = issue_target(record)?;
            crate::commands::gh::issue_agent::commands::push::push_review_document(
                issue_number,
                repo,
                crate::commands::gh::issue_agent::commands::push::ReviewDocumentPushScope::Comment {
                    path: record.path.clone(),
                    retry_started_at: record
                        .retry_started_at
                        .ok_or_else(|| anyhow::anyhow!("Action start time is unavailable"))?,
                },
            )
            .await?;
            Ok("Issue comment posted.".to_owned())
        }
        ReviewDocumentAction::UpdateIssue => {
            let (issue_number, repo) = issue_target(record)?;
            crate::commands::gh::issue_agent::commands::push::push_review_document(
                issue_number,
                repo,
                crate::commands::gh::issue_agent::commands::push::ReviewDocumentPushScope::IssueBody {
                    path: record.path.clone(),
                },
            )
            .await?;
            Ok("Issue body updated.".to_owned())
        }
        ReviewDocumentAction::PostReplies => {
            let pr_number = record
                .summary
                .target
                .pull_request_number
                .ok_or_else(|| anyhow::anyhow!("Pull request number is unavailable"))?;
            let repo = record
                .summary
                .target
                .repository
                .clone()
                .ok_or_else(|| anyhow::anyhow!("Repository is unavailable"))?;
            crate::commands::gh::pr_review::push_review_replies(
                pr_number,
                repo,
                record
                    .retry_started_at
                    .ok_or_else(|| anyhow::anyhow!("Action start time is unavailable"))?,
            )
            .await?;
            Ok("Pull request replies posted.".to_owned())
        }
    }
}

fn issue_target(record: &ActionRecord) -> anyhow::Result<(u64, String)> {
    let issue_number = record
        .summary
        .target
        .issue_number
        .and_then(|number| u64::try_from(number).ok())
        .ok_or_else(|| anyhow::anyhow!("Issue number is unavailable"))?;
    let repo = record
        .summary
        .target
        .repository
        .clone()
        .ok_or_else(|| anyhow::anyhow!("Repository is unavailable"))?;
    Ok((issue_number, repo))
}

#[cfg(test)]
mod tests {
    use super::{
        ActionStatus, ReviewDocumentAction, approved_frontmatter, same_action_content,
        should_run_again, validate_action,
    };
    use crate::commands::agent::watch_web::review_document::metadata::ReviewDocumentKind;
    use axum::http::StatusCode;
    use rstest::rstest;
    use serde_json::json;
    use tempfile::TempDir;

    #[rstest]
    #[case::issue_comment(
        ReviewDocumentKind::IssueComment,
        ReviewDocumentAction::PostComment,
        true
    )]
    #[case::pr_draft(ReviewDocumentKind::PrDraft, ReviewDocumentAction::CreatePr, true)]
    #[case::review_replies(
        ReviewDocumentKind::ReviewReplies,
        ReviewDocumentAction::PostReplies,
        true
    )]
    #[case::issue_body(ReviewDocumentKind::IssueBody, ReviewDocumentAction::UpdateIssue, true)]
    #[case::wrong_kind(
        ReviewDocumentKind::IssueBody,
        ReviewDocumentAction::PostComment,
        false
    )]
    fn action_matches_only_its_document_kind(
        #[case] kind: ReviewDocumentKind,
        #[case] action: ReviewDocumentAction,
        #[case] expected_valid: bool,
    ) {
        assert_eq!(
            validate_action(kind, action),
            if expected_valid {
                Ok(())
            } else {
                Err(StatusCode::BAD_REQUEST)
            },
        );
    }

    #[rstest]
    #[case::running(ActionStatus::Running, Err(StatusCode::CONFLICT))]
    #[case::failed(ActionStatus::Failed, Ok(true))]
    #[case::succeeded(ActionStatus::Succeeded, Ok(false))]
    fn action_retry_runs_only_after_a_failed_attempt(
        #[case] status: ActionStatus,
        #[case] expected: Result<bool, StatusCode>,
    ) {
        assert_eq!(should_run_again(status), expected);
    }

    #[rstest]
    #[case::same_content(json!({ "submit": true }), "Example body", true)]
    #[case::changed_frontmatter(json!({ "submit": false }), "Example body", false)]
    #[case::changed_body(json!({ "submit": true }), "Updated body", false)]
    fn failed_action_retry_reuses_only_the_saved_content(
        #[case] frontmatter: serde_json::Value,
        #[case] body: &str,
        #[case] expected: bool,
    ) {
        assert_eq!(
            same_action_content(
                &json!({ "submit": true }),
                "Example body",
                &frontmatter,
                body,
            ),
            expected,
        );
    }

    #[rstest]
    #[case::pr_draft(
        ReviewDocumentKind::PrDraft,
        ReviewDocumentAction::CreatePr,
        json!({
            "title": "Example title",
            "steps": { "ready-for-translation": false, "submit": false }
        }),
        json!({
            "title": "Example title",
            "steps": { "ready-for-translation": false, "submit": true }
        })
    )]
    #[case::issue_comment(
        ReviewDocumentKind::IssueComment,
        ReviewDocumentAction::PostComment,
        json!({ "submit": false }),
        json!({ "submit": true })
    )]
    fn direct_action_marks_the_document_approved(
        #[case] kind: ReviewDocumentKind,
        #[case] action: ReviewDocumentAction,
        #[case] frontmatter: serde_json::Value,
        #[case] expected: serde_json::Value,
    ) {
        assert_eq!(
            approved_frontmatter(kind, action, frontmatter),
            Ok(expected),
        );
    }

    #[tokio::test]
    async fn review_launcher_lock_does_not_block_the_review_child_wait() {
        use crate::shared::human_in_the_loop::LockGuard;

        let directory = TempDir::new().expect("fixture directory should exist");
        let path = directory.path().join("draft.md");
        std::fs::write(&path, "Fixture draft").expect("fixture draft should be written");
        let lock = LockGuard::acquire(&path).expect("review lock should be acquired");
        let result = super::wait_for_review_child_exit(&path)
            .await
            .map_err(|error| error.to_string());
        let parent_lock_remains = LockGuard::is_locked(&path);

        assert_eq!((result, parent_lock_remains), (Ok(()), true));
        drop(lock);
    }
}
