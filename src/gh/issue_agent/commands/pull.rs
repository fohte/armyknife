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
    let repo = get_repo(&args.issue.repo)?;
    let issue_number = args.issue.issue_number;

    eprintln!("Fetching issue #{issue_number} from {repo}...");

    let (owner, repo_name) = parse_repo(&repo)?;
    let client = OctocrabClient::get()?;

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
