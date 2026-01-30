//! Diff command for gh-issue-agent.
//!
//! Shows the diff between local changes and remote GitHub issue state.

use clap::Args;

use super::common::IssueContext;
use super::push::changeset::{ChangeSet, DetectOptions, LocalState, RemoteState};
use super::push::detect::check_remote_unchanged;

#[derive(Args, Clone, PartialEq, Eq, Debug)]
pub struct DiffArgs {
    #[command(flatten)]
    pub issue: super::IssueArgs,
}

pub async fn run(args: &DiffArgs) -> anyhow::Result<()> {
    let (ctx, client) = IssueContext::from_args(&args.issue).await?;

    let remote = ctx.fetch_remote(client).await?;
    let local = ctx.load_local()?;

    // Warn if remote has changed since pull (but don't error, just show diff)
    let remote_updated_at = remote.issue.updated_at.to_rfc3339();
    if check_remote_unchanged(&local.metadata.updated_at, &remote_updated_at, false).is_err() {
        eprintln!();
        eprintln!("Warning: Remote has been updated since last pull.");
        eprintln!("Consider running 'a gh issue-agent pull --force' to update local copy.");
    }

    // Detect all changes
    let local_state = LocalState {
        metadata: &local.metadata,
        body: &local.body,
        comments: &local.comments,
    };
    let remote_state = RemoteState {
        issue: &remote.issue,
        comments: &remote.comments,
    };
    let options = DetectOptions {
        current_user: &ctx.current_user,
        // For diff display, show all changes including other users' comments
        edit_others: true,
        allow_delete: true,
    };
    let changeset = ChangeSet::detect(&local_state, &remote_state, &options)?;

    // Display changes
    if changeset.has_changes() {
        changeset.display()?;
    } else {
        println!();
        println!("No changes detected.");
    }

    Ok(())
}
