use std::process::Command;

use clap::Args;

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
    let repo = get_repo(&args.issue.repo)?;
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
    super::pull::do_fetch_issue(&storage, &issue, &comments)?;

    // Print success message
    print_success_message(issue_number, &issue.title, storage.dir());

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
    let repo = get_repo(&args.issue.repo)?;
    let issue_number = args.issue.issue_number;

    let (owner, repo_name) = parse_repo(&repo)?;

    // Fetch issue and comments from GitHub
    let issue = client.get_issue(&owner, &repo_name, issue_number).await?;
    let comments = client
        .get_comments(&owner, &repo_name, issue_number)
        .await?;

    // Save to local storage (overwriting any local changes - no change check)
    super::pull::do_fetch_issue(storage, &issue, &comments)?;

    Ok(())
}

/// Get repository from argument or current directory.
fn get_repo(repo_arg: &Option<String>) -> Result<String, Box<dyn std::error::Error>> {
    if let Some(repo) = repo_arg {
        return Ok(repo.clone());
    }

    // Use `gh repo view` to get current repository
    let output = Command::new("gh")
        .args([
            "repo",
            "view",
            "--json",
            "nameWithOwner",
            "--jq",
            ".nameWithOwner",
        ])
        .output()
        .map_err(|e| format!("Failed to run gh repo view: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("gh repo view failed: {stderr}").into());
    }

    let repo = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if repo.is_empty() {
        return Err("Could not determine repository. Use -R to specify.".into());
    }

    Ok(repo)
}

/// Parse "owner/repo" into (owner, repo) tuple.
fn parse_repo(repo: &str) -> Result<(String, String), Box<dyn std::error::Error>> {
    if let Some((owner, repo_name)) = repo.split_once('/') {
        Ok((owner.to_string(), repo_name.to_string()))
    } else {
        Err(format!("Invalid repository format: {repo}. Expected owner/repo").into())
    }
}

/// Print success message after fetching issue.
fn print_success_message(issue_number: u64, title: &str, dir: &std::path::Path) {
    eprintln!();
    eprintln!(
        "Done! Issue #{issue_number} has been saved to {}/",
        dir.display()
    );
    eprintln!();
    eprintln!("Title: {title}");
    eprintln!();
    eprintln!("Files:");
    eprintln!(
        "  {}/issue.md          - Issue body (editable)",
        dir.display()
    );
    eprintln!(
        "  {}/metadata.json     - Metadata (editable: title, labels, assignees)",
        dir.display()
    );
    eprintln!(
        "  {}/comments/         - Comments (only your own comments are editable)",
        dir.display()
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gh::issue_agent::commands::IssueArgs;
    use crate::gh::issue_agent::models::{Author, Comment, Issue, Label};
    use chrono::{TimeZone, Utc};
    use rstest::{fixture, rstest};
    use std::collections::HashMap;
    use std::fs;
    use std::sync::Mutex;
    use tempfile::TempDir;

    type GitHubResult<T> = std::result::Result<T, crate::github::GitHubError>;

    // Mock client for testing (same as in pull.rs)
    struct MockClient {
        issues: Mutex<HashMap<String, Issue>>,
        comments: Mutex<HashMap<String, Vec<Comment>>>,
    }

    impl MockClient {
        fn new() -> Self {
            Self {
                issues: Mutex::new(HashMap::new()),
                comments: Mutex::new(HashMap::new()),
            }
        }

        fn with_issue(self, owner: &str, repo: &str, issue: Issue) -> Self {
            let key = format!("{owner}/{repo}/{}", issue.number);
            self.issues.lock().unwrap().insert(key, issue);
            self
        }

        fn with_comments(
            self,
            owner: &str,
            repo: &str,
            issue_number: i64,
            comments: Vec<Comment>,
        ) -> Self {
            let key = format!("{owner}/{repo}/{issue_number}");
            self.comments.lock().unwrap().insert(key, comments);
            self
        }
    }

    #[async_trait::async_trait]
    impl IssueClient for MockClient {
        async fn get_issue(
            &self,
            owner: &str,
            repo: &str,
            issue_number: u64,
        ) -> GitHubResult<Issue> {
            let key = format!("{owner}/{repo}/{issue_number}");
            self.issues
                .lock()
                .unwrap()
                .get(&key)
                .cloned()
                .ok_or_else(|| {
                    crate::github::GitHubError::TokenError(format!("Issue {key} not found"))
                })
        }

        async fn update_issue_body(
            &self,
            _owner: &str,
            _repo: &str,
            _issue_number: u64,
            _body: &str,
        ) -> GitHubResult<()> {
            Ok(())
        }

        async fn update_issue_title(
            &self,
            _owner: &str,
            _repo: &str,
            _issue_number: u64,
            _title: &str,
        ) -> GitHubResult<()> {
            Ok(())
        }

        async fn add_labels(
            &self,
            _owner: &str,
            _repo: &str,
            _issue_number: u64,
            _labels: &[String],
        ) -> GitHubResult<()> {
            Ok(())
        }

        async fn remove_label(
            &self,
            _owner: &str,
            _repo: &str,
            _issue_number: u64,
            _label: &str,
        ) -> GitHubResult<()> {
            Ok(())
        }
    }

    #[async_trait::async_trait]
    impl CommentClient for MockClient {
        async fn get_comments(
            &self,
            owner: &str,
            repo: &str,
            issue_number: u64,
        ) -> GitHubResult<Vec<Comment>> {
            let key = format!("{owner}/{repo}/{issue_number}");
            Ok(self
                .comments
                .lock()
                .unwrap()
                .get(&key)
                .cloned()
                .unwrap_or_default())
        }

        async fn update_comment(
            &self,
            _owner: &str,
            _repo: &str,
            _comment_id: u64,
            _body: &str,
        ) -> GitHubResult<()> {
            Ok(())
        }

        async fn create_comment(
            &self,
            _owner: &str,
            _repo: &str,
            _issue_number: u64,
            body: &str,
        ) -> GitHubResult<Comment> {
            Ok(Comment {
                id: "IC_new".to_string(),
                database_id: 99999,
                author: Some(Author {
                    login: "testuser".to_string(),
                }),
                created_at: Utc::now(),
                body: body.to_string(),
            })
        }
    }

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

    mod parse_repo_tests {
        use super::*;

        #[rstest]
        #[case::valid("owner/repo", ("owner", "repo"))]
        #[case::with_dashes("my-org/my-repo", ("my-org", "my-repo"))]
        fn test_parse_repo_valid(#[case] input: &str, #[case] expected: (&str, &str)) {
            let result = parse_repo(input).unwrap();
            assert_eq!(result, (expected.0.to_string(), expected.1.to_string()));
        }

        #[rstest]
        #[case::no_slash("ownerrepo")]
        #[case::empty("")]
        fn test_parse_repo_invalid(#[case] input: &str) {
            let result = parse_repo(input);
            assert!(result.is_err());
        }
    }

    mod get_repo_tests {
        use super::*;

        #[test]
        fn test_get_repo_with_explicit_arg() {
            let result = get_repo(&Some("owner/repo".to_string())).unwrap();
            assert_eq!(result, "owner/repo");
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
            let client = MockClient::new()
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
            let client = MockClient::new()
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
            let client = MockClient::new()
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
            let client = MockClient::new(); // No issues configured

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
            let client = MockClient::new().with_issue("owner", "repo", test_issue);

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

            let client = MockClient::new()
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
