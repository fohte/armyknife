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

    const TS: &str = "2024-01-02T00:00:00+00:00";
    const OLD_TS: &str = "2024-01-01T00:00:00+00:00";

    /// Test fixture builder with sensible defaults.
    struct TestSetup<'a> {
        dir: &'a std::path::Path,
        // Remote state
        remote_title: &'a str,
        remote_body: &'a str,
        remote_ts: &'a str,
        remote_comments: Vec<Comment>,
        // Local state
        local_title: &'a str,
        local_body: &'a str,
        local_labels: Vec<&'a str>,
        local_ts: &'a str,
        // Args
        dry_run: bool,
        force: bool,
        edit_others: bool,
        current_user: &'a str,
    }

    impl<'a> TestSetup<'a> {
        fn new(dir: &'a std::path::Path) -> Self {
            Self {
                dir,
                remote_title: "Title",
                remote_body: "Body",
                remote_ts: TS,
                remote_comments: vec![],
                local_title: "Title",
                local_body: "Body",
                local_labels: vec!["bug"],
                local_ts: TS,
                dry_run: false,
                force: false,
                edit_others: false,
                current_user: "testuser",
            }
        }

        fn remote_title(mut self, v: &'a str) -> Self {
            self.remote_title = v;
            self
        }
        fn remote_body(mut self, v: &'a str) -> Self {
            self.remote_body = v;
            self
        }
        fn remote_ts(mut self, v: &'a str) -> Self {
            self.remote_ts = v;
            self
        }
        fn remote_comments(mut self, v: Vec<Comment>) -> Self {
            self.remote_comments = v;
            self
        }
        fn local_title(mut self, v: &'a str) -> Self {
            self.local_title = v;
            self
        }
        fn local_body(mut self, v: &'a str) -> Self {
            self.local_body = v;
            self
        }
        fn local_labels(mut self, v: Vec<&'a str>) -> Self {
            self.local_labels = v;
            self
        }
        fn local_ts(mut self, v: &'a str) -> Self {
            self.local_ts = v;
            self
        }
        fn dry_run(mut self, v: bool) -> Self {
            self.dry_run = v;
            self
        }
        fn force(mut self, v: bool) -> Self {
            self.force = v;
            self
        }
        fn edit_others(mut self, v: bool) -> Self {
            self.edit_others = v;
            self
        }

        fn build(self) -> (MockGitHubClient, IssueStorage, PushArgs) {
            let issue = create_test_issue(123, self.remote_title, self.remote_body, self.remote_ts);
            let client = MockGitHubClient::new()
                .with_issue("owner", "repo", issue)
                .with_comments("owner", "repo", 123, self.remote_comments)
                .with_current_user(self.current_user);

            fs::create_dir_all(self.dir).unwrap();
            fs::write(self.dir.join("issue.md"), format!("{}\n", self.local_body)).unwrap();
            fs::write(
                self.dir.join("metadata.json"),
                create_metadata_json(123, self.local_title, self.local_ts, &self.local_labels),
            )
            .unwrap();

            let storage = IssueStorage::from_dir(self.dir);
            let args = PushArgs {
                issue: IssueArgs {
                    issue_number: 123,
                    repo: Some("owner/repo".to_string()),
                },
                dry_run: self.dry_run,
                force: self.force,
                edit_others: self.edit_others,
            };
            (client, storage, args)
        }
    }

    fn setup_local_comment(dir: &std::path::Path, filename: &str, content: &str) {
        let comments_dir = dir.join("comments");
        fs::create_dir_all(&comments_dir).unwrap();
        fs::write(comments_dir.join(filename), content).unwrap();
    }

    fn make_comment(author: &str) -> Comment {
        Comment {
            id: "IC_abc".to_string(),
            database_id: 12345,
            author: Some(Author {
                login: author.to_string(),
            }),
            created_at: Utc.with_ymd_and_hms(2024, 1, 1, 12, 0, 0).unwrap(),
            body: "Original".to_string(),
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
        let (client, storage, args) = TestSetup::new(test_dir.path())
            .local_body(local_body)
            .remote_body(remote_body)
            .dry_run(dry_run)
            .build();

        let result = run_with_client_and_storage(&args, &client, &storage, "testuser").await;
        assert!(result.is_ok());
        assert_eq!(client.updated_issue_bodies.lock().unwrap().len(), expected);
    }

    #[rstest]
    #[tokio::test]
    async fn test_updates_title(test_dir: TempDir) {
        let (client, storage, args) = TestSetup::new(test_dir.path())
            .remote_title("Old Title")
            .local_title("New Title")
            .build();

        let result = run_with_client_and_storage(&args, &client, &storage, "testuser").await;
        assert!(result.is_ok());
        assert_eq!(
            client.updated_issue_titles.lock().unwrap()[0].title,
            "New Title"
        );
    }

    #[rstest]
    #[tokio::test]
    async fn test_updates_labels(test_dir: TempDir) {
        let (client, storage, args) = TestSetup::new(test_dir.path())
            .local_labels(vec!["enhancement"])
            .build();

        let result = run_with_client_and_storage(&args, &client, &storage, "testuser").await;
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
        let (client, storage, args) = TestSetup::new(test_dir.path())
            .local_ts(OLD_TS) // Local has old timestamp, remote has new
            .force(force)
            .build();

        let result = run_with_client_and_storage(&args, &client, &storage, "testuser").await;
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
        let (client, storage, _) = TestSetup::new(test_dir.path()).build();
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
        let (client, storage, args) = TestSetup::new(test_dir.path())
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

        let result = run_with_client_and_storage(&args, &client, &storage, "testuser").await;
        assert!(result.is_ok());
        assert_eq!(client.updated_comments.lock().unwrap()[0].body, "Updated");
    }

    #[rstest]
    #[tokio::test]
    async fn test_creates_new_comment(test_dir: TempDir) {
        let (client, storage, args) = TestSetup::new(test_dir.path()).build();
        setup_local_comment(test_dir.path(), "new_my_comment.md", "New comment");

        let result = run_with_client_and_storage(&args, &client, &storage, "testuser").await;
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
        let (client, storage, args) = TestSetup::new(test_dir.path())
            .remote_comments(vec![make_comment("otheruser")])
            .edit_others(edit_others)
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

        let result = run_with_client_and_storage(&args, &client, &storage, "testuser").await;
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
        let (client, storage, args) = TestSetup::new(test_dir.path())
            .remote_ts(new_ts)
            .local_ts(OLD_TS)
            .local_title("Old Title")
            .remote_title("New Title")
            .force(true)
            .build();

        let result = run_with_client_and_storage(&args, &client, &storage, "testuser").await;
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
