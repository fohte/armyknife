use std::collections::HashMap;
use std::io::{self, Write};

use clap::Args;

use super::common::{get_repo_from_arg_or_git, parse_repo, print_fetch_success, write_diff};
use crate::commands::gh::issue_agent::models::{Comment, Issue, IssueFrontmatter};
use crate::commands::gh::issue_agent::storage::{IssueStorage, LocalChanges};
use crate::infra::github::OctocrabClient;

#[derive(Args, Clone, PartialEq, Eq, Debug)]
pub struct PullArgs {
    #[command(flatten)]
    pub issue: super::IssueArgs,

    /// Discard local changes and fetch latest (force overwrite)
    #[arg(short, long)]
    pub force: bool,
}

pub async fn run(args: &PullArgs) -> anyhow::Result<()> {
    let client = OctocrabClient::get()?;
    run_with_client(args, client).await
}

/// Internal implementation that accepts a client for testability.
pub(super) async fn run_with_client(
    args: &PullArgs,
    client: &OctocrabClient,
) -> anyhow::Result<()> {
    let repo = get_repo_from_arg_or_git(&args.issue.repo)?;
    let issue_number = args.issue.issue_number;

    let action = if args.force { "Refreshing" } else { "Fetching" };
    eprintln!("{action} issue #{issue_number} from {repo}...");

    let storage = IssueStorage::new(&repo, issue_number as i64);

    let title = run_with_client_and_storage(args, client, &storage).await?;

    // Print success message
    print_fetch_success(issue_number, &title, storage.dir());

    Ok(())
}

/// Core implementation with custom storage. Returns the issue title on success.
async fn run_with_client_and_storage(
    args: &PullArgs,
    client: &OctocrabClient,
    storage: &IssueStorage,
) -> anyhow::Result<String> {
    let repo = get_repo_from_arg_or_git(&args.issue.repo)?;
    let issue_number = args.issue.issue_number;

    let (owner, repo_name) = parse_repo(&repo)?;

    // Fetch issue and comments from GitHub
    let issue = client.get_issue(&owner, &repo_name, issue_number).await?;
    let comments = client
        .get_comments(&owner, &repo_name, issue_number)
        .await?;

    // Check for local changes before overwriting
    if storage.dir().exists() {
        let changes = storage.detect_changes(&issue, &comments)?;
        if changes.has_changes() {
            // Always show diff when there are local changes
            display_local_changes(storage, &issue, &comments, &changes)?;

            if !args.force {
                anyhow::bail!(
                    "Local changes would be overwritten. Use 'pull --force' to discard local changes."
                );
            }
        }
    }

    // Save to local storage
    save_issue_to_storage(storage, &issue, &comments)?;

    Ok(issue.title.clone())
}

/// Display local changes that differ from remote to stdout.
/// Ignores BrokenPipe errors (e.g., when piped to `head`).
fn display_local_changes(
    storage: &IssueStorage,
    remote_issue: &Issue,
    remote_comments: &[Comment],
    changes: &LocalChanges,
) -> anyhow::Result<()> {
    if let Err(e) = write_local_changes(
        &mut io::stdout(),
        storage,
        remote_issue,
        remote_comments,
        changes,
    ) {
        // Ignore BrokenPipe errors (e.g., when piped to `head`)
        if let Some(io_err) = e.downcast_ref::<io::Error>()
            && io_err.kind() == io::ErrorKind::BrokenPipe
        {
            return Ok(());
        }
        return Err(e);
    }
    Ok(())
}

/// Write local changes that differ from remote to a writer.
fn write_local_changes<W: Write>(
    writer: &mut W,
    storage: &IssueStorage,
    remote_issue: &Issue,
    remote_comments: &[Comment],
    changes: &LocalChanges,
) -> anyhow::Result<()> {
    writeln!(writer)?;
    writeln!(writer, "=== Local changes detected ===")?;

    // Show body diff (local -> remote, so local changes are shown as deleted)
    if changes.body_changed
        && let Ok(local_body) = storage.read_body()
    {
        let remote_body = remote_issue.body.as_deref().unwrap_or("");
        writeln!(writer)?;
        writeln!(writer, "=== Issue Body ===")?;
        write_diff(writer, &local_body, remote_body, false)?;
    }

    // Show title diff
    if changes.title_changed
        && let Ok(local_metadata) = storage.read_metadata()
    {
        writeln!(writer)?;
        writeln!(writer, "=== Title ===")?;
        writeln!(writer, "- {}", local_metadata.title)?;
        writeln!(writer, "+ {}", remote_issue.title)?;
    }

    // Show comment changes (read comments once for both modified and new)
    if (!changes.modified_comment_ids.is_empty() || !changes.new_comment_files.is_empty())
        && let Ok(local_comments) = storage.read_comments()
    {
        // Show modified comments
        if !changes.modified_comment_ids.is_empty() {
            let remote_comments_map: HashMap<&str, &Comment> =
                remote_comments.iter().map(|c| (c.id.as_str(), c)).collect();

            for local_comment in &local_comments {
                if let Some(comment_id) = &local_comment.metadata.id
                    && changes.modified_comment_ids.contains(comment_id)
                    && let Some(remote_comment) = remote_comments_map.get(comment_id.as_str())
                {
                    writeln!(writer)?;
                    writeln!(writer, "=== Comment: {} ===", local_comment.filename)?;
                    write_diff(writer, &local_comment.body, &remote_comment.body, false)?;
                }
            }
        }

        // Show new comments that will be deleted
        for local_comment in &local_comments {
            if changes.new_comment_files.contains(&local_comment.filename) {
                writeln!(writer)?;
                writeln!(
                    writer,
                    "=== New Comment (will be deleted): {} ===",
                    local_comment.filename
                )?;
                for line in local_comment.body.lines() {
                    writeln!(writer, "- {}", line)?;
                }
            }
        }
    }

    writeln!(writer)?;

    Ok(())
}

/// Save issue data to local storage.
pub(super) fn save_issue_to_storage(
    storage: &IssueStorage,
    issue: &crate::commands::gh::issue_agent::models::Issue,
    comments: &[crate::commands::gh::issue_agent::models::Comment],
) -> anyhow::Result<()> {
    // Save issue with frontmatter
    let body = issue.body.as_deref().unwrap_or("");
    let frontmatter = IssueFrontmatter::from_issue(issue);
    storage.save_issue(&frontmatter, body)?;

    // Save comments
    storage.save_comments(comments)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::gh::issue_agent::commands::IssueArgs;
    use crate::commands::gh::issue_agent::commands::test_helpers::{GitHubMockServer, test_dir};
    use crate::commands::gh::issue_agent::models::{Author, Comment, Issue};
    use crate::commands::gh::issue_agent::testing::factories;
    use chrono::{TimeZone, Utc};
    use indoc::indoc;
    use rstest::rstest;
    use std::fs;
    use tempfile::TempDir;

    mod save_issue_to_storage_tests {
        use super::*;

        fn test_issue() -> Issue {
            factories::issue_with(|i| {
                i.number = 123;
                i.title = "Test Issue".to_string();
                i.body = Some("Test body content".to_string());
                i.labels = factories::labels(&["bug"]);
                i.assignees = factories::assignees(&["assignee1"]);
                i.created_at = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap();
                i.updated_at = Utc.with_ymd_and_hms(2024, 1, 2, 0, 0, 0).unwrap();
            })
        }

        fn test_comment() -> Comment {
            factories::comment_with(|c| {
                c.id = "IC_abc123".to_string();
                c.database_id = 12345;
                c.author = Some(factories::author("commenter"));
                c.created_at = Utc.with_ymd_and_hms(2024, 1, 1, 12, 0, 0).unwrap();
                c.body = "Test comment body".to_string();
            })
        }

        #[rstest]
        fn test_saves_issue_body(test_dir: TempDir) {
            let storage = IssueStorage::from_dir(test_dir.path());
            save_issue_to_storage(&storage, &test_issue(), &[]).unwrap();

            let content = fs::read_to_string(test_dir.path().join("issue.md")).unwrap();
            // Should contain frontmatter and body
            assert!(content.contains("---"));
            assert!(content.contains("Test body content"));
        }

        #[rstest]
        fn test_saves_empty_body_when_none(test_dir: TempDir) {
            let mut issue = test_issue();
            issue.body = None;
            let storage = IssueStorage::from_dir(test_dir.path());
            save_issue_to_storage(&storage, &issue, &[]).unwrap();

            let content = fs::read_to_string(test_dir.path().join("issue.md")).unwrap();
            // Should contain frontmatter with empty body
            assert!(content.contains("---"));
            assert!(content.contains("title: Test Issue"));
        }

        #[rstest]
        fn test_saves_metadata(test_dir: TempDir) {
            let storage = IssueStorage::from_dir(test_dir.path());
            save_issue_to_storage(&storage, &test_issue(), &[]).unwrap();

            // Metadata should now be in frontmatter of issue.md
            let content = fs::read_to_string(test_dir.path().join("issue.md")).unwrap();
            assert!(content.contains("title: Test Issue"));
            assert!(content.contains("readonly:"));
            assert!(content.contains("number: 123"));
            assert!(content.contains("state: OPEN"));
        }

        #[rstest]
        fn test_saves_comments(test_dir: TempDir) {
            let storage = IssueStorage::from_dir(test_dir.path());
            save_issue_to_storage(&storage, &test_issue(), &[test_comment()]).unwrap();

            let comments_dir = test_dir.path().join("comments");
            assert!(comments_dir.exists());

            let comment_file = comments_dir.join("001_comment_12345.md");
            assert!(comment_file.exists());

            let content = fs::read_to_string(&comment_file).unwrap();
            assert_eq!(
                content,
                indoc! {"
                    <!-- author: commenter -->
                    <!-- createdAt: 2024-01-01T12:00:00+00:00 -->
                    <!-- id: IC_abc123 -->
                    <!-- databaseId: 12345 -->

                    Test comment body
                "}
            );
        }

        #[rstest]
        fn test_saves_multiple_comments(test_dir: TempDir) {
            let comments = vec![
                Comment {
                    id: "IC_1".to_string(),
                    database_id: 1001,
                    author: Some(Author {
                        login: "user1".to_string(),
                    }),
                    created_at: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
                    body: "First comment".to_string(),
                },
                Comment {
                    id: "IC_2".to_string(),
                    database_id: 1002,
                    author: Some(Author {
                        login: "user2".to_string(),
                    }),
                    created_at: Utc.with_ymd_and_hms(2024, 1, 2, 0, 0, 0).unwrap(),
                    body: "Second comment".to_string(),
                },
            ];

            let storage = IssueStorage::from_dir(test_dir.path());
            save_issue_to_storage(&storage, &test_issue(), &comments).unwrap();

            let comments_dir = test_dir.path().join("comments");
            assert!(comments_dir.join("001_comment_1001.md").exists());
            assert!(comments_dir.join("002_comment_1002.md").exists());
        }
    }

    mod run_with_client_and_storage_tests {
        use super::*;
        use indoc::indoc;

        #[rstest]
        #[tokio::test]
        async fn test_fetches_and_saves_issue(test_dir: TempDir) {
            let mock = GitHubMockServer::start().await;
            let ctx = mock.repo("owner", "repo");
            ctx.issue(123).get().await;
            ctx.graphql_comments(&[]).await;

            let client = mock.client();
            let storage = IssueStorage::from_dir(test_dir.path());
            let args = PullArgs {
                issue: IssueArgs {
                    issue_number: 123,
                    repo: Some("owner/repo".to_string()),
                },
                force: false,
            };

            run_with_client_and_storage(&args, &client, &storage)
                .await
                .unwrap();

            // Verify files were created
            assert!(test_dir.path().join("issue.md").exists());
            assert!(test_dir.path().join("comments").exists());

            // Verify issue.md with frontmatter
            let issue_md = fs::read_to_string(test_dir.path().join("issue.md")).unwrap();
            assert_eq!(
                issue_md,
                indoc! {"
                    ---
                    title: Test Issue
                    labels:
                    - bug
                    assignees: []
                    milestone: null
                    readonly:
                      number: 123
                      state: OPEN
                      author: testuser
                      createdAt: 2024-01-01T00:00:00+00:00
                      updatedAt: 2024-01-02T00:00:00+00:00
                    ---

                    Test body
                "}
            );
        }

        #[rstest]
        #[tokio::test]
        async fn test_fails_when_local_changes_exist(test_dir: TempDir) {
            let mock = GitHubMockServer::start().await;
            let ctx = mock.repo("owner", "repo");
            ctx.issue(123).get().await;
            ctx.graphql_comments(&[]).await;

            let client = mock.client();

            // Create storage dir with modified content
            fs::create_dir_all(test_dir.path()).unwrap();
            fs::write(test_dir.path().join("issue.md"), "Modified body\n").unwrap();

            let storage = IssueStorage::from_dir(test_dir.path());
            let args = PullArgs {
                issue: IssueArgs {
                    issue_number: 123,
                    repo: Some("owner/repo".to_string()),
                },
                force: false,
            };

            let result = run_with_client_and_storage(&args, &client, &storage).await;
            assert!(result.is_err());
            assert_eq!(
                result.unwrap_err().to_string(),
                "Local changes would be overwritten. Use 'pull --force' to discard local changes."
            );
        }

        #[rstest]
        #[tokio::test]
        async fn test_succeeds_when_no_local_changes(test_dir: TempDir) {
            let mock = GitHubMockServer::start().await;
            let ctx = mock.repo("owner", "repo");
            ctx.issue(123).get().await;
            ctx.graphql_comments(&[]).await;

            let client = mock.client();

            // Create storage dir with matching content (no changes)
            fs::create_dir_all(test_dir.path()).unwrap();
            fs::write(test_dir.path().join("issue.md"), "Test body\n").unwrap();

            let storage = IssueStorage::from_dir(test_dir.path());
            let args = PullArgs {
                issue: IssueArgs {
                    issue_number: 123,
                    repo: Some("owner/repo".to_string()),
                },
                force: false,
            };

            // Should succeed because local content matches remote
            let result = run_with_client_and_storage(&args, &client, &storage).await;
            assert!(result.is_ok());
        }

        #[rstest]
        #[tokio::test]
        async fn test_succeeds_when_dir_does_not_exist(test_dir: TempDir) {
            let mock = GitHubMockServer::start().await;
            let ctx = mock.repo("owner", "repo");
            ctx.issue(123).get().await;
            ctx.graphql_comments(&[]).await;

            let client = mock.client();

            // Use a non-existent subdirectory
            let storage = IssueStorage::from_dir(test_dir.path().join("new_dir"));
            let args = PullArgs {
                issue: IssueArgs {
                    issue_number: 123,
                    repo: Some("owner/repo".to_string()),
                },
                force: false,
            };

            let result = run_with_client_and_storage(&args, &client, &storage).await;
            assert!(result.is_ok());
            assert!(test_dir.path().join("new_dir/issue.md").exists());
        }

        #[rstest]
        #[tokio::test]
        async fn test_fails_when_issue_not_found(test_dir: TempDir) {
            let mock = GitHubMockServer::start().await;
            mock.repo("owner", "repo").issue(999).get_not_found().await;

            let client = mock.client();

            let storage = IssueStorage::from_dir(test_dir.path());
            let args = PullArgs {
                issue: IssueArgs {
                    issue_number: 999,
                    repo: Some("owner/repo".to_string()),
                },
                force: false,
            };

            let result = run_with_client_and_storage(&args, &client, &storage).await;
            assert!(result.is_err());
        }

        #[rstest]
        #[tokio::test]
        async fn test_fails_with_invalid_repo_format(test_dir: TempDir) {
            let mock = GitHubMockServer::start().await;
            let client = mock.client();

            let storage = IssueStorage::from_dir(test_dir.path());
            let args = PullArgs {
                issue: IssueArgs {
                    issue_number: 123,
                    repo: Some("invalid-repo-format".to_string()),
                },
                force: false,
            };

            let result = run_with_client_and_storage(&args, &client, &storage).await;
            assert!(result.is_err());
            assert_eq!(
                result.unwrap_err().to_string(),
                "Invalid input: Invalid repository format: invalid-repo-format. Expected owner/repo"
            );
        }
    }

    mod run_with_client_and_storage_force_tests {
        use super::*;
        use crate::commands::gh::issue_agent::commands::test_helpers::RemoteComment;
        use indoc::indoc;

        #[rstest]
        #[tokio::test]
        async fn test_force_overwrites_local_changes(test_dir: TempDir) {
            let mock = GitHubMockServer::start().await;
            let ctx = mock.repo("owner", "repo");
            ctx.issue(123).get().await;
            ctx.graphql_comments(&[]).await;

            let client = mock.client();

            // Create storage dir with different content (simulating local changes)
            fs::create_dir_all(test_dir.path()).unwrap();
            fs::write(
                test_dir.path().join("issue.md"),
                "Old content that will be overwritten\n",
            )
            .unwrap();

            let storage = IssueStorage::from_dir(test_dir.path());
            let args = PullArgs {
                issue: IssueArgs {
                    issue_number: 123,
                    repo: Some("owner/repo".to_string()),
                },
                force: true,
            };

            // With --force, should succeed even with local changes
            let result = run_with_client_and_storage(&args, &client, &storage).await;
            assert!(result.is_ok());

            // Verify content was overwritten with frontmatter
            let issue_md = fs::read_to_string(test_dir.path().join("issue.md")).unwrap();
            assert_eq!(
                issue_md,
                indoc! {"
                    ---
                    title: Test Issue
                    labels:
                    - bug
                    assignees: []
                    milestone: null
                    readonly:
                      number: 123
                      state: OPEN
                      author: testuser
                      createdAt: 2024-01-01T00:00:00+00:00
                      updatedAt: 2024-01-02T00:00:00+00:00
                    ---

                    Test body
                "}
            );
        }

        #[rstest]
        #[tokio::test]
        async fn test_force_fetches_and_saves_issue(test_dir: TempDir) {
            let mock = GitHubMockServer::start().await;
            let ctx = mock.repo("owner", "repo");
            ctx.issue(123).get().await;
            ctx.graphql_comments(&[RemoteComment {
                id: "IC_abc123",
                database_id: 12345,
                author: "commenter",
                body: "Test comment body",
            }])
            .await;

            let client = mock.client();
            let storage = IssueStorage::from_dir(test_dir.path());
            let args = PullArgs {
                issue: IssueArgs {
                    issue_number: 123,
                    repo: Some("owner/repo".to_string()),
                },
                force: true,
            };

            run_with_client_and_storage(&args, &client, &storage)
                .await
                .unwrap();

            // Verify files were created
            assert!(test_dir.path().join("issue.md").exists());
            assert!(test_dir.path().join("comments").exists());

            // Verify issue.md with frontmatter
            let issue_md = fs::read_to_string(test_dir.path().join("issue.md")).unwrap();
            assert_eq!(
                issue_md,
                indoc! {"
                    ---
                    title: Test Issue
                    labels:
                    - bug
                    assignees: []
                    milestone: null
                    readonly:
                      number: 123
                      state: OPEN
                      author: testuser
                      createdAt: 2024-01-01T00:00:00+00:00
                      updatedAt: 2024-01-02T00:00:00+00:00
                    ---

                    Test body
                "}
            );
        }

        #[rstest]
        #[tokio::test]
        async fn test_force_succeeds_when_dir_does_not_exist(test_dir: TempDir) {
            let mock = GitHubMockServer::start().await;
            let ctx = mock.repo("owner", "repo");
            ctx.issue(123).get().await;
            ctx.graphql_comments(&[]).await;

            let client = mock.client();

            // Use a non-existent subdirectory
            let storage = IssueStorage::from_dir(test_dir.path().join("new_dir"));
            let args = PullArgs {
                issue: IssueArgs {
                    issue_number: 123,
                    repo: Some("owner/repo".to_string()),
                },
                force: true,
            };

            let result = run_with_client_and_storage(&args, &client, &storage).await;
            assert!(result.is_ok());
            assert!(test_dir.path().join("new_dir/issue.md").exists());
        }

        #[rstest]
        #[tokio::test]
        async fn test_force_overwrites_comments(test_dir: TempDir) {
            let mock = GitHubMockServer::start().await;
            let ctx = mock.repo("owner", "repo");
            ctx.issue(123).get().await;
            ctx.graphql_comments(&[RemoteComment {
                id: "IC_new",
                database_id: 99999,
                author: "newuser",
                body: "New comment from refresh",
            }])
            .await;

            let client = mock.client();

            // Create storage dir with old comments
            let comments_dir = test_dir.path().join("comments");
            fs::create_dir_all(&comments_dir).unwrap();
            fs::write(comments_dir.join("001_comment_11111.md"), "Old comment").unwrap();

            let storage = IssueStorage::from_dir(test_dir.path());
            let args = PullArgs {
                issue: IssueArgs {
                    issue_number: 123,
                    repo: Some("owner/repo".to_string()),
                },
                force: true,
            };

            run_with_client_and_storage(&args, &client, &storage)
                .await
                .unwrap();

            // Verify new comment file exists with expected content
            assert!(comments_dir.join("001_comment_99999.md").exists());
            let content = fs::read_to_string(comments_dir.join("001_comment_99999.md")).unwrap();
            assert_eq!(
                content,
                indoc! {"
                    <!-- author: newuser -->
                    <!-- createdAt: 2024-01-01T12:00:00+00:00 -->
                    <!-- id: IC_new -->
                    <!-- databaseId: 99999 -->

                    New comment from refresh
                "}
            );
        }
    }

    mod write_local_changes_tests {
        use super::*;
        use crate::commands::gh::issue_agent::models::IssueMetadata;
        use crate::commands::gh::issue_agent::storage::LocalChanges;

        fn to_string<F>(f: F) -> String
        where
            F: FnOnce(&mut Vec<u8>) -> anyhow::Result<()>,
        {
            let mut buf = Vec::new();
            f(&mut buf).unwrap();
            String::from_utf8(buf).unwrap()
        }

        #[rstest]
        fn test_body_changes(test_dir: TempDir) {
            let storage = IssueStorage::from_dir(test_dir.path());

            fs::write(test_dir.path().join("issue.md"), "Local body\n").unwrap();

            let remote_issue = factories::issue_with(|i| {
                i.body = Some("Remote body".to_string());
            });

            let changes = LocalChanges {
                body_changed: true,
                title_changed: false,
                modified_comment_ids: vec![],
                new_comment_files: vec![],
            };

            let output =
                to_string(|w| write_local_changes(w, &storage, &remote_issue, &[], &changes));
            assert_eq!(
                output,
                indoc! {"

                    === Local changes detected ===

                    === Issue Body ===
                    -Local body
                    +Remote body

                "}
            );
        }

        #[rstest]
        fn test_title_changes(test_dir: TempDir) {
            let storage = IssueStorage::from_dir(test_dir.path());

            let local_metadata = IssueMetadata {
                number: 123,
                title: "Local Title".to_string(),
                state: "OPEN".to_string(),
                labels: vec![],
                assignees: vec![],
                milestone: None,
                author: "testuser".to_string(),
                created_at: "2024-01-01T00:00:00Z".to_string(),
                updated_at: "2024-01-02T00:00:00Z".to_string(),
            };
            storage.save_metadata(&local_metadata).unwrap();

            let remote_issue = factories::issue_with(|i| {
                i.title = "Remote Title".to_string();
            });

            let changes = LocalChanges {
                body_changed: false,
                title_changed: true,
                modified_comment_ids: vec![],
                new_comment_files: vec![],
            };

            let output =
                to_string(|w| write_local_changes(w, &storage, &remote_issue, &[], &changes));
            assert_eq!(
                output,
                indoc! {"

                    === Local changes detected ===

                    === Title ===
                    - Local Title
                    + Remote Title

                "}
            );
        }

        #[rstest]
        fn test_modified_comments(test_dir: TempDir) {
            let storage = IssueStorage::from_dir(test_dir.path());

            let comments_dir = test_dir.path().join("comments");
            fs::create_dir_all(&comments_dir).unwrap();
            fs::write(
                comments_dir.join("001_comment_12345.md"),
                indoc! {"
                    <!-- author: testuser -->
                    <!-- createdAt: 2024-01-01T00:00:00Z -->
                    <!-- id: IC_abc123 -->
                    <!-- databaseId: 12345 -->

                    Local comment
                "},
            )
            .unwrap();

            let remote_issue = factories::issue();
            let remote_comments = vec![factories::comment_with(|c| {
                c.id = "IC_abc123".to_string();
                c.database_id = 12345;
                c.body = "Remote comment".to_string();
            })];

            let changes = LocalChanges {
                body_changed: false,
                title_changed: false,
                modified_comment_ids: vec!["IC_abc123".to_string()],
                new_comment_files: vec![],
            };

            let output = to_string(|w| {
                write_local_changes(w, &storage, &remote_issue, &remote_comments, &changes)
            });
            assert_eq!(
                output,
                indoc! {"

                    === Local changes detected ===

                    === Comment: 001_comment_12345.md ===
                    -Local comment
                    +Remote comment

                "}
            );
        }

        #[rstest]
        fn test_new_comments_to_be_deleted(test_dir: TempDir) {
            let storage = IssueStorage::from_dir(test_dir.path());

            let comments_dir = test_dir.path().join("comments");
            fs::create_dir_all(&comments_dir).unwrap();
            fs::write(
                comments_dir.join("new_my_comment.md"),
                "New comment line 1\nNew comment line 2",
            )
            .unwrap();

            let remote_issue = factories::issue();

            let changes = LocalChanges {
                body_changed: false,
                title_changed: false,
                modified_comment_ids: vec![],
                new_comment_files: vec!["new_my_comment.md".to_string()],
            };

            let output =
                to_string(|w| write_local_changes(w, &storage, &remote_issue, &[], &changes));
            assert_eq!(
                output,
                indoc! {"

                    === Local changes detected ===

                    === New Comment (will be deleted): new_my_comment.md ===
                    - New comment line 1
                    - New comment line 2

                "}
            );
        }

        #[rstest]
        fn test_all_changes_combined(test_dir: TempDir) {
            let storage = IssueStorage::from_dir(test_dir.path());

            // Set up body
            fs::write(test_dir.path().join("issue.md"), "Local body\n").unwrap();

            // Set up metadata
            let local_metadata = IssueMetadata {
                number: 123,
                title: "Local Title".to_string(),
                state: "OPEN".to_string(),
                labels: vec![],
                assignees: vec![],
                milestone: None,
                author: "testuser".to_string(),
                created_at: "2024-01-01T00:00:00Z".to_string(),
                updated_at: "2024-01-02T00:00:00Z".to_string(),
            };
            storage.save_metadata(&local_metadata).unwrap();

            // Set up comments
            let comments_dir = test_dir.path().join("comments");
            fs::create_dir_all(&comments_dir).unwrap();
            fs::write(
                comments_dir.join("001_comment_12345.md"),
                indoc! {"
                    <!-- author: testuser -->
                    <!-- createdAt: 2024-01-01T00:00:00Z -->
                    <!-- id: IC_abc123 -->
                    <!-- databaseId: 12345 -->

                    Local comment
                "},
            )
            .unwrap();
            fs::write(comments_dir.join("new_draft.md"), "Draft comment").unwrap();

            let remote_issue = factories::issue_with(|i| {
                i.title = "Remote Title".to_string();
                i.body = Some("Remote body".to_string());
            });
            let remote_comments = vec![factories::comment_with(|c| {
                c.id = "IC_abc123".to_string();
                c.database_id = 12345;
                c.body = "Remote comment".to_string();
            })];

            let changes = LocalChanges {
                body_changed: true,
                title_changed: true,
                modified_comment_ids: vec!["IC_abc123".to_string()],
                new_comment_files: vec!["new_draft.md".to_string()],
            };

            let output = to_string(|w| {
                write_local_changes(w, &storage, &remote_issue, &remote_comments, &changes)
            });
            assert_eq!(
                output,
                indoc! {"

                    === Local changes detected ===

                    === Issue Body ===
                    -Local body
                    +Remote body

                    === Title ===
                    - Local Title
                    + Remote Title

                    === Comment: 001_comment_12345.md ===
                    -Local comment
                    +Remote comment

                    === New Comment (will be deleted): new_draft.md ===
                    - Draft comment

                "}
            );
        }
    }
}
