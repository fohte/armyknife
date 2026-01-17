use std::process::Command;

use clap::Args;

use crate::gh::issue_agent::models::IssueMetadata;
use crate::gh::issue_agent::storage::IssueStorage;
use crate::github::{CommentClient, IssueClient, OctocrabClient};

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
    let repo = get_repo(&args.issue.repo)?;
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
    do_fetch_issue(&storage, &issue, &comments)?;

    // Print success message
    print_success_message(issue_number, &issue.title, storage.dir());

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
    let repo = get_repo(&args.issue.repo)?;
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
    do_fetch_issue(storage, &issue, &comments)?;

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
        if owner.is_empty() || repo_name.is_empty() {
            return Err(format!("Invalid repository format: {repo}. Expected owner/repo").into());
        }
        Ok((owner.to_string(), repo_name.to_string()))
    } else {
        Err(format!("Invalid repository format: {repo}. Expected owner/repo").into())
    }
}

/// Save issue data to local storage.
pub(super) fn do_fetch_issue(
    storage: &IssueStorage,
    issue: &crate::gh::issue_agent::models::Issue,
    comments: &[crate::gh::issue_agent::models::Comment],
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

    // Mock client for testing
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
        #[case::with_numbers("org123/repo456", ("org123", "repo456"))]
        #[case::with_dots("org.name/repo.name", ("org.name", "repo.name"))]
        fn test_parse_repo_valid(#[case] input: &str, #[case] expected: (&str, &str)) {
            let result = parse_repo(input).unwrap();
            assert_eq!(result, (expected.0.to_string(), expected.1.to_string()));
        }

        #[rstest]
        #[case::no_slash("ownerrepo")]
        #[case::empty("")]
        #[case::only_slash("/")]
        #[case::empty_owner("/repo")]
        #[case::empty_repo("owner/")]
        fn test_parse_repo_invalid(#[case] input: &str) {
            let result = parse_repo(input);
            assert!(result.is_err());
            assert!(
                result
                    .unwrap_err()
                    .to_string()
                    .contains("Invalid repository format")
            );
        }

        #[test]
        fn test_parse_repo_with_multiple_slashes() {
            // split_once splits at first occurrence, so "a/b/c" -> ("a", "b/c")
            let result = parse_repo("org/repo/extra").unwrap();
            assert_eq!(result, ("org".to_string(), "repo/extra".to_string()));
        }
    }

    mod get_repo_tests {
        use super::*;

        #[test]
        fn test_get_repo_with_explicit_arg() {
            let result = get_repo(&Some("owner/repo".to_string())).unwrap();
            assert_eq!(result, "owner/repo");
        }

        // Note: Testing get_repo with None requires a controlled environment
        // (either a git repo or not). Since this is environment-dependent,
        // we only test the explicit arg case. The None case is covered by
        // integration tests in the actual CLI.
    }

    mod do_fetch_issue_tests {
        use super::*;

        #[rstest]
        fn test_saves_issue_body(test_dir: TempDir, test_issue: Issue) {
            let storage = IssueStorage::from_dir(test_dir.path());
            do_fetch_issue(&storage, &test_issue, &[]).unwrap();

            let body = fs::read_to_string(test_dir.path().join("issue.md")).unwrap();
            assert_eq!(body, "Test body content\n");
        }

        #[rstest]
        fn test_saves_empty_body_when_none(test_dir: TempDir, mut test_issue: Issue) {
            test_issue.body = None;
            let storage = IssueStorage::from_dir(test_dir.path());
            do_fetch_issue(&storage, &test_issue, &[]).unwrap();

            let body = fs::read_to_string(test_dir.path().join("issue.md")).unwrap();
            assert_eq!(body, "\n");
        }

        #[rstest]
        fn test_saves_metadata(test_dir: TempDir, test_issue: Issue) {
            let storage = IssueStorage::from_dir(test_dir.path());
            do_fetch_issue(&storage, &test_issue, &[]).unwrap();

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
            do_fetch_issue(&storage, &test_issue, &[test_comment]).unwrap();

            let comments_dir = test_dir.path().join("comments");
            assert!(comments_dir.exists());

            let comment_file = comments_dir.join("001_comment_12345.md");
            assert!(comment_file.exists());

            let content = fs::read_to_string(&comment_file).unwrap();
            assert!(content.contains("Test comment body"));
            assert!(content.contains("<!-- author: commenter -->"));
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
            do_fetch_issue(&storage, &test_issue, &comments).unwrap();

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
            let client = MockClient::new()
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
            let client = MockClient::new()
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
            assert!(
                result
                    .unwrap_err()
                    .to_string()
                    .contains("Local changes would be overwritten")
            );
        }

        #[rstest]
        #[tokio::test]
        async fn test_succeeds_when_no_local_changes(test_dir: TempDir, test_issue: Issue) {
            let client = MockClient::new()
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
            let client = MockClient::new()
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
            let client = MockClient::new(); // No issues configured

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
            let client = MockClient::new().with_issue("owner", "repo", test_issue);

            let storage = IssueStorage::from_dir(test_dir.path());
            let args = PullArgs {
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
    }
}
