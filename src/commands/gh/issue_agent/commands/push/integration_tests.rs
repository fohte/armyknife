//! Integration tests for push command using wiremock.

use super::*;
use crate::commands::gh::issue_agent::commands::IssueArgs;
use crate::commands::gh::issue_agent::commands::test_helpers::{
    TestSetup, create_comment_file, setup_local_comment, test_dir,
};
use crate::infra::github::RemoteComment;
use rstest::rstest;
use std::fs;
use tempfile::TempDir;

fn make_args(dry_run: bool, force: bool, edit_others: bool) -> PushArgs {
    make_args_full(dry_run, force, edit_others, false)
}

fn make_args_full(dry_run: bool, force: bool, edit_others: bool, allow_delete: bool) -> PushArgs {
    PushArgs {
        issue: IssueArgs {
            issue_number: 123,
            repo: Some("owner/repo".to_string()),
        },
        dry_run,
        force,
        edit_others,
        allow_delete,
    }
}

fn make_remote_comment<'a>(
    author: &'a str,
    id: &'a str,
    database_id: i64,
    body: &'a str,
) -> RemoteComment<'a> {
    RemoteComment {
        id,
        database_id,
        author,
        body,
    }
}

// Body update tests
#[rstest]
#[tokio::test]
async fn test_body_update_dry_run_no_api_call(test_dir: TempDir) {
    let (mock, storage) = TestSetup::new(test_dir.path())
        .local_body("Local")
        .remote_body("Remote")
        .build()
        .await;
    // Note: No mock_update_issue - it shouldn't be called in dry run

    let client = mock.client();
    let result = run_with_client_and_storage(
        &make_args(true, false, false),
        &client,
        &storage,
        "testuser",
    )
    .await;
    assert!(result.is_ok());
}

#[rstest]
#[tokio::test]
async fn test_body_update(test_dir: TempDir) {
    let (mock, storage) = TestSetup::new(test_dir.path())
        .local_body("Local")
        .remote_body("Remote")
        .build()
        .await;
    mock.mock_update_issue("owner", "repo", 123).await;
    mock.mock_get_issue("owner", "repo", 123).await;

    let client = mock.client();
    let result = run_with_client_and_storage(
        &make_args(false, false, false),
        &client,
        &storage,
        "testuser",
    )
    .await;
    assert!(result.is_ok());
}

#[rstest]
#[tokio::test]
async fn test_no_changes(test_dir: TempDir) {
    let (mock, storage) = TestSetup::new(test_dir.path()).build().await;
    // No API calls should be made when there are no changes

    let client = mock.client();
    let result = run_with_client_and_storage(
        &make_args(false, false, false),
        &client,
        &storage,
        "testuser",
    )
    .await;
    assert!(result.is_ok());
}

#[rstest]
#[tokio::test]
async fn test_updates_title(test_dir: TempDir) {
    let (mock, storage) = TestSetup::new(test_dir.path())
        .remote_title("Old Title")
        .local_title("New Title")
        .build()
        .await;
    mock.mock_update_issue("owner", "repo", 123).await;
    mock.mock_get_issue("owner", "repo", 123).await;

    let client = mock.client();
    let result = run_with_client_and_storage(
        &make_args(false, false, false),
        &client,
        &storage,
        "testuser",
    )
    .await;
    assert!(result.is_ok());
}

#[rstest]
#[tokio::test]
async fn test_updates_labels(test_dir: TempDir) {
    let (mock, storage) = TestSetup::new(test_dir.path())
        .local_labels(vec!["enhancement"])
        .build()
        .await;
    mock.mock_remove_label("owner", "repo", 123, "bug").await;
    mock.mock_add_labels("owner", "repo", 123).await;
    mock.mock_get_issue("owner", "repo", 123).await;

    let client = mock.client();
    let result = run_with_client_and_storage(
        &make_args(false, false, false),
        &client,
        &storage,
        "testuser",
    )
    .await;
    assert!(result.is_ok());
}

// Remote changed detection
#[rstest]
#[tokio::test]
async fn test_remote_changed_fails(test_dir: TempDir) {
    let (mock, storage) = TestSetup::new(test_dir.path())
        .local_ts("2024-01-01T00:00:00+00:00")
        .remote_ts("2024-01-02T00:00:00+00:00")
        .build()
        .await;

    let client = mock.client();
    let result = run_with_client_and_storage(
        &make_args(false, false, false),
        &client,
        &storage,
        "testuser",
    )
    .await;
    assert_eq!(
        result.unwrap_err().to_string(),
        "Remote has changed. Use --force to overwrite, or 'pull --force' to update local copy."
    );
}

#[rstest]
#[tokio::test]
async fn test_remote_changed_force_overrides(test_dir: TempDir) {
    let (mock, storage) = TestSetup::new(test_dir.path())
        .local_ts("2024-01-01T00:00:00+00:00")
        .remote_ts("2024-01-02T00:00:00+00:00")
        .build()
        .await;
    // No changes to push, just verifying force bypasses timestamp check

    let client = mock.client();
    let result = run_with_client_and_storage(
        &make_args(false, true, false),
        &client,
        &storage,
        "testuser",
    )
    .await;
    assert!(result.is_ok());
}

#[rstest]
#[tokio::test]
async fn test_invalid_repo_format(test_dir: TempDir) {
    let (mock, storage) = TestSetup::new(test_dir.path()).build().await;
    let args = PushArgs {
        issue: IssueArgs {
            issue_number: 123,
            repo: Some("invalid-format".to_string()),
        },
        dry_run: false,
        force: false,
        edit_others: false,
        allow_delete: false,
    };

    let client = mock.client();
    let result = run_with_client_and_storage(&args, &client, &storage, "testuser").await;
    assert_eq!(
        result.unwrap_err().to_string(),
        "Invalid input: Invalid repository format: invalid-format. Expected owner/repo"
    );
}

// Comment operations
#[rstest]
#[tokio::test]
async fn test_updates_own_comment(test_dir: TempDir) {
    let (mock, storage) = TestSetup::new(test_dir.path())
        .remote_comments(vec![make_remote_comment(
            "testuser", "IC_abc", 12345, "Original",
        )])
        .build()
        .await;
    setup_local_comment(
        test_dir.path(),
        "001_comment_12345.md",
        &create_comment_file(
            "testuser",
            "2024-01-01T12:00:00+00:00",
            "IC_abc",
            12345,
            "Updated",
        ),
    );
    mock.mock_update_comment("owner", "repo", 12345).await;
    mock.mock_get_issue("owner", "repo", 123).await;

    let client = mock.client();
    let result = run_with_client_and_storage(
        &make_args(false, false, false),
        &client,
        &storage,
        "testuser",
    )
    .await;
    assert!(result.is_ok());
}

#[rstest]
#[tokio::test]
async fn test_creates_new_comment(test_dir: TempDir) {
    let (mock, storage) = TestSetup::new(test_dir.path()).build().await;
    setup_local_comment(test_dir.path(), "new_my_comment.md", "New comment");
    mock.mock_create_comment("owner", "repo", 123).await;
    mock.mock_get_issue("owner", "repo", 123).await;

    let client = mock.client();
    let result = run_with_client_and_storage(
        &make_args(false, false, false),
        &client,
        &storage,
        "testuser",
    )
    .await;
    assert!(result.is_ok());
    // Verify the new comment file was removed after creation
    assert!(!test_dir.path().join("comments/new_my_comment.md").exists());
}

#[rstest]
#[tokio::test]
async fn test_edit_others_comment_denied_without_flag(test_dir: TempDir) {
    let (mock, storage) = TestSetup::new(test_dir.path())
        .remote_comments(vec![make_remote_comment(
            "otheruser",
            "IC_abc",
            12345,
            "Original",
        )])
        .build()
        .await;
    setup_local_comment(
        test_dir.path(),
        "001_comment_12345.md",
        &create_comment_file(
            "otheruser",
            "2024-01-01T12:00:00+00:00",
            "IC_abc",
            12345,
            "Modified",
        ),
    );

    let client = mock.client();
    let result = run_with_client_and_storage(
        &make_args(false, false, false),
        &client,
        &storage,
        "testuser",
    )
    .await;
    assert_eq!(
        result.unwrap_err().to_string(),
        "Cannot edit other user's comment: 001_comment_12345.md (author: otheruser). Use --edit-others to allow."
    );
}

#[rstest]
#[tokio::test]
async fn test_edit_others_comment_allowed_with_flag(test_dir: TempDir) {
    let (mock, storage) = TestSetup::new(test_dir.path())
        .remote_comments(vec![make_remote_comment(
            "otheruser",
            "IC_abc",
            12345,
            "Original",
        )])
        .build()
        .await;
    setup_local_comment(
        test_dir.path(),
        "001_comment_12345.md",
        &create_comment_file(
            "otheruser",
            "2024-01-01T12:00:00+00:00",
            "IC_abc",
            12345,
            "Modified",
        ),
    );
    mock.mock_update_comment("owner", "repo", 12345).await;
    mock.mock_get_issue("owner", "repo", 123).await;

    let client = mock.client();
    let result = run_with_client_and_storage(
        &make_args(false, false, true),
        &client,
        &storage,
        "testuser",
    )
    .await;
    assert!(result.is_ok());
}

#[rstest]
#[tokio::test]
async fn test_updates_metadata_after_push(test_dir: TempDir) {
    let (mock, storage) = TestSetup::new(test_dir.path())
        .remote_ts("2024-01-02T00:00:00+00:00")
        .local_ts("2024-01-01T00:00:00+00:00")
        .local_title("Old Title")
        .remote_title("New Title")
        .build()
        .await;
    // Local title differs from remote, so we need mock_update_issue
    mock.mock_update_issue("owner", "repo", 123).await;
    // mock_get_issue is already called by TestSetup for initial fetch,
    // and will be called again after push to refresh metadata

    let client = mock.client();
    let result = run_with_client_and_storage(
        &make_args(false, true, false),
        &client,
        &storage,
        "testuser",
    )
    .await;
    assert!(result.is_ok());

    let metadata_content = fs::read_to_string(test_dir.path().join("metadata.json")).unwrap();
    let metadata: serde_json::Value = serde_json::from_str(&metadata_content).unwrap();
    // After push, metadata should be updated from the mock response
    // The TestSetup uses "2024-01-02T00:00:00+00:00" format for remote_ts
    assert_eq!(metadata["updatedAt"], "2024-01-02T00:00:00+00:00");
}

// Comment deletion tests
#[rstest]
#[tokio::test]
async fn test_delete_comment_denied_without_flag(test_dir: TempDir) {
    let (mock, storage) = TestSetup::new(test_dir.path())
        .remote_comments(vec![make_remote_comment(
            "testuser", "IC_abc", 12345, "Original",
        )])
        .build()
        .await;
    // Don't create local comment file - simulating deletion

    let client = mock.client();
    let result = run_with_client_and_storage(
        &make_args_full(false, false, false, false),
        &client,
        &storage,
        "testuser",
    )
    .await;
    assert_eq!(
        result.unwrap_err().to_string(),
        "Cannot delete comment (database_id: 12345). Use --allow-delete to allow."
    );
}

#[rstest]
#[tokio::test]
async fn test_delete_comment_allowed_with_flag(test_dir: TempDir) {
    let (mock, storage) = TestSetup::new(test_dir.path())
        .remote_comments(vec![make_remote_comment(
            "testuser", "IC_abc", 12345, "Original",
        )])
        .build()
        .await;
    // Don't create local comment file - simulating deletion
    mock.mock_delete_comment("owner", "repo", 12345).await;
    mock.mock_get_issue("owner", "repo", 123).await;

    let client = mock.client();
    let result = run_with_client_and_storage(
        &make_args_full(false, false, false, true),
        &client,
        &storage,
        "testuser",
    )
    .await;
    assert!(result.is_ok());
}

#[rstest]
#[tokio::test]
async fn test_delete_others_comment_requires_allow_delete(test_dir: TempDir) {
    let (mock, storage) = TestSetup::new(test_dir.path())
        .remote_comments(vec![make_remote_comment(
            "otheruser",
            "IC_abc",
            12345,
            "Original",
        )])
        .build()
        .await;
    // Don't create local comment file - simulating deletion

    let client = mock.client();
    let result = run_with_client_and_storage(
        &make_args_full(false, false, false, false),
        &client,
        &storage,
        "testuser",
    )
    .await;
    assert_eq!(
        result.unwrap_err().to_string(),
        "Cannot delete other user's comment (database_id: 12345, author: otheruser). Use --allow-delete to allow."
    );
}
