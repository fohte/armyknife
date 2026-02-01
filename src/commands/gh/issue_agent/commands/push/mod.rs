//! Push command for gh-issue-agent.

pub(crate) mod changeset;
pub(crate) mod detect;
#[cfg(test)]
mod integration_tests;

use clap::Args;

use super::common::IssueContext;
use crate::commands::gh::issue_agent::models::IssueMetadata;
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
    let (ctx, client) = IssueContext::from_args(&args.issue).await?;
    run_with_context(args, &ctx, client).await
}

/// Internal implementation that accepts a client and storage for testability.
#[cfg(test)]
pub(super) async fn run_with_client_and_storage(
    args: &PushArgs,
    client: &OctocrabClient,
    storage: &crate::commands::gh::issue_agent::storage::IssueStorage,
    current_user: &str,
) -> anyhow::Result<()> {
    use super::common::{get_repo_from_arg_or_git, parse_repo};

    let repo = get_repo_from_arg_or_git(&args.issue.repo)?;
    let (owner, repo_name) = parse_repo(&repo)?;

    let ctx = IssueContext {
        owner: owner.to_string(),
        repo_name: repo_name.to_string(),
        issue_number: args.issue.issue_number,
        storage: storage.clone(),
        current_user: current_user.to_string(),
    };
    run_with_context(args, &ctx, client).await
}

async fn run_with_context(
    args: &PushArgs,
    ctx: &IssueContext,
    client: &OctocrabClient,
) -> anyhow::Result<()> {
    let remote = ctx.fetch_remote(client).await?;
    let local = ctx.load_local()?;

    // Check if remote has changed since pull
    let remote_updated_at = remote.issue.updated_at.to_rfc3339();
    if let Err(msg) =
        check_remote_unchanged(&local.metadata.updated_at, &remote_updated_at, args.force)
    {
        eprintln!();
        eprintln!("{}", msg);
        eprintln!();
        anyhow::bail!(
            "Remote has changed. Use --force to overwrite, or 'pull --force' to update local copy."
        );
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
        edit_others: args.edit_others,
        allow_delete: args.allow_delete,
    };
    let changeset = ChangeSet::detect(&local_state, &remote_state, &options)?;

    // Display and apply changes
    let has_changes = changeset.has_changes();
    changeset.display()?;

    if !args.dry_run && has_changes {
        changeset
            .apply(
                client,
                &ctx.owner,
                &ctx.repo_name,
                ctx.issue_number,
                &ctx.storage,
            )
            .await?;

        // Update local metadata to match remote after successful push
        let new_remote_issue = client
            .get_issue(&ctx.owner, &ctx.repo_name, ctx.issue_number)
            .await?;
        let new_metadata = IssueMetadata::from_issue(&new_remote_issue);
        ctx.storage.save_metadata(&new_metadata)?;
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
