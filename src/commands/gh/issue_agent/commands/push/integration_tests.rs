//! Integration tests for push command using wiremock.

use super::*;
use crate::commands::gh::issue_agent::commands::test_helpers::{
    TestSetup, approve_file, create_comment_file, create_comment_file_with_updated_at,
    setup_local_comment, test_dir,
};
use crate::commands::gh::issue_agent::models::IssueFrontmatter;
use crate::commands::gh::issue_agent::storage::{CommentSnapshot, IssueContent, IssueStorage};
use crate::commands::gh::issue_agent::testing::factories;
use crate::infra::github::{GitHubMockServer, RemoteComment};
use indoc::formatdoc;
use rstest::{fixture, rstest};
use tempfile::TempDir;

fn make_args(dry_run: bool, force: bool, edit_others: bool) -> PushArgs {
    make_args_full(dry_run, force, edit_others, false)
}

fn make_args_full(dry_run: bool, force: bool, edit_others: bool, allow_delete: bool) -> PushArgs {
    PushArgs {
        target: "123".to_string(),
        repo: Some("owner/repo".to_string()),
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

async fn comment_write_requests(
    mock: &GitHubMockServer,
) -> Vec<(String, Option<serde_json::Value>)> {
    mock.received_requests()
        .await
        .into_iter()
        .filter(|request| {
            let path = request.url.path();
            path.ends_with("/comments") || path.contains("/issues/comments/")
        })
        .map(|request| {
            (
                request.method.to_string(),
                serde_json::from_slice(&request.body).ok(),
            )
        })
        .collect()
}

fn issue_frontmatter(title: &str, labels: &[&str]) -> IssueFrontmatter {
    let issue = factories::issue_with(|issue| {
        issue.number = 123;
        issue.title = title.to_string();
        issue.labels = factories::labels(labels);
    });
    IssueFrontmatter::from_issue(&issue)
}

fn write_local_issue(storage: &IssueStorage, frontmatter: &IssueFrontmatter, body: &str) {
    let yaml = serde_yaml::to_string(frontmatter).unwrap();
    let content = formatdoc! {"
        ---
        {}---

        {}
    ", yaml, body};
    std::fs::write(storage.dir().join("issue.md"), content).unwrap();
}

fn remove_issue_approval(storage: &IssueStorage) {
    crate::shared::testing::init_approval_dir();
    let _ = crate::shared::human_in_the_loop::ApprovalManager::new(&storage.dir().join("issue.md"))
        .remove();
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
    let ctx = mock.repo("owner", "repo");
    ctx.issue(123).update().await;
    ctx.issue(123).get().await;

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
async fn test_no_changes_dry_run(test_dir: TempDir) {
    let (mock, storage) = TestSetup::new(test_dir.path()).build().await;
    // No API calls should be made in dry run with no changes

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
async fn test_updates_title(test_dir: TempDir) {
    let (mock, storage) = TestSetup::new(test_dir.path())
        .remote_title("Old Title")
        .local_title("New Title")
        .build()
        .await;
    let ctx = mock.repo("owner", "repo");
    ctx.issue(123).update().await;
    ctx.issue(123).get().await;

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
    let ctx = mock.repo("owner", "repo");
    ctx.issue(123).remove_label("bug").await;
    ctx.issue(123).add_labels().await;
    ctx.issue(123).get().await;

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
async fn test_no_op_push_preserves_remote_only_edit_and_snapshot(test_dir: TempDir) {
    let (mock, storage) = TestSetup::new(test_dir.path())
        .remote_labels(vec!["remote-label"])
        .build()
        .await;
    storage
        .save_issue(&issue_frontmatter("Title", &["bug"]), "Body")
        .unwrap();
    remove_issue_approval(&storage);
    let expected_issue = storage.read_issue().unwrap();
    let expected_base = storage.read_base_issue().unwrap();

    let result = run_with_client_and_storage(
        &make_args(false, false, false),
        &mock.client(),
        &storage,
        "testuser",
    )
    .await
    .map_err(|error| error.to_string());

    assert_eq!(
        (
            result,
            storage.read_issue().unwrap(),
            storage.read_base_issue().unwrap(),
        ),
        (Ok(()), expected_issue, expected_base),
    );
}

#[rstest]
#[tokio::test]
async fn test_remote_label_change_is_preserved_when_pushing_comment(test_dir: TempDir) {
    let (mock, storage) = TestSetup::new(test_dir.path())
        .remote_labels(vec!["remote-label"])
        .build()
        .await;
    storage
        .save_issue(&issue_frontmatter("Title", &["bug"]), "Body")
        .unwrap();
    setup_local_comment(test_dir.path(), "new_agent_comment.md", "Added comment");
    let comment_path = test_dir.path().join("comments/new_agent_comment.md");
    approve_file(&comment_path);
    mock.repo("owner", "repo").issue(123).create_comment().await;

    let result = run_with_client_and_storage(
        &make_args(false, false, false),
        &mock.client(),
        &storage,
        "testuser",
    )
    .await
    .map_err(|error| error.to_string());

    assert_eq!(
        (result, storage.read_metadata().unwrap().labels),
        (Ok(()), vec!["remote-label".to_string()]),
    );
}

#[rstest]
#[tokio::test]
async fn test_divergent_local_and_remote_labels_conflict(test_dir: TempDir) {
    let (mock, storage) = TestSetup::new(test_dir.path())
        .remote_labels(vec!["remote-label"])
        .build()
        .await;
    storage
        .save_issue(&issue_frontmatter("Title", &["bug"]), "Body")
        .unwrap();
    write_local_issue(
        &storage,
        &issue_frontmatter("Title", &["local-label"]),
        "Body",
    );

    let result = run_with_client_and_storage(
        &make_args(false, false, false),
        &mock.client(),
        &storage,
        "testuser",
    )
    .await
    .map_err(|error| error.to_string());

    assert_eq!(
        result,
        Err(
            "Remote has changed. Use --force to overwrite, or 'pull --force' to update local copy."
                .to_string()
        ),
    );
}

#[rstest]
#[tokio::test]
async fn test_local_label_change_requires_issue_approval(test_dir: TempDir) {
    let (mock, storage) = TestSetup::new(test_dir.path()).build().await;
    storage
        .save_issue(&issue_frontmatter("Title", &["bug"]), "Body")
        .unwrap();
    write_local_issue(
        &storage,
        &issue_frontmatter("Title", &["local-label"]),
        "Body",
    );
    remove_issue_approval(&storage);

    let result = run_with_client_and_storage(
        &make_args(false, false, false),
        &mock.client(),
        &storage,
        "testuser",
    )
    .await
    .map_err(|error| error.to_string());
    let expected_error = format!(
        "The following files have not been approved. Run 'a gh issue-agent review <file>' for each:\n  {}",
        storage.dir().join("issue.md").display()
    );

    assert_eq!(result, Err(expected_error));
}

#[rstest]
#[tokio::test]
async fn test_missing_base_snapshot_keeps_legacy_label_approval(test_dir: TempDir) {
    let (mock, storage) = TestSetup::new(test_dir.path())
        .remote_labels(vec!["remote-label"])
        .build()
        .await;
    remove_issue_approval(&storage);

    let result = run_with_client_and_storage(
        &make_args(false, false, false),
        &mock.client(),
        &storage,
        "testuser",
    )
    .await
    .map_err(|error| error.to_string());
    let expected_error = format!(
        "The following files have not been approved. Run 'a gh issue-agent review <file>' for each:\n  {}",
        storage.dir().join("issue.md").display()
    );

    assert_eq!(result, Err(expected_error));
}

#[rstest]
#[tokio::test]
async fn test_converged_issue_fields_refresh_snapshot_without_rewriting_issue(test_dir: TempDir) {
    let (mock, storage) = TestSetup::new(test_dir.path())
        .remote_title("Current title")
        .build()
        .await;
    storage
        .save_issue(&issue_frontmatter("Previous title", &["bug"]), "Body")
        .unwrap();
    let current_frontmatter = issue_frontmatter("Current title", &["bug"]);
    write_local_issue(&storage, &current_frontmatter, "Body");
    let expected_issue = IssueContent {
        frontmatter: current_frontmatter,
        body: "Body".to_string(),
    };
    let mut expected_base_frontmatter = issue_frontmatter("Current title", &["bug"]);
    expected_base_frontmatter.readonly.created_at = "2024-01-01T00:00:00+00:00".to_string();
    expected_base_frontmatter.readonly.updated_at = "2024-01-02T00:00:00+00:00".to_string();
    let expected_base = IssueContent {
        frontmatter: expected_base_frontmatter,
        body: "Body".to_string(),
    };

    let result = run_with_client_and_storage(
        &make_args(false, false, false),
        &mock.client(),
        &storage,
        "testuser",
    )
    .await
    .map_err(|error| error.to_string());

    assert_eq!(
        (
            result,
            storage.read_issue().unwrap(),
            storage.read_base_issue().unwrap(),
        ),
        (Ok(()), expected_issue, Some(expected_base)),
    );
}

// Remote changed detection
#[rstest]
#[tokio::test]
async fn test_remote_changed_fails(test_dir: TempDir) {
    // Scenario: Local body was modified, and remote body was also edited.
    // This should fail due to conflict.
    let (mock, storage) = TestSetup::new(test_dir.path())
        .local_body("Modified body")
        .local_last_edited_at(Some("2024-01-01T00:00:00+00:00"))
        .remote_body("Original body")
        .remote_last_edited_at(Some("2024-01-02T00:00:00+00:00"))
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
    // Scenario: Same conflict as above, but with --force flag.
    // This should succeed because force bypasses conflict detection.
    let (mock, storage) = TestSetup::new(test_dir.path())
        .local_body("Modified body")
        .local_last_edited_at(Some("2024-01-01T00:00:00+00:00"))
        .remote_body("Original body")
        .remote_last_edited_at(Some("2024-01-02T00:00:00+00:00"))
        .build()
        .await;

    // Add mock for update since there will be a body change
    mock.repo("owner", "repo").issue(123).update().await;

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
        target: "123".to_string(),
        repo: Some("invalid-format".to_string()),
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
async fn test_refreshes_own_comment_after_push(test_dir: TempDir) {
    let (mock, storage) = TestSetup::new(test_dir.path())
        .remote_comments(vec![make_remote_comment(
            "testuser", "IC_abc", 12345, "Original",
        )])
        .comments_after_initial_fetch(Some((
            vec![make_remote_comment("testuser", "IC_abc", 12345, "Updated")],
            "2024-01-02T12:00:00Z",
        )))
        .build()
        .await;
    storage
        .save_base_comments(&[CommentSnapshot {
            id: "IC_abc".to_string(),
            body: "Original".to_string(),
        }])
        .unwrap();
    setup_local_comment(
        test_dir.path(),
        "001_comment_12345.md",
        &create_comment_file_with_updated_at(
            "testuser",
            "2024-01-01T12:00:00+00:00",
            "2024-01-01T12:00:00+00:00",
            "IC_abc",
            12345,
            "Updated",
        ),
    );
    approve_file(&test_dir.path().join("comments/001_comment_12345.md"));
    let ctx = mock.repo("owner", "repo");
    ctx.comment().update().await;
    ctx.issue(123).get().await;

    let result = run_with_client_and_storage(
        &make_args(false, false, false),
        &mock.client(),
        &storage,
        "testuser",
    )
    .await;
    let comment_path = test_dir.path().join("comments/001_comment_12345.md");
    let comment_after_push = std::fs::read_to_string(&comment_path).unwrap();

    assert_eq!(
        (
            result.map_err(|error| error.to_string()),
            comment_after_push,
            storage.read_base_comments().unwrap(),
        ),
        (
            Ok(()),
            formatdoc! {"
                <!-- author: testuser -->
                <!-- createdAt: 2024-01-01T12:00:00+00:00 -->
                <!-- updatedAt: 2024-01-02T12:00:00+00:00 -->
                <!-- id: IC_abc -->
                <!-- databaseId: 12345 -->

                Updated
            "
            },
            Some(vec![CommentSnapshot {
                id: "IC_abc".to_string(),
                body: "Updated".to_string(),
            }]),
        ),
    );
}

#[rstest]
#[tokio::test]
async fn test_updates_own_comment_again_without_pull_after_refresh(test_dir: TempDir) {
    let (mock, storage) = TestSetup::new(test_dir.path())
        .remote_comments(vec![make_remote_comment(
            "testuser", "IC_abc", 12345, "Original",
        )])
        .remote_comments_updated_at("2024-01-01T12:00:00Z")
        .comments_after_initial_fetch(Some((
            vec![make_remote_comment("testuser", "IC_abc", 12345, "Updated")],
            "2024-01-02T12:00:00Z",
        )))
        .build()
        .await;
    storage
        .save_base_comments(&[CommentSnapshot {
            id: "IC_abc".to_string(),
            body: "Original".to_string(),
        }])
        .unwrap();
    setup_local_comment(
        test_dir.path(),
        "001_comment_12345.md",
        &create_comment_file_with_updated_at(
            "testuser",
            "2024-01-01T12:00:00+00:00",
            "2024-01-01T12:00:00+00:00",
            "IC_abc",
            12345,
            "Updated",
        ),
    );
    let comment_path = test_dir.path().join("comments/001_comment_12345.md");
    approve_file(&comment_path);
    let ctx = mock.repo("owner", "repo");
    ctx.comment().update().await;
    ctx.issue(123).get().await;

    let client = mock.client();
    let first_result = run_with_client_and_storage(
        &make_args(false, false, false),
        &client,
        &storage,
        "testuser",
    )
    .await
    .map_err(|error| error.to_string());
    let comment_after_first_push = std::fs::read_to_string(&comment_path).unwrap();
    setup_local_comment(
        test_dir.path(),
        "001_comment_12345.md",
        &comment_after_first_push.replacen("Updated", "Updated again", 1),
    );
    approve_file(&comment_path);
    approve_file(&test_dir.path().join("issue.md"));

    let second_result = run_with_client_and_storage(
        &make_args(false, false, false),
        &client,
        &storage,
        "testuser",
    )
    .await
    .map_err(|error| error.to_string());
    let comment_requests = comment_write_requests(&mock).await;

    assert_eq!(
        (first_result, second_result, comment_requests),
        (
            Ok(()),
            Ok(()),
            vec![
                (
                    "PATCH".to_string(),
                    Some(serde_json::json!({"body": "Updated"}))
                ),
                (
                    "PATCH".to_string(),
                    Some(serde_json::json!({"body": "Updated again"})),
                ),
            ],
        ),
    );
}

#[rstest]
#[tokio::test]
async fn test_remote_comment_posted_after_pull_is_not_deleted(test_dir: TempDir) {
    let (mock, storage) = TestSetup::new(test_dir.path())
        .remote_comments(vec![make_remote_comment(
            "otheruser",
            "IC_new",
            12346,
            "New",
        )])
        .build()
        .await;
    storage.save_base_comments(&[]).unwrap();

    let client = mock.client();
    let result = run_with_client_and_storage(
        &make_args_full(false, false, false, false),
        &client,
        &storage,
        "testuser",
    )
    .await;

    assert_eq!(
        result
            .map(|_| storage.read_base_comments().unwrap())
            .unwrap(),
        Some(vec![]),
    );
}

#[rstest]
#[tokio::test]
async fn test_remote_only_edit_does_not_require_edit_others(test_dir: TempDir) {
    let (mock, storage) = TestSetup::new(test_dir.path())
        .remote_comments(vec![make_remote_comment(
            "otheruser",
            "IC_abc",
            12345,
            "Remote edit",
        )])
        .build()
        .await;
    storage
        .save_base_comments(&[CommentSnapshot {
            id: "IC_abc".to_string(),
            body: "Original".to_string(),
        }])
        .unwrap();
    setup_local_comment(
        test_dir.path(),
        "001_comment_12345.md",
        &create_comment_file(
            "otheruser",
            "2024-01-01T12:00:00+00:00",
            "IC_abc",
            12345,
            "Original",
        ),
    );

    let client = mock.client();
    let result = run_with_client_and_storage(
        &make_args_full(false, false, false, false),
        &client,
        &storage,
        "testuser",
    )
    .await;

    assert_eq!(
        result
            .map(|_| storage.read_base_comments().unwrap())
            .unwrap(),
        Some(vec![CommentSnapshot {
            id: "IC_abc".to_string(),
            body: "Original".to_string(),
        }]),
    );
}

#[fixture]
async fn new_comment_setup(test_dir: TempDir) -> (TempDir, GitHubMockServer, IssueStorage) {
    let (mock, storage) = TestSetup::new(test_dir.path())
        .comments_after_initial_fetch(Some((
            vec![make_remote_comment(
                "testuser",
                "IC_new_comment",
                99999,
                "New comment",
            )],
            "2024-01-02T12:00:00Z",
        )))
        .build()
        .await;
    storage.save_base_comments(&[]).unwrap();
    setup_local_comment(test_dir.path(), "new_my_comment.md", "New comment");
    approve_file(&test_dir.path().join("comments/new_my_comment.md"));
    let ctx = mock.repo("owner", "repo");
    ctx.issue(123).create_comment().await;
    ctx.issue(123).get().await;
    (test_dir, mock, storage)
}

#[rstest]
#[tokio::test]
async fn test_creates_new_comment(
    #[future] new_comment_setup: (TempDir, GitHubMockServer, IssueStorage),
) {
    let (test_dir, mock, storage) = new_comment_setup.await;
    let client = mock.client();
    let result = run_with_client_and_storage(
        &make_args(false, false, false),
        &client,
        &storage,
        "testuser",
    )
    .await;
    let new_comment_path = test_dir.path().join("comments/001_comment_99999.md");
    let comment_after_push = std::fs::read_to_string(new_comment_path).unwrap();

    assert_eq!(
        (
            result.map_err(|error| error.to_string()),
            test_dir.path().join("comments/new_my_comment.md").exists(),
            comment_after_push,
        ),
        (
            Ok(()),
            false,
            formatdoc! {"
                <!-- author: testuser -->
                <!-- createdAt: 2024-01-01T12:00:00+00:00 -->
                <!-- updatedAt: 2024-01-02T12:00:00+00:00 -->
                <!-- id: IC_new_comment -->
                <!-- databaseId: 99999 -->

                New comment
            "
            },
        ),
    );
}

#[rstest]
#[tokio::test]
async fn test_does_not_delete_created_comment_on_second_push(
    #[future] new_comment_setup: (TempDir, GitHubMockServer, IssueStorage),
) {
    let (_test_dir, mock, storage) = new_comment_setup.await;
    let client = mock.client();
    let first_result = run_with_client_and_storage(
        &make_args(false, false, false),
        &client,
        &storage,
        "testuser",
    )
    .await
    .map_err(|error| error.to_string());
    let second_result = run_with_client_and_storage(
        &make_args(false, false, false),
        &client,
        &storage,
        "testuser",
    )
    .await
    .map_err(|error| error.to_string());
    let comment_requests = comment_write_requests(&mock).await;

    assert_eq!(
        (first_result, second_result, comment_requests),
        (
            Ok(()),
            Ok(()),
            vec![(
                "POST".to_string(),
                Some(serde_json::json!({"body": "New comment"})),
            )],
        ),
    );
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
#[case::edit_other_users_comment(Some("Modified"))]
#[case::delete_other_users_comment(None)]
#[tokio::test]
async fn test_dry_run_shows_other_users_comment_changes(
    test_dir: TempDir,
    #[case] local_body: Option<&str>,
) {
    let (mock, storage) = TestSetup::new(test_dir.path())
        .remote_comments(vec![make_remote_comment(
            "otheruser",
            "IC_abc",
            12345,
            "Original",
        )])
        .build()
        .await;
    storage
        .save_base_comments(&[CommentSnapshot {
            id: "IC_abc".to_string(),
            body: "Original".to_string(),
        }])
        .unwrap();
    if let Some(body) = local_body {
        setup_local_comment(
            test_dir.path(),
            "001_comment_12345.md",
            &create_comment_file(
                "otheruser",
                "2024-01-01T12:00:00+00:00",
                "IC_abc",
                12345,
                body,
            ),
        );
    }
    let local_comment_path = storage.dir().join("comments/001_comment_12345.md");
    let local_comment_before =
        std::fs::read_to_string(&local_comment_path).map_err(|error| error.kind());
    let snapshot_before = storage.read_base_comments().unwrap();

    let client = mock.client();
    let result = run_with_client_and_storage(
        &make_args_full(true, false, false, false),
        &client,
        &storage,
        "testuser",
    )
    .await;
    let local_comment_after =
        std::fs::read_to_string(&local_comment_path).map_err(|error| error.kind());
    let snapshot_after = storage.read_base_comments().unwrap();
    let github_writes = mock
        .received_requests()
        .await
        .into_iter()
        .filter(|request| matches!(request.method.as_str(), "PATCH" | "DELETE"))
        .map(|request| (request.method.to_string(), request.url.path().to_string()))
        .collect::<Vec<_>>();

    assert_eq!(
        (
            result.map_err(|error| error.to_string()),
            local_comment_after,
            snapshot_after,
            github_writes,
        ),
        (Ok(()), local_comment_before, snapshot_before, Vec::new()),
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
    approve_file(&test_dir.path().join("comments/001_comment_12345.md"));
    let ctx = mock.repo("owner", "repo");
    ctx.comment().update().await;
    ctx.issue(123).get().await;

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
    // Local title differs from remote, so we need issue update mock
    let ctx = mock.repo("owner", "repo");
    ctx.issue(123).update().await;
    // issue.get() is already called by TestSetup for initial fetch,
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

    // After push, issue.md should be updated with new frontmatter from mock response
    // Parse the frontmatter to verify it contains proper structure
    let issue_content = storage.read_issue().expect("should read issue");
    // Verify readonly metadata is present with expected fields
    assert!(!issue_content.frontmatter.readonly.updated_at.is_empty());
    assert_eq!(issue_content.frontmatter.readonly.number, 123);
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
    let ctx = mock.repo("owner", "repo");
    ctx.comment().delete().await;
    ctx.issue(123).get().await;

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

// Approval check tests
#[rstest]
#[case::fails_without_approval(false, false)]
#[case::dry_run_skips_approval_check(true, true)]
#[tokio::test]
async fn test_push_approval_check(
    test_dir: TempDir,
    #[case] dry_run: bool,
    #[case] should_succeed: bool,
) {
    let (mock, storage) = TestSetup::new(test_dir.path())
        .local_body("Changed body")
        .remote_body("Original body")
        .build()
        .await;

    // Drop the approval record for issue.md that TestSetup creates so the
    // approval check sees this document as unapproved.
    crate::shared::testing::init_approval_dir();
    let _ =
        crate::shared::human_in_the_loop::ApprovalManager::new(&test_dir.path().join("issue.md"))
            .remove();

    let client = mock.client();
    let result = run_with_client_and_storage(
        &make_args(dry_run, false, false),
        &client,
        &storage,
        "testuser",
    )
    .await;

    if should_succeed {
        assert!(
            result.is_ok(),
            "push should succeed but failed: {:?}",
            result.err()
        );
    } else {
        let err = result.expect_err("push should fail without approval");
        assert!(
            err.to_string().contains("not been approved"),
            "Expected approval error, got: {}",
            err
        );
    }
}

// Test run_with_client_and_storage rejects NewIssuePath
// Uses a directory without issue.md frontmatter so parse_target returns NewIssuePath
#[rstest]
#[tokio::test]
async fn test_rejects_new_issue_path() {
    let temp_dir = TempDir::new().unwrap();
    // Create a directory without issue.md (or with non-frontmatter issue.md)
    // so that parse_target returns NewIssuePath
    let new_dir = temp_dir.path().join("new");
    std::fs::create_dir_all(&new_dir).unwrap();

    let mock = crate::infra::github::GitHubMockServer::start().await;
    let storage =
        crate::commands::gh::issue_agent::storage::IssueStorage::from_dir(temp_dir.path());
    let args = PushArgs {
        target: new_dir.to_string_lossy().to_string(),
        repo: Some("owner/repo".to_string()),
        dry_run: false,
        force: false,
        edit_others: false,
        allow_delete: false,
    };

    let client = mock.client();
    let result = run_with_client_and_storage(&args, &client, &storage, "testuser").await;

    let err = result.expect_err("should fail for new issue path");
    assert_eq!(
        err.to_string(),
        "run_with_client_and_storage does not support new issue creation"
    );
}
