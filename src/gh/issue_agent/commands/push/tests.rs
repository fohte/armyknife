//! Integration tests for push command.

use super::*;
use crate::gh::issue_agent::commands::IssueArgs;
use crate::gh::issue_agent::commands::test_helpers::{
    create_comment_file, create_metadata_json, create_test_issue, test_dir,
};
use crate::gh::issue_agent::models::{Author, Comment};
use crate::github::MockGitHubClient;
use chrono::{TimeZone, Utc};
use rstest::rstest;
use std::collections::HashSet;
use std::fs;
use tempfile::TempDir;

mod check_remote_unchanged_tests {
    use super::*;

    #[rstest]
    #[case::same_timestamp("2024-01-01T00:00:00Z", "2024-01-01T00:00:00Z", false)]
    #[case::force_with_different("2024-01-01T00:00:00Z", "2024-01-02T00:00:00Z", true)]
    #[case::force_with_same("2024-01-01T00:00:00Z", "2024-01-01T00:00:00Z", true)]
    fn test_ok(#[case] local: &str, #[case] remote: &str, #[case] force: bool) {
        assert!(check_remote_unchanged(local, remote, force).is_ok());
    }

    #[rstest]
    #[case::different_timestamp("2024-01-01T00:00:00Z", "2024-01-02T00:00:00Z", false)]
    #[case::local_newer("2024-01-02T00:00:00Z", "2024-01-01T00:00:00Z", false)]
    fn test_err(#[case] local: &str, #[case] remote: &str, #[case] force: bool) {
        let result = check_remote_unchanged(local, remote, force);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("Remote has changed"));
    }
}

mod check_can_edit_comment_tests {
    use super::*;

    #[rstest]
    #[case::own_comment("alice", "alice", false)]
    #[case::other_with_flag("bob", "alice", true)]
    #[case::own_with_flag("alice", "alice", true)]
    fn test_allowed(#[case] author: &str, #[case] current_user: &str, #[case] edit_others: bool) {
        assert!(check_can_edit_comment(author, current_user, edit_others, "001.md").is_ok());
    }

    #[rstest]
    #[case::other_without_flag("bob", "alice", false)]
    #[case::unknown_author("unknown", "alice", false)]
    fn test_denied(#[case] author: &str, #[case] current_user: &str, #[case] edit_others: bool) {
        let result = check_can_edit_comment(author, current_user, edit_others, "001.md");
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .contains("Cannot edit other user's comment")
        );
    }
}

mod compute_label_changes_tests {
    use super::*;

    #[rstest]
    #[case::no_changes(vec!["bug"], vec!["bug"], vec![], vec![])]
    #[case::add_one(vec!["bug", "new"], vec!["bug"], vec![], vec!["new"])]
    #[case::remove_one(vec!["bug"], vec!["bug", "old"], vec!["old"], vec![])]
    #[case::add_and_remove(vec!["new"], vec!["old"], vec!["old"], vec!["new"])]
    #[case::empty_local(vec![], vec!["a", "b"], vec!["a", "b"], vec![])]
    #[case::empty_remote(vec!["a", "b"], vec![], vec![], vec!["a", "b"])]
    #[case::both_empty(vec![], vec![], vec![], vec![])]
    fn test_label_changes(
        #[case] local: Vec<&str>,
        #[case] remote: Vec<&str>,
        #[case] expected_remove: Vec<&str>,
        #[case] expected_add: Vec<&str>,
    ) {
        let local: HashSet<&str> = local.into_iter().collect();
        let remote: HashSet<&str> = remote.into_iter().collect();
        let (mut to_remove, mut to_add) = compute_label_changes(&local, &remote);
        to_remove.sort();
        to_add.sort();
        let mut expected_remove = expected_remove;
        let mut expected_add = expected_add;
        expected_remove.sort();
        expected_add.sort();
        assert_eq!(to_remove, expected_remove);
        assert_eq!(to_add, expected_add);
    }
}

mod run_with_client_and_storage_tests {
    use super::*;

    const TIMESTAMP: &str = "2024-01-02T00:00:00+00:00";
    const OLD_TIMESTAMP: &str = "2024-01-01T00:00:00+00:00";

    fn setup_storage(dir: &std::path::Path, body: &str, metadata_json: &str) {
        fs::create_dir_all(dir).unwrap();
        fs::write(dir.join("issue.md"), format!("{}\n", body)).unwrap();
        fs::write(dir.join("metadata.json"), metadata_json).unwrap();
    }

    fn setup_comment(dir: &std::path::Path, filename: &str, content: &str) {
        let comments_dir = dir.join("comments");
        fs::create_dir_all(&comments_dir).unwrap();
        fs::write(comments_dir.join(filename), content).unwrap();
    }

    fn make_args(issue_number: u64, dry_run: bool, force: bool, edit_others: bool) -> PushArgs {
        PushArgs {
            issue: IssueArgs {
                issue_number,
                repo: Some("owner/repo".to_string()),
            },
            dry_run,
            force,
            edit_others,
        }
    }

    fn make_comment(id: &str, db_id: i64, author: &str, body: &str) -> Comment {
        Comment {
            id: id.to_string(),
            database_id: db_id,
            author: Some(Author {
                login: author.to_string(),
            }),
            created_at: Utc.with_ymd_and_hms(2024, 1, 1, 12, 0, 0).unwrap(),
            body: body.to_string(),
        }
    }

    // Basic push operations: body, title, labels
    #[rstest]
    #[case::dry_run_no_api_call("Local", "Remote", true, false, 0)]
    #[case::updates_body("Local", "Remote", false, false, 1)]
    #[case::no_changes("Same", "Same", false, false, 0)]
    #[tokio::test]
    async fn test_body_update(
        test_dir: TempDir,
        #[case] local_body: &str,
        #[case] remote_body: &str,
        #[case] dry_run: bool,
        #[case] force: bool,
        #[case] expected_updates: usize,
    ) {
        let issue = create_test_issue(123, "Title", remote_body, TIMESTAMP);
        let client = MockGitHubClient::new()
            .with_issue("owner", "repo", issue)
            .with_comments("owner", "repo", 123, vec![]);

        setup_storage(
            test_dir.path(),
            local_body,
            &create_metadata_json(123, "Title", TIMESTAMP, &["bug"]),
        );

        let storage = IssueStorage::from_dir(test_dir.path());
        let args = make_args(123, dry_run, force, false);

        let result = run_with_client_and_storage(&args, &client, &storage, "testuser").await;
        assert!(result.is_ok());
        assert_eq!(
            client.updated_issue_bodies.lock().unwrap().len(),
            expected_updates
        );
    }

    #[rstest]
    #[tokio::test]
    async fn test_updates_title(test_dir: TempDir) {
        let issue = create_test_issue(123, "Old Title", "Body", TIMESTAMP);
        let client = MockGitHubClient::new()
            .with_issue("owner", "repo", issue)
            .with_comments("owner", "repo", 123, vec![]);

        setup_storage(
            test_dir.path(),
            "Body",
            &create_metadata_json(123, "New Title", TIMESTAMP, &["bug"]),
        );

        let storage = IssueStorage::from_dir(test_dir.path());
        let result = run_with_client_and_storage(
            &make_args(123, false, false, false),
            &client,
            &storage,
            "testuser",
        )
        .await;

        assert!(result.is_ok());
        let updated = client.updated_issue_titles.lock().unwrap();
        assert_eq!(updated.len(), 1);
        assert_eq!(updated[0].title, "New Title");
    }

    #[rstest]
    #[tokio::test]
    async fn test_updates_labels(test_dir: TempDir) {
        let issue = create_test_issue(123, "Title", "Body", TIMESTAMP);
        let client = MockGitHubClient::new()
            .with_issue("owner", "repo", issue)
            .with_comments("owner", "repo", 123, vec![]);

        setup_storage(
            test_dir.path(),
            "Body",
            &create_metadata_json(123, "Title", TIMESTAMP, &["enhancement"]),
        );

        let storage = IssueStorage::from_dir(test_dir.path());
        let result = run_with_client_and_storage(
            &make_args(123, false, false, false),
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
        let issue = create_test_issue(123, "Title", "Remote", TIMESTAMP);
        let client = MockGitHubClient::new()
            .with_issue("owner", "repo", issue)
            .with_comments("owner", "repo", 123, vec![]);

        setup_storage(
            test_dir.path(),
            "Local",
            &create_metadata_json(123, "Title", OLD_TIMESTAMP, &["bug"]),
        );

        let storage = IssueStorage::from_dir(test_dir.path());
        let result = run_with_client_and_storage(
            &make_args(123, false, force, false),
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
        let client = MockGitHubClient::new();
        setup_storage(
            test_dir.path(),
            "Body",
            &create_metadata_json(123, "T", TIMESTAMP, &[]),
        );

        let storage = IssueStorage::from_dir(test_dir.path());
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
        let issue = create_test_issue(123, "Title", "Body", TIMESTAMP);
        let comment = make_comment("IC_abc", 12345, "testuser", "Original");
        let client = MockGitHubClient::new()
            .with_issue("owner", "repo", issue)
            .with_comments("owner", "repo", 123, vec![comment])
            .with_current_user("testuser");

        setup_storage(
            test_dir.path(),
            "Body",
            &create_metadata_json(123, "Title", TIMESTAMP, &["bug"]),
        );
        setup_comment(
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

        let storage = IssueStorage::from_dir(test_dir.path());
        let result = run_with_client_and_storage(
            &make_args(123, false, false, false),
            &client,
            &storage,
            "testuser",
        )
        .await;

        assert!(result.is_ok());
        let updated = client.updated_comments.lock().unwrap();
        assert_eq!(updated.len(), 1);
        assert_eq!(updated[0].body, "Updated");
    }

    #[rstest]
    #[tokio::test]
    async fn test_creates_new_comment(test_dir: TempDir) {
        let issue = create_test_issue(123, "Title", "Body", TIMESTAMP);
        let client = MockGitHubClient::new()
            .with_issue("owner", "repo", issue)
            .with_comments("owner", "repo", 123, vec![])
            .with_current_user("testuser");

        setup_storage(
            test_dir.path(),
            "Body",
            &create_metadata_json(123, "Title", TIMESTAMP, &["bug"]),
        );
        setup_comment(test_dir.path(), "new_my_comment.md", "New comment");

        let storage = IssueStorage::from_dir(test_dir.path());
        let result = run_with_client_and_storage(
            &make_args(123, false, false, false),
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

    // Editing others' comments
    #[rstest]
    #[case::denied_without_flag(false, true)]
    #[case::allowed_with_flag(true, false)]
    #[tokio::test]
    async fn test_edit_others_comment(
        test_dir: TempDir,
        #[case] edit_others: bool,
        #[case] expect_err: bool,
    ) {
        let issue = create_test_issue(123, "Title", "Body", TIMESTAMP);
        let comment = make_comment("IC_abc", 12345, "otheruser", "Original");
        let client = MockGitHubClient::new()
            .with_issue("owner", "repo", issue)
            .with_comments("owner", "repo", 123, vec![comment])
            .with_current_user("testuser");

        setup_storage(
            test_dir.path(),
            "Body",
            &create_metadata_json(123, "Title", TIMESTAMP, &["bug"]),
        );
        setup_comment(
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

        let storage = IssueStorage::from_dir(test_dir.path());
        let result = run_with_client_and_storage(
            &make_args(123, false, false, edit_others),
            &client,
            &storage,
            "testuser",
        )
        .await;

        if expect_err {
            let err = result.unwrap_err().to_string();
            assert!(err.contains("Cannot edit other user's comment"));
        } else {
            assert!(result.is_ok());
            assert_eq!(client.updated_comments.lock().unwrap()[0].body, "Modified");
        }
    }

    #[rstest]
    #[tokio::test]
    async fn test_updates_metadata_after_push(test_dir: TempDir) {
        let new_timestamp = "2024-01-03T00:00:00+00:00";
        let mut issue = create_test_issue(123, "New Title", "Body", new_timestamp);
        issue.body = Some("Body".to_string());

        let client = MockGitHubClient::new()
            .with_issue("owner", "repo", issue)
            .with_comments("owner", "repo", 123, vec![]);

        setup_storage(
            test_dir.path(),
            "Body",
            &create_metadata_json(123, "Old Title", OLD_TIMESTAMP, &["bug"]),
        );

        let storage = IssueStorage::from_dir(test_dir.path());
        let result = run_with_client_and_storage(
            &make_args(123, false, true, false),
            &client,
            &storage,
            "testuser",
        )
        .await;

        assert!(result.is_ok());
        let metadata = fs::read_to_string(test_dir.path().join("metadata.json")).unwrap();
        assert!(metadata.contains("2024-01-03"));
    }
}

mod format_diff_tests {
    use super::*;

    #[rstest]
    #[case::no_changes("a\n", "a\n", vec![" a"], vec![])]
    #[case::add_line("a\n", "a\nb\n", vec![" a", "+b"], vec![])]
    #[case::delete_line("a\nb\n", "a\n", vec![" a", "-b"], vec![])]
    #[case::modify("old\n", "new\n", vec!["-old", "+new"], vec![])]
    #[case::modify_middle("a\nold\nc\n", "a\nnew\nc\n", vec![" a", "-old", "+new", " c"], vec![])]
    #[case::empty_both("", "", vec![], vec![])]
    fn test_format_diff(
        #[case] old: &str,
        #[case] new: &str,
        #[case] expected: Vec<&str>,
        #[case] _unused: Vec<&str>,
    ) {
        let diff = format_diff(old, new);
        for line in expected {
            assert!(
                diff.contains(line),
                "Expected '{}' in diff:\n{}",
                line,
                diff
            );
        }
    }
}
