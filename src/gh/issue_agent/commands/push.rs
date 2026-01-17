use std::collections::{HashMap, HashSet};
use std::process::Command;

use clap::Args;
use similar::{ChangeTag, TextDiff};

use crate::gh::issue_agent::models::{Comment, IssueMetadata};
use crate::gh::issue_agent::storage::IssueStorage;
use crate::github::{CommentClient, IssueClient, OctocrabClient};

#[derive(Args, Clone, PartialEq, Eq, Debug)]
pub struct PushArgs {
    #[command(flatten)]
    pub issue: super::IssueArgs,

    /// Show what would be changed without applying
    #[arg(long)]
    pub dry_run: bool,

    /// Allow overwriting remote changes (like git push --force)
    #[arg(long)]
    pub force: bool,

    /// Allow editing other users' comments
    #[arg(long)]
    pub edit_others: bool,
}

pub async fn run(args: &PushArgs) -> Result<(), Box<dyn std::error::Error>> {
    let repo = get_repo(&args.issue.repo)?;
    let issue_number = args.issue.issue_number;

    let storage = IssueStorage::new(&repo, issue_number as i64);

    // 1. Check if local cache exists
    if !storage.dir().exists() {
        return Err(format!(
            "Issue #{} not found locally. Run 'a gh issue-agent pull {}' first.",
            issue_number, issue_number
        )
        .into());
    }

    println!("Fetching latest from GitHub...");

    // 2. Fetch latest from GitHub
    let client = OctocrabClient::get()?;
    let (owner, repo_name) = parse_repo(&repo)?;

    let remote_issue = client.get_issue(owner, repo_name, issue_number).await?;
    let remote_comments = client.get_comments(owner, repo_name, issue_number).await?;

    // 3. Check if remote has changed since pull
    let local_metadata = storage.read_metadata()?;
    if !args.force && local_metadata.updated_at != remote_issue.updated_at.to_rfc3339() {
        eprintln!();
        eprintln!("Remote has changed since pull:");
        eprintln!("  Local:  {}", local_metadata.updated_at);
        eprintln!("  Remote: {}", remote_issue.updated_at.to_rfc3339());
        eprintln!();
        return Err(
            "Remote has changed. Use --force to overwrite, or 'refresh' to update local copy."
                .into(),
        );
    }

    // 4. Detect and display changes
    let current_user = get_current_user()?;
    let mut has_changes = false;

    // Compare issue body
    let local_body = storage.read_body().unwrap_or_default();
    let remote_body = remote_issue.body.as_deref().unwrap_or("");
    if local_body != remote_body {
        println!();
        println!("=== Issue Body ===");
        print_diff(remote_body, &local_body);
        if !args.dry_run {
            println!();
            println!("Updating issue body...");
            client
                .update_issue_body(owner, repo_name, issue_number, &local_body)
                .await?;
        }
        has_changes = true;
    }

    // Compare title
    if local_metadata.title != remote_issue.title {
        println!();
        println!("=== Title ===");
        println!("- {}", remote_issue.title);
        println!("+ {}", local_metadata.title);
        if !args.dry_run {
            println!();
            println!("Updating title...");
            client
                .update_issue_title(owner, repo_name, issue_number, &local_metadata.title)
                .await?;
        }
        has_changes = true;
    }

    // Compare labels
    let remote_labels: HashSet<&str> = remote_issue
        .labels
        .iter()
        .map(|l| l.name.as_str())
        .collect();
    let local_labels: HashSet<&str> = local_metadata.labels.iter().map(|s| s.as_str()).collect();

    if remote_labels != local_labels {
        println!();
        println!("=== Labels ===");
        let mut remote_sorted: Vec<_> = remote_labels.iter().collect();
        remote_sorted.sort();
        let mut local_sorted: Vec<_> = local_labels.iter().collect();
        local_sorted.sort();
        println!("- {:?}", remote_sorted);
        println!("+ {:?}", local_sorted);

        if !args.dry_run {
            println!();
            println!("Updating labels...");

            for label in remote_labels.difference(&local_labels) {
                client
                    .remove_label(owner, repo_name, issue_number, label)
                    .await?;
            }

            let labels_to_add: Vec<String> = local_labels
                .difference(&remote_labels)
                .map(|s| s.to_string())
                .collect();

            if !labels_to_add.is_empty() {
                client
                    .add_labels(owner, repo_name, issue_number, &labels_to_add)
                    .await?;
            }
        }
        has_changes = true;
    }

    // Compare comments
    let local_comments = storage.read_comments().unwrap_or_default();
    let remote_comments_map: HashMap<&str, &Comment> =
        remote_comments.iter().map(|c| (c.id.as_str(), c)).collect();

    for local_comment in &local_comments {
        if local_comment.is_new() {
            // New comment
            println!();
            println!("=== New Comment: {} ===", local_comment.filename);
            println!("{}", local_comment.body);

            if !args.dry_run {
                println!();
                println!("Creating comment...");
                client
                    .create_comment(owner, repo_name, issue_number, &local_comment.body)
                    .await?;

                // Remove the new comment file after successful creation
                let comment_path = storage.dir().join("comments").join(&local_comment.filename);
                std::fs::remove_file(&comment_path)?;
            }
            has_changes = true;
        } else {
            // Existing comment
            let Some(comment_id) = &local_comment.metadata.id else {
                continue;
            };

            let Some(remote_comment) = remote_comments_map.get(comment_id.as_str()) else {
                continue;
            };

            if local_comment.body == remote_comment.body {
                continue;
            }

            let author = local_comment
                .metadata
                .author
                .as_deref()
                .unwrap_or("unknown");

            // Check if editing other user's comment
            if author != current_user && !args.edit_others {
                return Err(format!(
                    "Cannot edit other user's comment: {} (author: {}). Use --edit-others to allow.",
                    local_comment.filename, author
                )
                .into());
            }

            println!();
            if author != current_user {
                println!(
                    "=== Comment: {} (author: {}) ===",
                    local_comment.filename, author
                );
            } else {
                println!("=== Comment: {} ===", local_comment.filename);
            }
            print_diff(&remote_comment.body, &local_comment.body);

            if !args.dry_run {
                println!();
                println!("Updating comment...");
                let database_id = local_comment
                    .metadata
                    .database_id
                    .ok_or("Comment missing databaseId")?;
                client
                    .update_comment(owner, repo_name, database_id as u64, &local_comment.body)
                    .await?;
            }
            has_changes = true;
        }
    }

    // 5-6. Show result
    println!();
    if args.dry_run {
        if has_changes {
            println!("[dry-run] Changes detected. Run without --dry-run to apply.");
        } else {
            println!("[dry-run] No changes detected.");
        }
    } else if has_changes {
        // Update local updatedAt to match remote after successful push
        let new_remote_issue = client.get_issue(owner, repo_name, issue_number).await?;
        let new_metadata = IssueMetadata::from_issue(&new_remote_issue);
        storage.save_metadata(&new_metadata)?;
        println!("Done! Changes have been pushed to GitHub.");
    } else {
        println!("No changes to push.");
    }

    Ok(())
}

/// Get repository from argument or current directory.
fn get_repo(repo_arg: &Option<String>) -> Result<String, Box<dyn std::error::Error>> {
    if let Some(repo) = repo_arg {
        return Ok(repo.clone());
    }

    // Use `gh repo view` to get current repo
    let output = Command::new("gh")
        .args([
            "repo",
            "view",
            "--json",
            "nameWithOwner",
            "--jq",
            ".nameWithOwner",
        ])
        .output()?;

    if !output.status.success() {
        return Err("Failed to determine current repository. Use -R to specify.".into());
    }

    let repo = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if repo.is_empty() {
        return Err("Failed to determine current repository. Use -R to specify.".into());
    }

    Ok(repo)
}

/// Parse owner/repo string into (owner, repo) tuple.
fn parse_repo(repo: &str) -> Result<(&str, &str), Box<dyn std::error::Error>> {
    repo.split_once('/')
        .ok_or_else(|| format!("Invalid repository format: {}. Expected owner/repo.", repo).into())
}

/// Get current GitHub user from `gh api user`.
fn get_current_user() -> Result<String, Box<dyn std::error::Error>> {
    let output = Command::new("gh")
        .args(["api", "user", "--jq", ".login"])
        .output()?;

    if !output.status.success() {
        return Err("Failed to get current GitHub user".into());
    }

    let user = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Ok(user)
}

/// Print unified diff between old and new text.
fn print_diff(old: &str, new: &str) {
    let diff = TextDiff::from_lines(old, new);

    for change in diff.iter_all_changes() {
        let sign = match change.tag() {
            ChangeTag::Delete => "-",
            ChangeTag::Insert => "+",
            ChangeTag::Equal => " ",
        };
        // change already includes newline, so use print! instead of println!
        print!("{}{}", sign, change);
    }
}
