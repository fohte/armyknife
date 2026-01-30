//! Diff command for gh-issue-agent.
//!
//! Shows the diff between local changes and remote GitHub issue state.

use clap::Args;

use super::common::{get_repo_from_arg_or_git, parse_repo};
use super::push::changeset::{ChangeSet, DetectOptions, LocalState, RemoteState};
use super::push::detect::check_remote_unchanged;
use crate::commands::gh::issue_agent::storage::IssueStorage;
use crate::infra::github::OctocrabClient;

#[derive(Args, Clone, PartialEq, Eq, Debug)]
pub struct DiffArgs {
    #[command(flatten)]
    pub issue: super::IssueArgs,
}

pub async fn run(args: &DiffArgs) -> anyhow::Result<()> {
    let repo = get_repo_from_arg_or_git(&args.issue.repo)?;
    let issue_number = args.issue.issue_number;

    // Validate repo format before making any API calls
    parse_repo(&repo)?;

    let storage = IssueStorage::new(&repo, issue_number as i64);

    // Check if local cache exists
    if !storage.dir().exists() {
        anyhow::bail!(
            "Issue #{} not found locally. Run 'a gh issue-agent pull {}' first.",
            issue_number,
            issue_number
        );
    }

    println!("Fetching latest from GitHub...");

    // Fetch latest from GitHub
    let client = OctocrabClient::get()?;
    let current_user = client.get_current_user().await?;

    run_with_client_and_user(args, client, &storage, &current_user).await
}

async fn run_with_client_and_user(
    args: &DiffArgs,
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

    // Warn if remote has changed since pull (but don't error, just show diff)
    let remote_updated_at = remote_issue.updated_at.to_rfc3339();
    if check_remote_unchanged(&local_metadata.updated_at, &remote_updated_at, false).is_err() {
        eprintln!();
        eprintln!("Warning: Remote has been updated since last pull.");
        eprintln!("Consider running 'a gh issue-agent pull --force' to update local copy.");
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
        // For diff display, show all changes including other users' comments
        edit_others: true,
        allow_delete: true,
    };
    let changeset = ChangeSet::detect(&local, &remote, &options)?;

    // Display changes
    if changeset.has_changes() {
        changeset.display()?;
    } else {
        println!();
        println!("No changes detected.");
    }

    Ok(())
}
