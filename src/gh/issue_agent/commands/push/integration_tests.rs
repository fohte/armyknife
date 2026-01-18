//! Integration tests for push command.

use super::*;
use crate::gh::issue_agent::commands::IssueArgs;
use crate::gh::issue_agent::commands::test_helpers::{
    OLD_TS, TestSetup, create_comment_file, make_comment, setup_local_comment, test_dir,
};
use rstest::rstest;
use std::fs;
use tempfile::TempDir;

fn make_args(dry_run: bool, force: bool, edit_others: bool) -> PushArgs {
    PushArgs {
        issue: IssueArgs {
            issue_number: 123,
            repo: Some("owner/repo".to_string()),
        },
        dry_run,
        force,
        edit_others,
    }
}

// Body update tests
#[rstest]
#[case::dry_run_no_api_call("Local", "Remote", true, 0)]
#[case::updates_body("Local", "Remote", false, 1)]
#[case::no_changes("Same", "Same", false, 0)]
#[tokio::test]
async fn test_body_update(
    test_dir: TempDir,
    #[case] local_body: &str,
    #[case] remote_body: &str,
    #[case] dry_run: bool,
    #[case] expected: usize,
) {
    let (client, storage) = TestSetup::new(test_dir.path())
        .local_body(local_body)
        .remote_body(remote_body)
        .build();

    let result = run_with_client_and_storage(
        &make_args(dry_run, false, false),
        &client,
        &storage,
        "testuser",
    )
    .await;
    assert!(result.is_ok());
    assert_eq!(client.updated_issue_bodies.lock().unwrap().len(), expected);
}

#[rstest]
#[tokio::test]
async fn test_updates_title(test_dir: TempDir) {
    let (client, storage) = TestSetup::new(test_dir.path())
        .remote_title("Old Title")
        .local_title("New Title")
        .build();

    let result = run_with_client_and_storage(
        &make_args(false, false, false),
        &client,
        &storage,
        "testuser",
    )
    .await;
    assert!(result.is_ok());
    assert_eq!(
        client.updated_issue_titles.lock().unwrap()[0].title,
        "New Title"
    );
}

#[rstest]
#[tokio::test]
async fn test_updates_labels(test_dir: TempDir) {
    let (client, storage) = TestSetup::new(test_dir.path())
        .local_labels(vec!["enhancement"])
        .build();

    let result = run_with_client_and_storage(
        &make_args(false, false, false),
        &client,
        &storage,
        "testuser",
    )
    .await;
    assert!(result.is_ok());
    assert_eq!(client.removed_labels.lock().unwrap()[0].label, "bug");
    assert_eq!(
        client.added_labels.lock().unwrap()[0].labels,
        vec!["enhancement"]
    );
}

// Remote changed detection
#[rstest]
#[case::fails_when_remote_changed(false, true)]
#[case::force_overrides(true, false)]
#[tokio::test]
async fn test_remote_changed(test_dir: TempDir, #[case] force: bool, #[case] expect_err: bool) {
    let (client, storage) = TestSetup::new(test_dir.path()).local_ts(OLD_TS).build();

    let result = run_with_client_and_storage(
        &make_args(false, force, false),
        &client,
        &storage,
        "testuser",
    )
    .await;
    if expect_err {
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Remote has changed")
        );
    } else {
        assert!(result.is_ok());
    }
}

#[rstest]
#[tokio::test]
async fn test_invalid_repo_format(test_dir: TempDir) {
    let (client, storage) = TestSetup::new(test_dir.path()).build();
    let args = PushArgs {
        issue: IssueArgs {
            issue_number: 123,
            repo: Some("invalid-format".to_string()),
        },
        dry_run: false,
        force: false,
        edit_others: false,
    };

    let result = run_with_client_and_storage(&args, &client, &storage, "testuser").await;
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("Invalid repository format")
    );
}

// Comment operations
#[rstest]
#[tokio::test]
async fn test_updates_own_comment(test_dir: TempDir) {
    let (client, storage) = TestSetup::new(test_dir.path())
        .remote_comments(vec![make_comment("testuser")])
        .build();
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

    let result = run_with_client_and_storage(
        &make_args(false, false, false),
        &client,
        &storage,
        "testuser",
    )
    .await;
    assert!(result.is_ok());
    assert_eq!(client.updated_comments.lock().unwrap()[0].body, "Updated");
}

#[rstest]
#[tokio::test]
async fn test_creates_new_comment(test_dir: TempDir) {
    let (client, storage) = TestSetup::new(test_dir.path()).build();
    setup_local_comment(test_dir.path(), "new_my_comment.md", "New comment");

    let result = run_with_client_and_storage(
        &make_args(false, false, false),
        &client,
        &storage,
        "testuser",
    )
    .await;
    assert!(result.is_ok());
    assert_eq!(
        client.created_comments.lock().unwrap()[0].body,
        "New comment"
    );
    assert!(!test_dir.path().join("comments/new_my_comment.md").exists());
}

#[rstest]
#[case::denied_without_flag(false, true)]
#[case::allowed_with_flag(true, false)]
#[tokio::test]
async fn test_edit_others_comment(
    test_dir: TempDir,
    #[case] edit_others: bool,
    #[case] expect_err: bool,
) {
    let (client, storage) = TestSetup::new(test_dir.path())
        .remote_comments(vec![make_comment("otheruser")])
        .build();
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

    let result = run_with_client_and_storage(
        &make_args(false, false, edit_others),
        &client,
        &storage,
        "testuser",
    )
    .await;
    if expect_err {
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Cannot edit other user's comment")
        );
    } else {
        assert!(result.is_ok());
        assert_eq!(client.updated_comments.lock().unwrap()[0].body, "Modified");
    }
}

#[rstest]
#[tokio::test]
async fn test_updates_metadata_after_push(test_dir: TempDir) {
    let new_ts = "2024-01-03T00:00:00+00:00";
    let (client, storage) = TestSetup::new(test_dir.path())
        .remote_ts(new_ts)
        .local_ts(OLD_TS)
        .local_title("Old Title")
        .remote_title("New Title")
        .build();

    let result = run_with_client_and_storage(
        &make_args(false, true, false),
        &client,
        &storage,
        "testuser",
    )
    .await;
    assert!(result.is_ok());
    assert!(
        fs::read_to_string(test_dir.path().join("metadata.json"))
            .unwrap()
            .contains("2024-01-03")
    );
}
