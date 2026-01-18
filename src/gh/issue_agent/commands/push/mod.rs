//! Push command for gh-issue-agent.

#[cfg(test)]
mod tests;

use std::collections::{HashMap, HashSet};

use clap::Args;
use similar::{ChangeTag, TextDiff};

use super::common::{get_repo_from_arg_or_git, parse_repo};
use crate::gh::issue_agent::models::{Comment, Issue, IssueMetadata};
use crate::gh::issue_agent::storage::{IssueStorage, LocalComment};
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
    let repo = get_repo_from_arg_or_git(&args.issue.repo)?;
    let issue_number = args.issue.issue_number;

    // Validate repo format before making any API calls
    parse_repo(&repo)?;

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
    let current_user = get_current_user(client).await?;

    run_with_client_and_user(args, client, &storage, &current_user).await
}

/// Internal implementation that accepts a client and user for testability.
#[cfg(test)]
pub(super) async fn run_with_client_and_storage<C>(
    args: &PushArgs,
    client: &C,
    storage: &IssueStorage,
    current_user: &str,
) -> Result<(), Box<dyn std::error::Error>>
where
    C: IssueClient + CommentClient,
{
    run_with_client_and_user(args, client, storage, current_user).await
}

async fn run_with_client_and_user<C>(
    args: &PushArgs,
    client: &C,
    storage: &IssueStorage,
    current_user: &str,
) -> Result<(), Box<dyn std::error::Error>>
where
    C: IssueClient + CommentClient,
{
    let repo = get_repo_from_arg_or_git(&args.issue.repo)?;
    let issue_number = args.issue.issue_number;
    let (owner, repo_name) = parse_repo(&repo)?;

    // Fetch remote state
    let remote_issue = client.get_issue(&owner, &repo_name, issue_number).await?;
    let remote_comments = client
        .get_comments(&owner, &repo_name, issue_number)
        .await?;

    // Load local state
    let local_metadata = storage.read_metadata()?;
    let local_body = storage.read_body()?;
    let local_comments = storage.read_comments()?;

    // Check if remote has changed since pull
    let remote_updated_at = remote_issue.updated_at.to_rfc3339();
    if let Err(msg) =
        check_remote_unchanged(&local_metadata.updated_at, &remote_updated_at, args.force)
    {
        eprintln!();
        eprintln!("{}", msg);
        eprintln!();
        return Err(
            "Remote has changed. Use --force to overwrite, or 'refresh' to update local copy."
                .into(),
        );
    }

    // Detect all changes
    let changeset = ChangeSet::detect(
        &local_metadata,
        &local_body,
        &local_comments,
        &remote_issue,
        &remote_comments,
        current_user,
        args.edit_others,
    )?;

    // Display and apply changes
    let has_changes = changeset.has_changes();
    changeset.display();

    if !args.dry_run && has_changes {
        changeset
            .apply(client, &owner, &repo_name, issue_number, storage)
            .await?;

        // Update local metadata to match remote after successful push
        let new_remote_issue = client.get_issue(&owner, &repo_name, issue_number).await?;
        let new_metadata = IssueMetadata::from_issue(&new_remote_issue);
        storage.save_metadata(&new_metadata)?;
    }

    // Show result
    print_result(args.dry_run, has_changes);

    Ok(())
}

// =============================================================================
// ChangeSet: Represents all detected changes
// =============================================================================

/// Represents all changes detected between local and remote state.
struct ChangeSet<'a> {
    body: Option<BodyChange<'a>>,
    title: Option<TitleChange<'a>>,
    labels: Option<LabelChange>,
    comments: Vec<CommentChange<'a>>,
}

struct BodyChange<'a> {
    local: &'a str,
    remote: &'a str,
}

struct TitleChange<'a> {
    local: &'a str,
    remote: &'a str,
}

struct LabelChange {
    to_add: Vec<String>,
    to_remove: Vec<String>,
    local_sorted: Vec<String>,
    remote_sorted: Vec<String>,
}

enum CommentChange<'a> {
    New {
        filename: &'a str,
        body: &'a str,
    },
    Updated {
        filename: &'a str,
        local_body: &'a str,
        remote_body: &'a str,
        database_id: i64,
        author: &'a str,
        current_user: &'a str,
    },
}

impl<'a> ChangeSet<'a> {
    fn detect(
        local_metadata: &'a IssueMetadata,
        local_body: &'a str,
        local_comments: &'a [LocalComment],
        remote_issue: &'a Issue,
        remote_comments: &'a [Comment],
        current_user: &'a str,
        edit_others: bool,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let body = detect_body_change(local_body, remote_issue);
        let title = detect_title_change(local_metadata, remote_issue);
        let labels = detect_label_change(local_metadata, remote_issue);
        let comments =
            detect_comment_changes(local_comments, remote_comments, current_user, edit_others)?;

        Ok(Self {
            body,
            title,
            labels,
            comments,
        })
    }

    fn has_changes(&self) -> bool {
        self.body.is_some()
            || self.title.is_some()
            || self.labels.is_some()
            || !self.comments.is_empty()
    }

    fn display(&self) {
        if let Some(change) = &self.body {
            println!();
            println!("=== Issue Body ===");
            print_diff(change.remote, change.local);
        }

        if let Some(change) = &self.title {
            println!();
            println!("=== Title ===");
            println!("- {}", change.remote);
            println!("+ {}", change.local);
        }

        if let Some(change) = &self.labels {
            println!();
            println!("=== Labels ===");
            println!("- {:?}", change.remote_sorted);
            println!("+ {:?}", change.local_sorted);
        }

        for change in &self.comments {
            match change {
                CommentChange::New { filename, body } => {
                    println!();
                    println!("=== New Comment: {} ===", filename);
                    println!("{}", body);
                }
                CommentChange::Updated {
                    filename,
                    local_body,
                    remote_body,
                    author,
                    current_user,
                    ..
                } => {
                    println!();
                    if author != current_user {
                        println!("=== Comment: {} (author: {}) ===", filename, author);
                    } else {
                        println!("=== Comment: {} ===", filename);
                    }
                    print_diff(remote_body, local_body);
                }
            }
        }
    }

    async fn apply<C>(
        &self,
        client: &C,
        owner: &str,
        repo: &str,
        issue_number: u64,
        storage: &IssueStorage,
    ) -> Result<(), Box<dyn std::error::Error>>
    where
        C: IssueClient + CommentClient,
    {
        if let Some(change) = &self.body {
            println!();
            println!("Updating issue body...");
            client
                .update_issue_body(owner, repo, issue_number, change.local)
                .await?;
        }

        if let Some(change) = &self.title {
            println!();
            println!("Updating title...");
            client
                .update_issue_title(owner, repo, issue_number, change.local)
                .await?;
        }

        if let Some(change) = &self.labels {
            println!();
            println!("Updating labels...");
            for label in &change.to_remove {
                client
                    .remove_label(owner, repo, issue_number, label)
                    .await?;
            }
            if !change.to_add.is_empty() {
                client
                    .add_labels(owner, repo, issue_number, &change.to_add)
                    .await?;
            }
        }

        for change in &self.comments {
            match change {
                CommentChange::New { filename, body } => {
                    println!();
                    println!("Creating comment...");
                    client
                        .create_comment(owner, repo, issue_number, body)
                        .await?;
                    // Remove the new comment file after successful creation
                    let comment_path = storage.dir().join("comments").join(filename);
                    std::fs::remove_file(&comment_path)?;
                }
                CommentChange::Updated {
                    database_id,
                    local_body,
                    ..
                } => {
                    println!();
                    println!("Updating comment...");
                    client
                        .update_comment(owner, repo, *database_id as u64, local_body)
                        .await?;
                }
            }
        }

        Ok(())
    }
}

// =============================================================================
// Change detection functions
// =============================================================================

fn detect_body_change<'a>(local_body: &'a str, remote_issue: &'a Issue) -> Option<BodyChange<'a>> {
    let remote_body = remote_issue.body.as_deref().unwrap_or("");
    if local_body != remote_body {
        Some(BodyChange {
            local: local_body,
            remote: remote_body,
        })
    } else {
        None
    }
}

fn detect_title_change<'a>(
    local_metadata: &'a IssueMetadata,
    remote_issue: &'a Issue,
) -> Option<TitleChange<'a>> {
    if local_metadata.title != remote_issue.title {
        Some(TitleChange {
            local: &local_metadata.title,
            remote: &remote_issue.title,
        })
    } else {
        None
    }
}

fn detect_label_change(
    local_metadata: &IssueMetadata,
    remote_issue: &Issue,
) -> Option<LabelChange> {
    let remote_labels: HashSet<&str> = remote_issue
        .labels
        .iter()
        .map(|l| l.name.as_str())
        .collect();
    let local_labels: HashSet<&str> = local_metadata.labels.iter().map(|s| s.as_str()).collect();

    if remote_labels == local_labels {
        return None;
    }

    // Compute to_add and to_remove as owned Strings
    let to_remove: Vec<String> = remote_labels
        .difference(&local_labels)
        .map(|s| s.to_string())
        .collect();
    let to_add: Vec<String> = local_labels
        .difference(&remote_labels)
        .map(|s| s.to_string())
        .collect();

    let mut remote_sorted: Vec<String> = remote_labels.into_iter().map(|s| s.to_string()).collect();
    remote_sorted.sort();
    let mut local_sorted: Vec<String> = local_labels.into_iter().map(|s| s.to_string()).collect();
    local_sorted.sort();

    Some(LabelChange {
        to_add,
        to_remove,
        local_sorted,
        remote_sorted,
    })
}

fn detect_comment_changes<'a>(
    local_comments: &'a [LocalComment],
    remote_comments: &'a [Comment],
    current_user: &'a str,
    edit_others: bool,
) -> Result<Vec<CommentChange<'a>>, Box<dyn std::error::Error>> {
    let remote_comments_map: HashMap<&str, &Comment> =
        remote_comments.iter().map(|c| (c.id.as_str(), c)).collect();

    let mut changes = Vec::new();

    for local_comment in local_comments {
        if local_comment.is_new() {
            changes.push(CommentChange::New {
                filename: &local_comment.filename,
                body: &local_comment.body,
            });
            continue;
        }

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
        check_can_edit_comment(author, current_user, edit_others, &local_comment.filename)?;

        let database_id = local_comment
            .metadata
            .database_id
            .ok_or("Comment missing databaseId")?;

        changes.push(CommentChange::Updated {
            filename: &local_comment.filename,
            local_body: &local_comment.body,
            remote_body: &remote_comment.body,
            database_id,
            author,
            current_user,
        });
    }

    Ok(changes)
}

fn print_result(dry_run: bool, has_changes: bool) {
    println!();
    if dry_run {
        if has_changes {
            println!("[dry-run] Changes detected. Run without --dry-run to apply.");
        } else {
            println!("[dry-run] No changes detected.");
        }
    } else if has_changes {
        println!("Done! Changes have been pushed to GitHub.");
    } else {
        println!("No changes to push.");
    }
}

/// Get current GitHub user from the API.
async fn get_current_user(client: &OctocrabClient) -> Result<String, Box<dyn std::error::Error>> {
    let user = client.client.current().user().await?;
    Ok(user.login)
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

/// Check if remote has changed since the last pull.
/// Returns Ok(()) if no conflict, Err with message if changed.
fn check_remote_unchanged(
    local_updated_at: &str,
    remote_updated_at: &str,
    force: bool,
) -> Result<(), String> {
    if force || local_updated_at == remote_updated_at {
        Ok(())
    } else {
        Err(format!(
            "Remote has changed since pull. Local: {}, Remote: {}",
            local_updated_at, remote_updated_at
        ))
    }
}

/// Check if the user can edit a comment.
/// Returns Ok(()) if allowed, Err with message if not.
fn check_can_edit_comment(
    comment_author: &str,
    current_user: &str,
    edit_others: bool,
    filename: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    if comment_author == current_user || edit_others {
        Ok(())
    } else {
        Err(format!(
            "Cannot edit other user's comment: {} (author: {}). Use --edit-others to allow.",
            filename, comment_author
        )
        .into())
    }
}

/// Format diff as a string (for testing).
#[cfg(test)]
fn format_diff(old: &str, new: &str) -> String {
    let diff = TextDiff::from_lines(old, new);
    let mut result = String::new();

    for change in diff.iter_all_changes() {
        let sign = match change.tag() {
            ChangeTag::Delete => "-",
            ChangeTag::Insert => "+",
            ChangeTag::Equal => " ",
        };
        result.push_str(sign);
        result.push_str(&change.to_string());
    }
    result
}
