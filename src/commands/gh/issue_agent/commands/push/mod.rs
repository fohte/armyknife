//! Push command for gh-issue-agent.

pub(crate) mod changeset;
pub(crate) mod detect;
#[cfg(test)]
mod integration_tests;

use clap::Args;

use super::common::{get_repo_from_arg_or_git, parse_repo};
use crate::commands::gh::issue_agent::models::IssueMetadata;
use crate::commands::gh::issue_agent::storage::IssueStorage;
use crate::infra::github::OctocrabClient;

use changeset::{ChangeSet, DetectOptions, LocalState, RemoteState};
use detect::check_remote_unchanged;

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

    /// Allow deleting comments from GitHub
    #[arg(long)]
    pub allow_delete: bool,
}

pub async fn run(args: &PushArgs) -> anyhow::Result<()> {
    let repo = get_repo_from_arg_or_git(&args.issue.repo)?;
    let issue_number = args.issue.issue_number;

    // Validate repo format before making any API calls
    parse_repo(&repo)?;

    let storage = IssueStorage::new(&repo, issue_number as i64);

    // 1. Check if local cache exists
    if !storage.dir().exists() {
        anyhow::bail!(
            "Issue #{} not found locally. Run 'a gh issue-agent pull {}' first.",
            issue_number,
            issue_number
        );
    }

    println!("Fetching latest from GitHub...");

    // 2. Fetch latest from GitHub
    let client = OctocrabClient::get()?;
    let current_user = client.get_current_user().await?;

    run_with_client_and_user(args, client, &storage, &current_user).await
}

/// Internal implementation that accepts a client and user for testability.
#[cfg(test)]
pub(super) async fn run_with_client_and_storage(
    args: &PushArgs,
    client: &OctocrabClient,
    storage: &IssueStorage,
    current_user: &str,
) -> anyhow::Result<()> {
    run_with_client_and_user(args, client, storage, current_user).await
}

async fn run_with_client_and_user(
    args: &PushArgs,
    client: &OctocrabClient,
    storage: &IssueStorage,
    current_user: &str,
) -> anyhow::Result<()> {
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
        anyhow::bail!(
            "Remote has changed. Use --force to overwrite, or 'pull --force' to update local copy."
        );
    }

    // Detect all changes
    let local = LocalState {
        metadata: &local_metadata,
        body: &local_body,
        comments: &local_comments,
    };
    let remote = RemoteState {
        issue: &remote_issue,
        comments: &remote_comments,
    };
    let options = DetectOptions {
        current_user,
        edit_others: args.edit_others,
        allow_delete: args.allow_delete,
    };
    let changeset = ChangeSet::detect(&local, &remote, &options)?;

    // Display and apply changes
    let has_changes = changeset.has_changes();
    changeset.display()?;

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
