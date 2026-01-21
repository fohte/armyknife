use clap::Args;

use super::common::{get_repo_from_arg_or_git, parse_repo, print_fetch_success};
use crate::commands::gh::issue_agent::models::IssueMetadata;
use crate::commands::gh::issue_agent::storage::IssueStorage;
use crate::infra::github::{CommentClient, IssueClient, OctocrabClient};

#[derive(Args, Clone, PartialEq, Eq, Debug)]
pub struct PullArgs {
    #[command(flatten)]
    pub issue: super::IssueArgs,
}

pub async fn run(args: &PullArgs) -> Result<(), Box<dyn std::error::Error>> {
    let client = OctocrabClient::get()?;
    run_with_client(args, client).await
}

/// Internal implementation that accepts a client for testability.
pub(super) async fn run_with_client<C>(
    args: &PullArgs,
    client: &C,
) -> Result<(), Box<dyn std::error::Error>>
where
    C: IssueClient + CommentClient,
{
    let repo = get_repo_from_arg_or_git(&args.issue.repo)?;
    let issue_number = args.issue.issue_number;

    eprintln!("Fetching issue #{issue_number} from {repo}...");

    let (owner, repo_name) = parse_repo(&repo)?;

    // Fetch issue and comments from GitHub
    let issue = client.get_issue(&owner, &repo_name, issue_number).await?;
    let comments = client
        .get_comments(&owner, &repo_name, issue_number)
        .await?;

    let storage = IssueStorage::new(&repo, issue.number);

    // Check for local changes before overwriting
    if storage.dir().exists() && storage.has_changes(&issue, &comments)? {
        return Err(
            "Local changes would be overwritten. Use 'refresh' to discard local changes.".into(),
        );
    }

    // Save to local storage
    save_issue_to_storage(&storage, &issue, &comments)?;

    // Print success message
    print_fetch_success(issue_number, &issue.title, storage.dir());

    Ok(())
}

/// Internal implementation with custom storage for testability.
#[cfg(test)]
pub(super) async fn run_with_client_and_storage<C>(
    args: &PullArgs,
    client: &C,
    storage: &IssueStorage,
) -> Result<(), Box<dyn std::error::Error>>
where
    C: IssueClient + CommentClient,
{
    let repo = get_repo_from_arg_or_git(&args.issue.repo)?;
    let issue_number = args.issue.issue_number;

    let (owner, repo_name) = parse_repo(&repo)?;

    // Fetch issue and comments from GitHub
    let issue = client.get_issue(&owner, &repo_name, issue_number).await?;
    let comments = client
        .get_comments(&owner, &repo_name, issue_number)
        .await?;

    // Check for local changes before overwriting
    if storage.dir().exists() && storage.has_changes(&issue, &comments)? {
        return Err(
            "Local changes would be overwritten. Use 'refresh' to discard local changes.".into(),
        );
    }

    // Save to local storage
    save_issue_to_storage(storage, &issue, &comments)?;

    Ok(())
}

/// Save issue data to local storage.
pub(super) fn save_issue_to_storage(
    storage: &IssueStorage,
    issue: &crate::commands::gh::issue_agent::models::Issue,
    comments: &[crate::commands::gh::issue_agent::models::Comment],
) -> Result<(), Box<dyn std::error::Error>> {
    // Save issue body
    let body = issue.body.as_deref().unwrap_or("");
    storage.save_body(body)?;

    // Save metadata
    let metadata = IssueMetadata::from_issue(issue);
    storage.save_metadata(&metadata)?;

    // Save comments
    storage.save_comments(comments)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::gh::issue_agent::commands::IssueArgs;
    use crate::commands::gh::issue_agent::commands::test_helpers::{
        test_comment, test_dir, test_issue,
    };
    use crate::commands::gh::issue_agent::models::{Author, Comment, Issue};
    use crate::infra::github::MockGitHubClient;
    use chrono::{TimeZone, Utc};
    use indoc::indoc;
    use rstest::rstest;
    use std::fs;
    use tempfile::TempDir;

    mod save_issue_to_storage_tests {
        use super::*;

        #[rstest]
        fn test_saves_issue_body(test_dir: TempDir, test_issue: Issue) {
            let storage = IssueStorage::from_dir(test_dir.path());
            save_issue_to_storage(&storage, &test_issue, &[]).unwrap();

            let body = fs::read_to_string(test_dir.path().join("issue.md")).unwrap();
            assert_eq!(body, "Test body content\n");
        }

        #[rstest]
        fn test_saves_empty_body_when_none(test_dir: TempDir, mut test_issue: Issue) {
            test_issue.body = None;
            let storage = IssueStorage::from_dir(test_dir.path());
            save_issue_to_storage(&storage, &test_issue, &[]).unwrap();

            let body = fs::read_to_string(test_dir.path().join("issue.md")).unwrap();
            assert_eq!(body, "\n");
        }

        #[rstest]
        fn test_saves_metadata(test_dir: TempDir, test_issue: Issue) {
            let storage = IssueStorage::from_dir(test_dir.path());
            save_issue_to_storage(&storage, &test_issue, &[]).unwrap();

            let metadata_path = test_dir.path().join("metadata.json");
            assert!(metadata_path.exists());

            let content = fs::read_to_string(&metadata_path).unwrap();
            let metadata: IssueMetadata = serde_json::from_str(&content).unwrap();
            assert_eq!(metadata.number, 123);
            assert_eq!(metadata.title, "Test Issue");
            assert_eq!(metadata.state, "OPEN");
        }

        #[rstest]
        fn test_saves_comments(test_dir: TempDir, test_issue: Issue, test_comment: Comment) {
            let storage = IssueStorage::from_dir(test_dir.path());
            save_issue_to_storage(&storage, &test_issue, &[test_comment]).unwrap();

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

                    Test comment body"}
            );
        }

        #[rstest]
        fn test_saves_multiple_comments(test_dir: TempDir, test_issue: Issue) {
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
            save_issue_to_storage(&storage, &test_issue, &comments).unwrap();

            let comments_dir = test_dir.path().join("comments");
            assert!(comments_dir.join("001_comment_1001.md").exists());
            assert!(comments_dir.join("002_comment_1002.md").exists());
        }
    }

    mod run_with_client_and_storage_tests {
        use super::*;

        #[rstest]
        #[tokio::test]
        async fn test_fetches_and_saves_issue(
            test_dir: TempDir,
            test_issue: Issue,
            test_comment: Comment,
        ) {
            let client = MockGitHubClient::new()
                .with_issue("owner", "repo", test_issue.clone())
                .with_comments("owner", "repo", 123, vec![test_comment]);

            let storage = IssueStorage::from_dir(test_dir.path());
            let args = PullArgs {
                issue: IssueArgs {
                    issue_number: 123,
                    repo: Some("owner/repo".to_string()),
                },
            };

            run_with_client_and_storage(&args, &client, &storage)
                .await
                .unwrap();

            // Verify files were created
            assert!(test_dir.path().join("issue.md").exists());
            assert!(test_dir.path().join("metadata.json").exists());
            assert!(test_dir.path().join("comments").exists());
        }

        #[rstest]
        #[tokio::test]
        async fn test_fails_when_local_changes_exist(test_dir: TempDir, test_issue: Issue) {
            let client = MockGitHubClient::new()
                .with_issue("owner", "repo", test_issue.clone())
                .with_comments("owner", "repo", 123, vec![]);

            // Create storage dir with modified content
            fs::create_dir_all(test_dir.path()).unwrap();
            fs::write(test_dir.path().join("issue.md"), "Modified body\n").unwrap();

            let storage = IssueStorage::from_dir(test_dir.path());
            let args = PullArgs {
                issue: IssueArgs {
                    issue_number: 123,
                    repo: Some("owner/repo".to_string()),
                },
            };

            let result = run_with_client_and_storage(&args, &client, &storage).await;
            assert!(result.is_err());
            assert_eq!(
                result.unwrap_err().to_string(),
                "Local changes would be overwritten. Use 'refresh' to discard local changes."
            );
        }

        #[rstest]
        #[tokio::test]
        async fn test_succeeds_when_no_local_changes(test_dir: TempDir, test_issue: Issue) {
            let client = MockGitHubClient::new()
                .with_issue("owner", "repo", test_issue.clone())
                .with_comments("owner", "repo", 123, vec![]);

            // Create storage dir with matching content (no changes)
            fs::create_dir_all(test_dir.path()).unwrap();
            fs::write(test_dir.path().join("issue.md"), "Test body content\n").unwrap();

            let storage = IssueStorage::from_dir(test_dir.path());
            let args = PullArgs {
                issue: IssueArgs {
                    issue_number: 123,
                    repo: Some("owner/repo".to_string()),
                },
            };

            // Should succeed because local content matches remote
            let result = run_with_client_and_storage(&args, &client, &storage).await;
            assert!(result.is_ok());
        }

        #[rstest]
        #[tokio::test]
        async fn test_succeeds_when_dir_does_not_exist(test_dir: TempDir, test_issue: Issue) {
            let client = MockGitHubClient::new()
                .with_issue("owner", "repo", test_issue.clone())
                .with_comments("owner", "repo", 123, vec![]);

            // Use a non-existent subdirectory
            let storage = IssueStorage::from_dir(test_dir.path().join("new_dir"));
            let args = PullArgs {
                issue: IssueArgs {
                    issue_number: 123,
                    repo: Some("owner/repo".to_string()),
                },
            };

            let result = run_with_client_and_storage(&args, &client, &storage).await;
            assert!(result.is_ok());
            assert!(test_dir.path().join("new_dir/issue.md").exists());
        }

        #[rstest]
        #[tokio::test]
        async fn test_fails_when_issue_not_found(test_dir: TempDir) {
            let client = MockGitHubClient::new(); // No issues configured

            let storage = IssueStorage::from_dir(test_dir.path());
            let args = PullArgs {
                issue: IssueArgs {
                    issue_number: 999,
                    repo: Some("owner/repo".to_string()),
                },
            };

            let result = run_with_client_and_storage(&args, &client, &storage).await;
            assert!(result.is_err());
        }

        #[rstest]
        #[tokio::test]
        async fn test_fails_with_invalid_repo_format(test_dir: TempDir, test_issue: Issue) {
            let client = MockGitHubClient::new().with_issue("owner", "repo", test_issue);

            let storage = IssueStorage::from_dir(test_dir.path());
            let args = PullArgs {
                issue: IssueArgs {
                    issue_number: 123,
                    repo: Some("invalid-repo-format".to_string()),
                },
            };

            let result = run_with_client_and_storage(&args, &client, &storage).await;
            assert!(result.is_err());
            assert_eq!(
                result.unwrap_err().to_string(),
                "Invalid input: Invalid repository format: invalid-repo-format. Expected owner/repo"
            );
        }
    }
}
