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
        assert!(err.contains(local));
        assert!(err.contains(remote));
    }
}

mod check_can_edit_comment_tests {
    use super::*;

    #[rstest]
    #[case::own_comment("alice", "alice", false, "001_comment.md")]
    #[case::other_comment_with_edit_others("bob", "alice", true, "001_comment.md")]
    #[case::own_comment_with_edit_others("alice", "alice", true, "001_comment.md")]
    fn test_allowed(
        #[case] author: &str,
        #[case] current_user: &str,
        #[case] edit_others: bool,
        #[case] filename: &str,
    ) {
        assert!(check_can_edit_comment(author, current_user, edit_others, filename).is_ok());
    }

    #[rstest]
    #[case::other_comment_without_flag("bob", "alice", false, "001_comment.md")]
    #[case::unknown_author("unknown", "alice", false, "002_comment.md")]
    fn test_denied(
        #[case] author: &str,
        #[case] current_user: &str,
        #[case] edit_others: bool,
        #[case] filename: &str,
    ) {
        let result = check_can_edit_comment(author, current_user, edit_others, filename);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("Cannot edit other user's comment"));
        assert!(err.contains(filename));
        assert!(err.contains(author));
        assert!(err.contains("--edit-others"));
    }
}

mod compute_label_changes_tests {
    use super::*;

    #[rstest]
    #[case::no_changes(
        vec!["bug", "feature"],
        vec!["bug", "feature"],
        vec![],
        vec![]
    )]
    #[case::add_one_label(
        vec!["bug", "feature", "new-label"],
        vec!["bug", "feature"],
        vec![],
        vec!["new-label"]
    )]
    #[case::remove_one_label(
        vec!["bug"],
        vec!["bug", "feature"],
        vec!["feature"],
        vec![]
    )]
    #[case::add_and_remove(
        vec!["bug", "new-label"],
        vec!["bug", "old-label"],
        vec!["old-label"],
        vec!["new-label"]
    )]
    #[case::empty_local(
        vec![],
        vec!["bug", "feature"],
        vec!["bug", "feature"],
        vec![]
    )]
    #[case::empty_remote(
        vec!["bug", "feature"],
        vec![],
        vec![],
        vec!["bug", "feature"]
    )]
    #[case::both_empty(
        vec![],
        vec![],
        vec![],
        vec![]
    )]
    fn test_label_changes(
        #[case] local_labels: Vec<&str>,
        #[case] remote_labels: Vec<&str>,
        #[case] expected_remove: Vec<&str>,
        #[case] expected_add: Vec<&str>,
    ) {
        let local: HashSet<&str> = local_labels.into_iter().collect();
        let remote: HashSet<&str> = remote_labels.into_iter().collect();
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

    /// Helper to set up local storage with issue body and metadata.
    fn setup_storage(dir: &std::path::Path, body: &str, metadata_json: &str) {
        fs::create_dir_all(dir).unwrap();
        fs::write(dir.join("issue.md"), format!("{}\n", body)).unwrap();
        fs::write(dir.join("metadata.json"), metadata_json).unwrap();
    }

    /// Helper to set up a comment file in storage.
    fn setup_comment(dir: &std::path::Path, filename: &str, content: &str) {
        let comments_dir = dir.join("comments");
        fs::create_dir_all(&comments_dir).unwrap();
        fs::write(comments_dir.join(filename), content).unwrap();
    }

    #[rstest]
    #[tokio::test]
    async fn test_dry_run_detects_body_change(test_dir: TempDir) {
        let updated_at = "2024-01-02T00:00:00+00:00";
        let issue = create_test_issue(123, "Test Issue", "Remote body", updated_at);
        let client = MockGitHubClient::new()
            .with_issue("owner", "repo", issue)
            .with_comments("owner", "repo", 123, vec![]);

        setup_storage(
            test_dir.path(),
            "Local body",
            &create_metadata_json(123, "Test Issue", updated_at, &["bug"]),
        );

        let storage = IssueStorage::from_dir(test_dir.path());
        let args = PushArgs {
            issue: IssueArgs {
                issue_number: 123,
                repo: Some("owner/repo".to_string()),
            },
            dry_run: true,
            force: false,
            edit_others: false,
        };

        let result = run_with_client_and_storage(&args, &client, &storage, "testuser").await;
        assert!(result.is_ok());

        // In dry-run mode, no API calls should be made
        assert!(client.updated_issue_bodies.lock().unwrap().is_empty());
    }

    #[rstest]
    #[tokio::test]
    async fn test_updates_issue_body(test_dir: TempDir) {
        let updated_at = "2024-01-02T00:00:00+00:00";
        let issue = create_test_issue(123, "Test Issue", "Remote body", updated_at);
        let client = MockGitHubClient::new()
            .with_issue("owner", "repo", issue)
            .with_comments("owner", "repo", 123, vec![]);

        setup_storage(
            test_dir.path(),
            "Local body",
            &create_metadata_json(123, "Test Issue", updated_at, &["bug"]),
        );

        let storage = IssueStorage::from_dir(test_dir.path());
        let args = PushArgs {
            issue: IssueArgs {
                issue_number: 123,
                repo: Some("owner/repo".to_string()),
            },
            dry_run: false,
            force: false,
            edit_others: false,
        };

        let result = run_with_client_and_storage(&args, &client, &storage, "testuser").await;
        assert!(result.is_ok());

        // Verify API was called
        let updated = client.updated_issue_bodies.lock().unwrap();
        assert_eq!(updated.len(), 1);
        assert_eq!(updated[0].body, "Local body");
    }

    #[rstest]
    #[tokio::test]
    async fn test_updates_title(test_dir: TempDir) {
        let updated_at = "2024-01-02T00:00:00+00:00";
        let issue = create_test_issue(123, "Old Title", "Body", updated_at);
        let client = MockGitHubClient::new()
            .with_issue("owner", "repo", issue)
            .with_comments("owner", "repo", 123, vec![]);

        setup_storage(
            test_dir.path(),
            "Body",
            &create_metadata_json(123, "New Title", updated_at, &["bug"]),
        );

        let storage = IssueStorage::from_dir(test_dir.path());
        let args = PushArgs {
            issue: IssueArgs {
                issue_number: 123,
                repo: Some("owner/repo".to_string()),
            },
            dry_run: false,
            force: false,
            edit_others: false,
        };

        let result = run_with_client_and_storage(&args, &client, &storage, "testuser").await;
        assert!(result.is_ok());

        let updated = client.updated_issue_titles.lock().unwrap();
        assert_eq!(updated.len(), 1);
        assert_eq!(updated[0].title, "New Title");
    }

    #[rstest]
    #[tokio::test]
    async fn test_updates_labels(test_dir: TempDir) {
        let updated_at = "2024-01-02T00:00:00+00:00";
        let issue = create_test_issue(123, "Test Issue", "Body", updated_at);
        let client = MockGitHubClient::new()
            .with_issue("owner", "repo", issue)
            .with_comments("owner", "repo", 123, vec![]);

        setup_storage(
            test_dir.path(),
            "Body",
            &create_metadata_json(123, "Test Issue", updated_at, &["enhancement"]),
        );

        let storage = IssueStorage::from_dir(test_dir.path());
        let args = PushArgs {
            issue: IssueArgs {
                issue_number: 123,
                repo: Some("owner/repo".to_string()),
            },
            dry_run: false,
            force: false,
            edit_others: false,
        };

        let result = run_with_client_and_storage(&args, &client, &storage, "testuser").await;
        assert!(result.is_ok());

        // "bug" should be removed, "enhancement" should be added
        let removed = client.removed_labels.lock().unwrap();
        assert_eq!(removed.len(), 1);
        assert_eq!(removed[0].label, "bug");

        let added = client.added_labels.lock().unwrap();
        assert_eq!(added.len(), 1);
        assert_eq!(added[0].labels, vec!["enhancement"]);
    }

    #[rstest]
    #[tokio::test]
    async fn test_fails_when_remote_changed(test_dir: TempDir) {
        let local_updated_at = "2024-01-01T00:00:00+00:00";
        let remote_updated_at = "2024-01-02T00:00:00+00:00";
        let issue = create_test_issue(123, "Test Issue", "Body", remote_updated_at);
        let client = MockGitHubClient::new()
            .with_issue("owner", "repo", issue)
            .with_comments("owner", "repo", 123, vec![]);

        setup_storage(
            test_dir.path(),
            "Body",
            &create_metadata_json(123, "Test Issue", local_updated_at, &["bug"]),
        );

        let storage = IssueStorage::from_dir(test_dir.path());
        let args = PushArgs {
            issue: IssueArgs {
                issue_number: 123,
                repo: Some("owner/repo".to_string()),
            },
            dry_run: false,
            force: false,
            edit_others: false,
        };

        let result = run_with_client_and_storage(&args, &client, &storage, "testuser").await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Remote has changed")
        );
    }

    #[rstest]
    #[tokio::test]
    async fn test_force_overrides_remote_changed(test_dir: TempDir) {
        let local_updated_at = "2024-01-01T00:00:00+00:00";
        let remote_updated_at = "2024-01-02T00:00:00+00:00";
        let issue = create_test_issue(123, "Test Issue", "Remote body", remote_updated_at);
        let client = MockGitHubClient::new()
            .with_issue("owner", "repo", issue)
            .with_comments("owner", "repo", 123, vec![]);

        setup_storage(
            test_dir.path(),
            "Local body",
            &create_metadata_json(123, "Test Issue", local_updated_at, &["bug"]),
        );

        let storage = IssueStorage::from_dir(test_dir.path());
        let args = PushArgs {
            issue: IssueArgs {
                issue_number: 123,
                repo: Some("owner/repo".to_string()),
            },
            dry_run: false,
            force: true, // Force flag
            edit_others: false,
        };

        let result = run_with_client_and_storage(&args, &client, &storage, "testuser").await;
        assert!(result.is_ok());

        // Body should be updated despite timestamp mismatch
        let updated = client.updated_issue_bodies.lock().unwrap();
        assert_eq!(updated.len(), 1);
    }

    #[rstest]
    #[tokio::test]
    async fn test_no_changes_to_push(test_dir: TempDir) {
        let updated_at = "2024-01-02T00:00:00+00:00";
        let issue = create_test_issue(123, "Test Issue", "Same body", updated_at);
        let client = MockGitHubClient::new()
            .with_issue("owner", "repo", issue)
            .with_comments("owner", "repo", 123, vec![]);

        setup_storage(
            test_dir.path(),
            "Same body",
            &create_metadata_json(123, "Test Issue", updated_at, &["bug"]),
        );

        let storage = IssueStorage::from_dir(test_dir.path());
        let args = PushArgs {
            issue: IssueArgs {
                issue_number: 123,
                repo: Some("owner/repo".to_string()),
            },
            dry_run: false,
            force: false,
            edit_others: false,
        };

        let result = run_with_client_and_storage(&args, &client, &storage, "testuser").await;
        assert!(result.is_ok());

        // No API calls should be made
        assert!(client.updated_issue_bodies.lock().unwrap().is_empty());
        assert!(client.updated_issue_titles.lock().unwrap().is_empty());
        assert!(client.added_labels.lock().unwrap().is_empty());
        assert!(client.removed_labels.lock().unwrap().is_empty());
    }

    #[rstest]
    #[tokio::test]
    async fn test_fails_with_invalid_repo_format(test_dir: TempDir) {
        let client = MockGitHubClient::new();

        setup_storage(
            test_dir.path(),
            "Body",
            &create_metadata_json(123, "Test", "2024-01-01T00:00:00+00:00", &[]),
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
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Invalid repository format")
        );
    }

    #[rstest]
    #[tokio::test]
    async fn test_updates_own_comment(test_dir: TempDir) {
        let updated_at = "2024-01-02T00:00:00+00:00";
        let issue = create_test_issue(123, "Test Issue", "Body", updated_at);
        let remote_comment = Comment {
            id: "IC_abc123".to_string(),
            database_id: 12345,
            author: Some(Author {
                login: "testuser".to_string(),
            }),
            created_at: Utc.with_ymd_and_hms(2024, 1, 1, 12, 0, 0).unwrap(),
            body: "Original comment".to_string(),
        };
        let client = MockGitHubClient::new()
            .with_issue("owner", "repo", issue)
            .with_comments("owner", "repo", 123, vec![remote_comment])
            .with_current_user("testuser");

        setup_storage(
            test_dir.path(),
            "Body",
            &create_metadata_json(123, "Test Issue", updated_at, &["bug"]),
        );
        setup_comment(
            test_dir.path(),
            "001_comment_12345.md",
            &create_comment_file(
                "testuser",
                "2024-01-01T12:00:00+00:00",
                "IC_abc123",
                12345,
                "Updated comment body",
            ),
        );

        let storage = IssueStorage::from_dir(test_dir.path());
        let args = PushArgs {
            issue: IssueArgs {
                issue_number: 123,
                repo: Some("owner/repo".to_string()),
            },
            dry_run: false,
            force: false,
            edit_others: false,
        };

        let result = run_with_client_and_storage(&args, &client, &storage, "testuser").await;
        assert!(result.is_ok());

        let updated = client.updated_comments.lock().unwrap();
        assert_eq!(updated.len(), 1);
        assert_eq!(updated[0].body, "Updated comment body");
        assert_eq!(updated[0].comment.comment_id, 12345);
    }

    #[rstest]
    #[tokio::test]
    async fn test_creates_new_comment(test_dir: TempDir) {
        let updated_at = "2024-01-02T00:00:00+00:00";
        let issue = create_test_issue(123, "Test Issue", "Body", updated_at);
        let client = MockGitHubClient::new()
            .with_issue("owner", "repo", issue)
            .with_comments("owner", "repo", 123, vec![])
            .with_current_user("testuser");

        setup_storage(
            test_dir.path(),
            "Body",
            &create_metadata_json(123, "Test Issue", updated_at, &["bug"]),
        );
        setup_comment(
            test_dir.path(),
            "new_my_comment.md",
            "This is a new comment",
        );

        let storage = IssueStorage::from_dir(test_dir.path());
        let args = PushArgs {
            issue: IssueArgs {
                issue_number: 123,
                repo: Some("owner/repo".to_string()),
            },
            dry_run: false,
            force: false,
            edit_others: false,
        };

        let result = run_with_client_and_storage(&args, &client, &storage, "testuser").await;
        assert!(result.is_ok());

        let created = client.created_comments.lock().unwrap();
        assert_eq!(created.len(), 1);
        assert_eq!(created[0].body, "This is a new comment");

        // New comment file should be deleted after successful creation
        assert!(!test_dir.path().join("comments/new_my_comment.md").exists());
    }

    #[rstest]
    #[tokio::test]
    async fn test_fails_editing_others_comment_without_flag(test_dir: TempDir) {
        let updated_at = "2024-01-02T00:00:00+00:00";
        let issue = create_test_issue(123, "Test Issue", "Body", updated_at);
        let remote_comment = Comment {
            id: "IC_abc123".to_string(),
            database_id: 12345,
            author: Some(Author {
                login: "otheruser".to_string(),
            }),
            created_at: Utc.with_ymd_and_hms(2024, 1, 1, 12, 0, 0).unwrap(),
            body: "Original comment".to_string(),
        };
        let client = MockGitHubClient::new()
            .with_issue("owner", "repo", issue)
            .with_comments("owner", "repo", 123, vec![remote_comment])
            .with_current_user("testuser");

        setup_storage(
            test_dir.path(),
            "Body",
            &create_metadata_json(123, "Test Issue", updated_at, &["bug"]),
        );
        setup_comment(
            test_dir.path(),
            "001_comment_12345.md",
            &create_comment_file(
                "otheruser",
                "2024-01-01T12:00:00+00:00",
                "IC_abc123",
                12345,
                "Modified other's comment",
            ),
        );

        let storage = IssueStorage::from_dir(test_dir.path());
        let args = PushArgs {
            issue: IssueArgs {
                issue_number: 123,
                repo: Some("owner/repo".to_string()),
            },
            dry_run: false,
            force: false,
            edit_others: false, // Flag not set
        };

        let result = run_with_client_and_storage(&args, &client, &storage, "testuser").await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("Cannot edit other user's comment"));
        assert!(err.contains("--edit-others"));
    }

    #[rstest]
    #[tokio::test]
    async fn test_allows_editing_others_comment_with_flag(test_dir: TempDir) {
        let updated_at = "2024-01-02T00:00:00+00:00";
        let issue = create_test_issue(123, "Test Issue", "Body", updated_at);
        let remote_comment = Comment {
            id: "IC_abc123".to_string(),
            database_id: 12345,
            author: Some(Author {
                login: "otheruser".to_string(),
            }),
            created_at: Utc.with_ymd_and_hms(2024, 1, 1, 12, 0, 0).unwrap(),
            body: "Original comment".to_string(),
        };
        let client = MockGitHubClient::new()
            .with_issue("owner", "repo", issue)
            .with_comments("owner", "repo", 123, vec![remote_comment])
            .with_current_user("testuser");

        setup_storage(
            test_dir.path(),
            "Body",
            &create_metadata_json(123, "Test Issue", updated_at, &["bug"]),
        );
        setup_comment(
            test_dir.path(),
            "001_comment_12345.md",
            &create_comment_file(
                "otheruser",
                "2024-01-01T12:00:00+00:00",
                "IC_abc123",
                12345,
                "Modified other's comment",
            ),
        );

        let storage = IssueStorage::from_dir(test_dir.path());
        let args = PushArgs {
            issue: IssueArgs {
                issue_number: 123,
                repo: Some("owner/repo".to_string()),
            },
            dry_run: false,
            force: false,
            edit_others: true, // Flag set
        };

        let result = run_with_client_and_storage(&args, &client, &storage, "testuser").await;
        assert!(result.is_ok());

        let updated = client.updated_comments.lock().unwrap();
        assert_eq!(updated.len(), 1);
        assert_eq!(updated[0].body, "Modified other's comment");
    }

    #[rstest]
    #[tokio::test]
    async fn test_updates_metadata_after_push(test_dir: TempDir) {
        let initial_updated_at = "2024-01-02T00:00:00+00:00";
        let new_updated_at = "2024-01-03T00:00:00+00:00";
        // Remote issue has new timestamp, and we use different title to trigger a change
        let mut issue = create_test_issue(123, "New Title", "Body", new_updated_at);
        issue.body = Some("Body".to_string());

        let client = MockGitHubClient::new()
            .with_issue("owner", "repo", issue)
            .with_comments("owner", "repo", 123, vec![]);

        setup_storage(
            test_dir.path(),
            "Body",
            &create_metadata_json(123, "Old Title", initial_updated_at, &["bug"]),
        );

        let storage = IssueStorage::from_dir(test_dir.path());
        let args = PushArgs {
            issue: IssueArgs {
                issue_number: 123,
                repo: Some("owner/repo".to_string()),
            },
            dry_run: false,
            force: true, // Force to bypass timestamp check
            edit_others: false,
        };

        let result = run_with_client_and_storage(&args, &client, &storage, "testuser").await;
        assert!(result.is_ok());

        // Verify title was updated
        let updated = client.updated_issue_titles.lock().unwrap();
        assert_eq!(updated.len(), 1);

        // Verify metadata was updated with new timestamp from re-fetched issue
        let metadata_content = fs::read_to_string(test_dir.path().join("metadata.json")).unwrap();
        assert!(metadata_content.contains("2024-01-03"));
    }
}

mod format_diff_tests {
    use super::*;

    #[rstest]
    #[case::single_line("line1\n", "line1\n", vec![" line1"])]
    #[case::multiple_lines("line1\nline2\n", "line1\nline2\n", vec![" line1", " line2"])]
    #[case::empty("", "", vec![])]
    fn test_no_changes(#[case] old: &str, #[case] new: &str, #[case] expected_lines: Vec<&str>) {
        let diff = format_diff(old, new);
        for line in expected_lines {
            assert!(
                diff.contains(line),
                "Expected '{}' in diff:\n{}",
                line,
                diff
            );
        }
        // No changes should not have - or + markers (except in content)
        let lines: Vec<&str> = diff.lines().collect();
        for line in lines {
            assert!(
                line.starts_with(' ') || line.is_empty(),
                "Expected no changes but found: {}",
                line
            );
        }
    }

    #[rstest]
    #[case::add_one_line("line1\n", "line1\nline2\n", vec![" line1", "+line2"])]
    #[case::add_multiple("a\n", "a\nb\nc\n", vec![" a", "+b", "+c"])]
    #[case::add_to_empty("", "new\n", vec!["+new"])]
    fn test_additions(#[case] old: &str, #[case] new: &str, #[case] expected: Vec<&str>) {
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

    #[rstest]
    #[case::delete_one_line("line1\nline2\n", "line1\n", vec![" line1", "-line2"])]
    #[case::delete_multiple("a\nb\nc\n", "a\n", vec![" a", "-b", "-c"])]
    #[case::delete_all("old\n", "", vec!["-old"])]
    fn test_deletions(#[case] old: &str, #[case] new: &str, #[case] expected: Vec<&str>) {
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

    #[rstest]
    #[case::simple_modification("old\n", "new\n", vec!["-old", "+new"])]
    #[case::modify_middle("a\nold\nc\n", "a\nnew\nc\n", vec![" a", "-old", "+new", " c"])]
    #[case::complex_change("foo\nbar\nbaz\n", "foo\nqux\nbaz\n", vec![" foo", "-bar", "+qux", " baz"])]
    fn test_modifications(#[case] old: &str, #[case] new: &str, #[case] expected: Vec<&str>) {
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

    #[rstest]
    #[case::mixed_operations(
        "keep\ndelete\nmodify\n",
        "keep\nmodified\nnew\n",
        vec![" keep", "-delete", "-modify", "+modified", "+new"]
    )]
    fn test_mixed_changes(#[case] old: &str, #[case] new: &str, #[case] expected: Vec<&str>) {
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
