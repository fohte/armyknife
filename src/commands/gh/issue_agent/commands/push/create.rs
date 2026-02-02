//! Create new issue from local directory.

use std::path::{Path, PathBuf};

use super::PushArgs;
use crate::commands::gh::issue_agent::models::{IssueFrontmatter, NewIssue};
use crate::commands::gh::issue_agent::storage::IssueStorage;
use crate::infra::git::parse_repo;
use crate::infra::github::OctocrabClient;

/// Run create for new issue.
pub async fn run_create(args: &PushArgs, path: PathBuf) -> anyhow::Result<()> {
    let client = OctocrabClient::get()?;
    run_create_with_client(args, path, client).await
}

/// Run create with an injected client (for testing).
pub async fn run_create_with_client(
    args: &PushArgs,
    path: PathBuf,
    client: impl std::ops::Deref<Target = OctocrabClient>,
) -> anyhow::Result<()> {
    let client = &*client;
    // Get repo from path or -R option
    let repo = get_repo_from_path_or_arg(&path, &args.repo)?;
    let (owner, repo_name) = parse_repo(&repo)?;

    // Read and parse issue.md
    let issue_md_path = path.join("issue.md");
    if !issue_md_path.exists() {
        anyhow::bail!(
            "issue.md not found in {}. Create it with frontmatter and content.",
            path.display()
        );
    }

    let content = std::fs::read_to_string(&issue_md_path)?;
    let new_issue =
        NewIssue::parse(&content).map_err(|e| anyhow::anyhow!("Failed to parse issue.md: {e}"))?;

    // Display what will be created
    display_new_issue(&new_issue);

    if args.dry_run {
        println!();
        println!("[dry-run] Would create issue. Run without --dry-run to create.");
        return Ok(());
    }

    // Create issue on GitHub
    println!();
    println!("Creating issue on GitHub...");

    let created = client
        .create_issue(
            &owner,
            &repo_name,
            new_issue.title(),
            &new_issue.body,
            &new_issue.frontmatter.labels,
            &new_issue.frontmatter.assignees,
        )
        .await?;

    let issue_number = created.number;

    // Rename directory from new/ to {issue_number}/
    let new_dir = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("Cannot determine parent directory"))?
        .join(issue_number.to_string());

    // Check if destination already exists before rename
    if new_dir.exists() {
        anyhow::bail!(
            "Issue #{issue_number} created on GitHub, but directory '{}' already exists locally. \
             Remove or rename it, then run 'pull {issue_number}' to fetch the issue.",
            new_dir.display()
        );
    }

    if let Err(e) = std::fs::rename(&path, &new_dir) {
        anyhow::bail!(
            "Issue #{issue_number} created on GitHub, but failed to rename local directory: {e}. \
             Run 'pull {issue_number}' to fetch it locally."
        );
    }

    // Save issue.md with frontmatter (same format as pull)
    let storage = IssueStorage::from_dir(&new_dir);
    let frontmatter = IssueFrontmatter::from_issue(&created);
    if let Err(e) = storage.save_issue(&frontmatter, &new_issue.body) {
        // Directory was renamed successfully, so inform user about partial state
        anyhow::bail!(
            "Issue #{issue_number} created and directory renamed, but failed to save metadata: {e}. \
             Run 'pull --force {issue_number}' to refresh local state."
        );
    }

    // Success message
    println!();
    println!("Done! Created issue #{issue_number}");
    println!();
    println!("Local files moved to: {}/", new_dir.display());
    println!(
        "View on GitHub: https://github.com/{owner}/{repo_name}/issues/{issue_number}",
        owner = owner,
        repo_name = repo_name,
        issue_number = issue_number
    );

    Ok(())
}

/// Get repository from path structure or -R argument.
fn get_repo_from_path_or_arg(path: &Path, repo_arg: &Option<String>) -> anyhow::Result<String> {
    if let Some(repo) = repo_arg {
        return Ok(repo.clone());
    }

    // Try to extract owner/repo from path structure:
    // Expected: .../{owner}/{repo}/new/
    if path.file_name().is_some_and(|name| name == "new")
        && let Some(repo_dir) = path.parent()
        && let (Some(repo_name), Some(owner_name)) = (
            repo_dir.file_name(),
            repo_dir.parent().and_then(|p| p.file_name()),
        )
    {
        let repo = repo_name.to_string_lossy();
        let owner = owner_name.to_string_lossy();
        return Ok(format!("{}/{}", owner, repo));
    }

    anyhow::bail!(
        "Cannot determine repository from path '{}'. Use -R owner/repo to specify.",
        path.display()
    )
}

/// Display the new issue that will be created.
fn display_new_issue(issue: &NewIssue) {
    println!("=== New Issue ===");
    println!();
    println!("Title: {}", issue.title());
    println!();

    if !issue.frontmatter.labels.is_empty() {
        println!("Labels: {}", issue.frontmatter.labels.join(", "));
    }
    if !issue.frontmatter.assignees.is_empty() {
        println!("Assignees: {}", issue.frontmatter.assignees.join(", "));
    }

    if !issue.frontmatter.labels.is_empty() || !issue.frontmatter.assignees.is_empty() {
        println!();
    }

    println!("Body:");
    println!("---");
    if issue.body.is_empty() {
        println!("(empty)");
    } else {
        println!("{}", issue.body);
    }
    println!("---");
}

#[cfg(test)]
mod tests {
    use super::*;
    use indoc::indoc;
    use rstest::rstest;
    use tempfile::TempDir;

    mod run_create_tests {
        use super::*;
        use crate::infra::github::GitHubMockServer;

        fn make_args(repo: Option<String>, dry_run: bool) -> PushArgs {
            PushArgs {
                target: String::new(), // not used in run_create_with_client
                repo,
                dry_run,
                force: false,
                edit_others: false,
                allow_delete: false,
            }
        }

        fn setup_new_issue_dir(base: &Path) -> PathBuf {
            let new_dir = base.join("owner").join("repo").join("new");
            std::fs::create_dir_all(&new_dir).unwrap();
            new_dir
        }

        fn write_issue_md(dir: &Path, content: &str) {
            std::fs::write(dir.join("issue.md"), content).unwrap();
        }

        #[rstest]
        #[tokio::test]
        async fn test_create_issue_success() {
            let temp = TempDir::new().unwrap();
            let new_dir = setup_new_issue_dir(temp.path());
            write_issue_md(
                &new_dir,
                indoc! {"
                    ---
                    title: Test Issue
                    labels: [bug]
                    assignees: [testuser]
                    ---

                    This is the body.
                "},
            );

            let mock = GitHubMockServer::start().await;
            mock.repo("owner", "repo")
                .issue(42)
                .title("Test Issue")
                .body("This is the body.\n")
                .labels(vec!["bug"])
                .create()
                .await;

            let client = mock.client();
            let args = make_args(None, false);
            let result = run_create_with_client(&args, new_dir.clone(), &client).await;

            assert!(result.is_ok());

            // Verify directory was renamed
            let renamed_dir = temp.path().join("owner").join("repo").join("42");
            assert!(renamed_dir.exists());
            assert!(!new_dir.exists());

            // Verify issue.md with frontmatter was saved
            let issue_md = std::fs::read_to_string(renamed_dir.join("issue.md")).unwrap();
            assert!(issue_md.contains("title: Test Issue"));
            assert!(issue_md.contains("readonly:"));
            assert!(issue_md.contains("number: 42"));
        }

        #[rstest]
        #[tokio::test]
        async fn test_create_issue_dry_run() {
            let temp = TempDir::new().unwrap();
            let new_dir = setup_new_issue_dir(temp.path());
            write_issue_md(
                &new_dir,
                indoc! {"
                    ---
                    title: Dry Run Test
                    ---

                    Body content.
                "},
            );

            let mock = GitHubMockServer::start().await;
            // No mock setup needed - API should not be called in dry run

            let client = mock.client();
            let args = make_args(None, true);
            let result = run_create_with_client(&args, new_dir.clone(), &client).await;

            assert!(result.is_ok());

            // Verify directory was NOT renamed
            assert!(new_dir.exists());
        }

        #[rstest]
        #[tokio::test]
        async fn test_create_issue_missing_issue_md() {
            let temp = TempDir::new().unwrap();
            let new_dir = setup_new_issue_dir(temp.path());
            // Don't create issue.md

            let mock = GitHubMockServer::start().await;
            let client = mock.client();
            let args = make_args(None, false);
            let result = run_create_with_client(&args, new_dir, &client).await;

            assert!(result.is_err());
            assert!(
                result
                    .unwrap_err()
                    .to_string()
                    .contains("issue.md not found")
            );
        }

        #[rstest]
        #[tokio::test]
        async fn test_create_issue_invalid_format() {
            let temp = TempDir::new().unwrap();
            let new_dir = setup_new_issue_dir(temp.path());
            write_issue_md(&new_dir, "No title here, just body text.");

            let mock = GitHubMockServer::start().await;
            let client = mock.client();
            let args = make_args(None, false);
            let result = run_create_with_client(&args, new_dir, &client).await;

            assert!(result.is_err());
            assert!(
                result
                    .unwrap_err()
                    .to_string()
                    .contains("Failed to parse issue.md")
            );
        }

        #[rstest]
        #[tokio::test]
        async fn test_create_issue_with_repo_arg() {
            let temp = TempDir::new().unwrap();
            // Create dir without owner/repo structure
            let new_dir = temp.path().join("new");
            std::fs::create_dir_all(&new_dir).unwrap();
            write_issue_md(
                &new_dir,
                indoc! {"
                    ---
                    title: With Repo Arg
                    ---

                    Body.
                "},
            );

            let mock = GitHubMockServer::start().await;
            mock.repo("custom", "repo")
                .issue(99)
                .title("With Repo Arg")
                .body("Body.\n")
                .labels(vec![])
                .create()
                .await;

            let client = mock.client();
            let args = make_args(Some("custom/repo".to_string()), false);
            let result = run_create_with_client(&args, new_dir.clone(), &client).await;

            assert!(result.is_ok());

            // Verify directory was renamed
            let renamed_dir = temp.path().join("99");
            assert!(renamed_dir.exists());
        }

        #[rstest]
        #[tokio::test]
        async fn test_create_issue_cannot_determine_repo() {
            let temp = TempDir::new().unwrap();
            // Create dir without owner/repo/new structure and no -R arg
            let some_dir = temp.path().join("random");
            std::fs::create_dir_all(&some_dir).unwrap();
            write_issue_md(
                &some_dir,
                indoc! {"
                    ---
                    title: Test
                    ---

                    Body.
                "},
            );

            let mock = GitHubMockServer::start().await;
            let client = mock.client();
            let args = make_args(None, false);
            let result = run_create_with_client(&args, some_dir, &client).await;

            assert!(result.is_err());
            assert!(
                result
                    .unwrap_err()
                    .to_string()
                    .contains("Cannot determine repository")
            );
        }
    }

    mod get_repo_from_path_tests {
        use super::*;

        #[rstest]
        fn test_with_repo_arg() {
            let path = PathBuf::from("/some/random/path");
            let repo_arg = Some("owner/repo".to_string());

            let result = get_repo_from_path_or_arg(&path, &repo_arg).unwrap();
            assert_eq!(result, "owner/repo");
        }

        #[rstest]
        fn test_extract_from_new_path() {
            let path = PathBuf::from("/home/user/.cache/gh-issue-agent/fohte/armyknife/new");
            let result = get_repo_from_path_or_arg(&path, &None).unwrap();
            assert_eq!(result, "fohte/armyknife");
        }

        #[rstest]
        fn test_extract_from_relative_new_path() {
            let path = PathBuf::from("owner/repo/new");
            let result = get_repo_from_path_or_arg(&path, &None).unwrap();
            assert_eq!(result, "owner/repo");
        }

        #[rstest]
        fn test_fail_without_new_suffix() {
            let path = PathBuf::from("/home/user/.cache/gh-issue-agent/fohte/armyknife/123");
            let result = get_repo_from_path_or_arg(&path, &None);
            assert!(result.is_err());
        }

        #[rstest]
        fn test_fail_too_short_path() {
            let path = PathBuf::from("new");
            let result = get_repo_from_path_or_arg(&path, &None);
            assert!(result.is_err());
        }
    }

    mod display_new_issue_tests {
        use super::*;
        use crate::commands::gh::issue_agent::models::NewIssueFrontmatter;

        #[rstest]
        fn test_display_with_labels_and_assignees() {
            let issue = NewIssue {
                frontmatter: NewIssueFrontmatter {
                    title: "Test Title".to_string(),
                    labels: vec!["bug".to_string(), "urgent".to_string()],
                    assignees: vec!["user1".to_string()],
                },
                body: "Test body content.".to_string(),
            };

            // Just verify it doesn't panic
            display_new_issue(&issue);
        }

        #[rstest]
        fn test_display_with_empty_body() {
            let issue = NewIssue {
                frontmatter: NewIssueFrontmatter {
                    title: "Title Only".to_string(),
                    ..Default::default()
                },
                body: String::new(),
            };

            // Just verify it doesn't panic
            display_new_issue(&issue);
        }

        #[rstest]
        fn test_display_minimal() {
            let issue = NewIssue {
                frontmatter: NewIssueFrontmatter {
                    title: "Minimal".to_string(),
                    ..Default::default()
                },
                body: "Body".to_string(),
            };

            // Just verify it doesn't panic
            display_new_issue(&issue);
        }
    }
}
