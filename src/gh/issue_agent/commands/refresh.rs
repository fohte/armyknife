use clap::Args;

use super::common::{get_repo_from_arg_or_gh, parse_repo, print_fetch_success};
use super::pull::save_issue_to_storage;
use crate::gh::issue_agent::storage::IssueStorage;
use crate::github::{CommentClient, IssueClient, OctocrabClient};

#[derive(Args, Clone, PartialEq, Eq, Debug)]
pub struct RefreshArgs {
    #[command(flatten)]
    pub issue: super::IssueArgs,
}

pub async fn run(args: &RefreshArgs) -> Result<(), Box<dyn std::error::Error>> {
    let client = OctocrabClient::get()?;
    run_with_client(args, client).await
}

/// Internal implementation that accepts a client for testability.
async fn run_with_client<C>(
    args: &RefreshArgs,
    client: &C,
) -> Result<(), Box<dyn std::error::Error>>
where
    C: IssueClient + CommentClient,
{
    let repo = get_repo_from_arg_or_gh(&args.issue.repo)?;
    let issue_number = args.issue.issue_number;

    eprintln!("Refreshing issue #{issue_number} from {repo}...");

    let (owner, repo_name) = parse_repo(&repo)?;

    // Fetch issue and comments from GitHub
    let issue = client.get_issue(&owner, &repo_name, issue_number).await?;
    let comments = client
        .get_comments(&owner, &repo_name, issue_number)
        .await?;

    let storage = IssueStorage::new(&repo, issue.number);

    // Save to local storage (overwriting any local changes)
    save_issue_to_storage(&storage, &issue, &comments)?;

    // Print success message
    print_fetch_success(issue_number, &issue.title, storage.dir());

    Ok(())
}

/// Internal implementation with custom storage for testability.
#[cfg(test)]
async fn run_with_client_and_storage<C>(
    args: &RefreshArgs,
    client: &C,
    storage: &IssueStorage,
) -> Result<(), Box<dyn std::error::Error>>
where
    C: IssueClient + CommentClient,
{
    let repo = get_repo_from_arg_or_gh(&args.issue.repo)?;
    let issue_number = args.issue.issue_number;

    let (owner, repo_name) = parse_repo(&repo)?;

    // Fetch issue and comments from GitHub
    let issue = client.get_issue(&owner, &repo_name, issue_number).await?;
    let comments = client
        .get_comments(&owner, &repo_name, issue_number)
        .await?;

    // Save to local storage (overwriting any local changes - no change check)
    save_issue_to_storage(storage, &issue, &comments)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gh::issue_agent::commands::IssueArgs;
    use crate::gh::issue_agent::models::{Author, Comment, Issue, Label};
    use crate::github::MockGitHubClient;
    use chrono::{TimeZone, Utc};
    use rstest::{fixture, rstest};
    use std::fs;
    use tempfile::TempDir;

    #[fixture]
    fn test_dir() -> TempDir {
        tempfile::tempdir().unwrap()
    }

    #[fixture]
    fn test_issue() -> Issue {
        Issue {
            number: 123,
            title: "Test Issue".to_string(),
            body: Some("Test body content".to_string()),
            state: "OPEN".to_string(),
            labels: vec![Label {
                name: "bug".to_string(),
            }],
            assignees: vec![Author {
                login: "assignee1".to_string(),
            }],
            milestone: None,
            author: Some(Author {
                login: "testuser".to_string(),
            }),
            created_at: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            updated_at: Utc.with_ymd_and_hms(2024, 1, 2, 0, 0, 0).unwrap(),
        }
    }

    #[fixture]
    fn test_comment() -> Comment {
        Comment {
            id: "IC_abc123".to_string(),
            database_id: 12345,
            author: Some(Author {
                login: "commenter".to_string(),
            }),
            created_at: Utc.with_ymd_and_hms(2024, 1, 1, 12, 0, 0).unwrap(),
            body: "Test comment body".to_string(),
        }
    }

    // parse_repo and get_repo tests are in commands/common.rs

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
            let args = RefreshArgs {
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
        async fn test_overwrites_local_changes(test_dir: TempDir, test_issue: Issue) {
            let client = MockGitHubClient::new()
                .with_issue("owner", "repo", test_issue.clone())
                .with_comments("owner", "repo", 123, vec![]);

            // Create storage dir with different content (simulating local changes)
            fs::create_dir_all(test_dir.path()).unwrap();
            fs::write(
                test_dir.path().join("issue.md"),
                "Old content that will be overwritten\n",
            )
            .unwrap();

            let storage = IssueStorage::from_dir(test_dir.path());
            let args = RefreshArgs {
                issue: IssueArgs {
                    issue_number: 123,
                    repo: Some("owner/repo".to_string()),
                },
            };

            // Unlike pull, refresh should succeed even with local changes
            let result = run_with_client_and_storage(&args, &client, &storage).await;
            assert!(result.is_ok());

            // Verify content was overwritten
            let body = fs::read_to_string(test_dir.path().join("issue.md")).unwrap();
            assert_eq!(body, "Test body content\n");
        }

        #[rstest]
        #[tokio::test]
        async fn test_succeeds_when_dir_does_not_exist(test_dir: TempDir, test_issue: Issue) {
            let client = MockGitHubClient::new()
                .with_issue("owner", "repo", test_issue.clone())
                .with_comments("owner", "repo", 123, vec![]);

            // Use a non-existent subdirectory
            let storage = IssueStorage::from_dir(test_dir.path().join("new_dir"));
            let args = RefreshArgs {
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
            let args = RefreshArgs {
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
            let args = RefreshArgs {
                issue: IssueArgs {
                    issue_number: 123,
                    repo: Some("invalid-repo-format".to_string()),
                },
            };

            let result = run_with_client_and_storage(&args, &client, &storage).await;
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
        async fn test_overwrites_comments(test_dir: TempDir, test_issue: Issue) {
            let new_comment = Comment {
                id: "IC_new".to_string(),
                database_id: 99999,
                author: Some(Author {
                    login: "newuser".to_string(),
                }),
                created_at: Utc.with_ymd_and_hms(2024, 2, 1, 0, 0, 0).unwrap(),
                body: "New comment from refresh".to_string(),
            };

            let client = MockGitHubClient::new()
                .with_issue("owner", "repo", test_issue.clone())
                .with_comments("owner", "repo", 123, vec![new_comment]);

            // Create storage dir with old comments
            let comments_dir = test_dir.path().join("comments");
            fs::create_dir_all(&comments_dir).unwrap();
            fs::write(comments_dir.join("001_comment_11111.md"), "Old comment").unwrap();

            let storage = IssueStorage::from_dir(test_dir.path());
            let args = RefreshArgs {
                issue: IssueArgs {
                    issue_number: 123,
                    repo: Some("owner/repo".to_string()),
                },
            };

            run_with_client_and_storage(&args, &client, &storage)
                .await
                .unwrap();

            // Verify new comment file exists
            assert!(comments_dir.join("001_comment_99999.md").exists());
            let content = fs::read_to_string(comments_dir.join("001_comment_99999.md")).unwrap();
            assert!(content.contains("New comment from refresh"));
        }
    }
}
