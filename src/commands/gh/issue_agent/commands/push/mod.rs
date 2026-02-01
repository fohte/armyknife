//! Push command for gh-issue-agent.

pub(crate) mod changeset;
mod create;
pub(crate) mod detect;
#[cfg(test)]
mod integration_tests;

use std::path::PathBuf;

use clap::Args;

use super::common::IssueContext;
use crate::commands::gh::issue_agent::models::IssueMetadata;
use crate::infra::github::OctocrabClient;

use changeset::{ChangeSet, DetectOptions, LocalState, RemoteState};
use detect::check_remote_unchanged;

/// Target for push command: either an issue number or a path to new issue directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PushTarget {
    IssueNumber(u64),
    NewIssuePath(PathBuf),
}

#[derive(Args, Clone, PartialEq, Eq, Debug)]
pub struct PushArgs {
    /// Issue number or path to new issue directory
    pub target: String,

    /// Target repository (owner/repo)
    #[arg(short = 'R', long)]
    pub repo: Option<String>,

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

/// Parse target string into PushTarget.
fn parse_target(target: &str) -> anyhow::Result<PushTarget> {
    // Try to parse as issue number first
    if let Ok(n) = target.parse::<u64>() {
        return Ok(PushTarget::IssueNumber(n));
    }

    // Try as a path
    let path = PathBuf::from(target);
    if path.exists() && path.is_dir() {
        return Ok(PushTarget::NewIssuePath(path));
    }

    anyhow::bail!(
        "Invalid target: '{}' is neither a valid issue number nor an existing directory",
        target
    )
}

pub async fn run(args: &PushArgs) -> anyhow::Result<()> {
    let target = parse_target(&args.target)?;

    match target {
        PushTarget::IssueNumber(issue_number) => run_update(args, issue_number).await,
        PushTarget::NewIssuePath(path) => create::run_create(args, path).await,
    }
}

/// Run update for existing issue (original behavior).
async fn run_update(args: &PushArgs, issue_number: u64) -> anyhow::Result<()> {
    let issue_args = super::IssueArgs {
        issue_number,
        repo: args.repo.clone(),
    };
    let (ctx, client) = IssueContext::from_args(&issue_args).await?;
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

    // Parse target to get issue number (for backward compatibility in tests)
    let issue_number = match parse_target(&args.target)? {
        PushTarget::IssueNumber(n) => n,
        PushTarget::NewIssuePath(_) => {
            anyhow::bail!("run_with_client_and_storage does not support new issue creation")
        }
    };

    let repo = get_repo_from_arg_or_git(&args.repo)?;
    let (owner, repo_name) = parse_repo(&repo)?;

    let ctx = IssueContext {
        owner: owner.to_string(),
        repo_name: repo_name.to_string(),
        issue_number,
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

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;
    use tempfile::TempDir;

    mod parse_target_tests {
        use super::*;

        #[rstest]
        #[case("123", PushTarget::IssueNumber(123))]
        #[case("1", PushTarget::IssueNumber(1))]
        #[case("999999", PushTarget::IssueNumber(999999))]
        fn test_parse_issue_number(#[case] input: &str, #[case] expected: PushTarget) {
            let result = parse_target(input).unwrap();
            assert_eq!(result, expected);
        }

        #[rstest]
        fn test_parse_path() {
            let temp_dir = TempDir::new().unwrap();
            let path = temp_dir.path().to_string_lossy().to_string();

            let result = parse_target(&path).unwrap();
            assert!(matches!(result, PushTarget::NewIssuePath(_)));
        }

        #[rstest]
        fn test_parse_invalid() {
            let result = parse_target("/nonexistent/path/to/nowhere");
            assert!(result.is_err());
            assert!(result.unwrap_err().to_string().contains("Invalid target"));
        }

        #[rstest]
        fn test_parse_negative_number() {
            // Negative numbers are not valid issue numbers
            let result = parse_target("-1");
            assert!(result.is_err());
        }
    }
}
