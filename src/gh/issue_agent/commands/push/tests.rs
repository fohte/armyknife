//! Integration tests for push command.

use super::detect::{check_can_edit_comment, check_remote_unchanged, detect_label_change};
use super::diff::format_diff;
use super::*;
use crate::gh::issue_agent::commands::IssueArgs;
use crate::gh::issue_agent::commands::test_helpers::{
    OLD_TS, TestSetup, create_comment_file, make_comment, setup_local_comment, test_dir,
};
use rstest::rstest;
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
        assert!(result.unwrap_err().contains("Remote has changed"));
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
                .to_string()
                .contains("Cannot edit other user's comment")
        );
    }
}

mod detect_label_change_tests {
    use super::*;
    use crate::gh::issue_agent::models::{Issue, IssueMetadata};
    use crate::testing::factories;

    fn metadata_with_labels(labels: &[&str]) -> IssueMetadata {
        IssueMetadata {
            number: 1,
            title: "Test".to_string(),
            state: "OPEN".to_string(),
            labels: labels.iter().map(|s| s.to_string()).collect(),
            assignees: vec![],
            milestone: None,
            author: "testuser".to_string(),
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-02T00:00:00Z".to_string(),
        }
    }

    fn issue_with_labels(labels: &[&str]) -> Issue {
        factories::issue_with(|i| {
            i.labels = factories::labels(labels);
        })
    }

    #[rstest]
    #[case::no_changes(vec!["bug"], vec!["bug"])]
    #[case::both_empty(vec![], vec![])]
    fn test_returns_none_when_no_changes(#[case] local: Vec<&str>, #[case] remote: Vec<&str>) {
        let metadata = metadata_with_labels(&local);
        let issue = issue_with_labels(&remote);
        assert!(detect_label_change(&metadata, &issue).is_none());
    }

    #[rstest]
    #[case::add_one(vec!["bug", "new"], vec!["bug"], vec![], vec!["new"])]
    #[case::remove_one(vec!["bug"], vec!["bug", "old"], vec!["old"], vec![])]
    #[case::add_and_remove(vec!["new"], vec!["old"], vec!["old"], vec!["new"])]
    #[case::empty_local(vec![], vec!["a", "b"], vec!["a", "b"], vec![])]
    #[case::empty_remote(vec!["a", "b"], vec![], vec![], vec!["a", "b"])]
    fn test_detects_label_changes(
        #[case] local: Vec<&str>,
        #[case] remote: Vec<&str>,
        #[case] expected_remove: Vec<&str>,
        #[case] expected_add: Vec<&str>,
    ) {
        let metadata = metadata_with_labels(&local);
        let issue = issue_with_labels(&remote);

        let change = detect_label_change(&metadata, &issue).expect("Expected label changes");

        let mut to_remove = change.to_remove.clone();
        to_remove.sort();
        let mut to_add = change.to_add.clone();
        to_add.sort();
        let mut expected_remove: Vec<String> =
            expected_remove.iter().map(|s| s.to_string()).collect();
        let mut expected_add: Vec<String> = expected_add.iter().map(|s| s.to_string()).collect();
        expected_remove.sort();
        expected_add.sort();

        assert_eq!(to_remove, expected_remove);
        assert_eq!(to_add, expected_add);
    }
}

mod run_with_client_and_storage_tests {
    use super::*;

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
}

mod format_diff_tests {
    use super::*;

    #[rstest]
    #[case::no_changes("a\n", "a\n", vec![" a"])]
    #[case::add_line("a\n", "a\nb\n", vec![" a", "+b"])]
    #[case::delete_line("a\nb\n", "a\n", vec![" a", "-b"])]
    #[case::modify("old\n", "new\n", vec!["-old", "+new"])]
    #[case::modify_middle("a\nold\nc\n", "a\nnew\nc\n", vec![" a", "-old", "+new", " c"])]
    #[case::empty_both("", "", vec![])]
    fn test_format_diff(#[case] old: &str, #[case] new: &str, #[case] expected: Vec<&str>) {
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
