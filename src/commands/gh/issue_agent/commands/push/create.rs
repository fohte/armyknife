//! Create new issue from local directory.

use std::path::{Path, PathBuf};

use super::PushArgs;
use crate::commands::gh::issue_agent::models::{IssueMetadata, NewIssue};
use crate::commands::gh::issue_agent::storage::IssueStorage;
use crate::infra::git::parse_repo;
use crate::infra::github::OctocrabClient;

/// Run create for new issue.
pub async fn run_create(args: &PushArgs, path: PathBuf) -> anyhow::Result<()> {
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

    let client = OctocrabClient::get()?;
    let created = client
        .create_issue(
            &owner,
            &repo_name,
            &new_issue.title,
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

    std::fs::rename(&path, &new_dir)?;

    // Update issue.md to remove frontmatter (keep only body like existing issues)
    let storage = IssueStorage::from_dir(&new_dir);
    storage.save_body(&new_issue.body)?;

    // Save metadata
    let metadata = IssueMetadata::from_issue(&created);
    storage.save_metadata(&metadata)?;

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
    println!("Title: {}", issue.title);
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
    use rstest::rstest;

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
}
