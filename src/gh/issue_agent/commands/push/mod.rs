//! Push command for gh-issue-agent.

mod changeset;
mod detect;
mod diff;
#[cfg(test)]
mod integration_tests;

use clap::Args;

use super::common::{get_repo_from_arg_or_git, parse_repo};
use crate::gh::issue_agent::models::IssueMetadata;
use crate::gh::issue_agent::storage::IssueStorage;
use crate::github::{CommentClient, IssueClient, OctocrabClient};

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
