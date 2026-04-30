//! Push command for gh-issue-agent.

pub(crate) mod changeset;
mod create;
pub(crate) mod detect;
#[cfg(test)]
mod integration_tests;
mod links;

use std::path::PathBuf;

use clap::Args;

use super::common;
use super::common::IssueContext;
use crate::commands::gh::issue_agent::models::IssueFrontmatter;
use crate::infra::github::GitHubClient;
use crate::shared::human_in_the_loop::ApprovalManager;

use changeset::{ChangeSet, CommentChange, DetectOptions, LocalState, RemoteState};
use detect::{ConflictCheckInput, check_conflicts};

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
        // Reject directories that contain an existing issue (pulled from GitHub).
        // Users should use the issue number directly instead of the directory path.
        if let Some(issue_number) = read_issue_number_from_dir(&path) {
            anyhow::bail!(
                "Directory '{}' contains existing issue #{}. Use the issue number instead:\n  \
                 a gh issue-agent push {}",
                path.display(),
                issue_number,
                issue_number
            );
        }
        return Ok(PushTarget::NewIssuePath(path));
    }

    anyhow::bail!(
        "Invalid target: '{}' is neither a valid issue number nor an existing directory",
        target
    )
}

/// Try to read the issue number from issue.md frontmatter in a directory.
/// Returns Some(issue_number) if the directory contains an existing issue.
fn read_issue_number_from_dir(dir: &std::path::Path) -> Option<u64> {
    let storage = crate::commands::gh::issue_agent::storage::IssueStorage::from_dir(dir);
    let metadata = storage.read_metadata().ok()?;
    u64::try_from(metadata.number).ok()
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
    client: &GitHubClient,
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
    client: &GitHubClient,
) -> anyhow::Result<()> {
    let remote = ctx.fetch_remote(client).await?;
    let local = ctx.load_local()?;

    // Check for field-level conflicts unless --force is specified
    if !args.force {
        let conflict_input = ConflictCheckInput {
            local_metadata: &local.metadata,
            local_body: &local.body,
            local_comments: &local.comments,
            remote_issue: &remote.issue,
            remote_comments: &remote.comments,
        };
        let conflicts = check_conflicts(&conflict_input);

        if !conflicts.is_empty() {
            eprintln!();
            eprintln!(
                "Conflict detected! The following fields were edited both locally and remotely:"
            );
            for conflict in &conflicts {
                eprintln!("  - {}", conflict);
            }
            eprintln!();
            anyhow::bail!(
                "Remote has changed. Use --force to overwrite, or 'pull --force' to update local copy."
            );
        }
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

    // Verify approval before applying (skip for dry-run)
    if !args.dry_run && has_changes {
        verify_changeset_approval(&changeset, ctx)?;
    }

    if !args.dry_run && has_changes {
        changeset
            .apply(
                client,
                &ctx.owner,
                &ctx.repo_name,
                ctx.issue_number,
                &ctx.storage,
                &remote.issue,
            )
            .await?;

        // Update local issue.md with new frontmatter from remote after successful push
        let new_remote_issue = common::fetch_issue_with_sub_issues(
            client,
            &ctx.owner,
            &ctx.repo_name,
            ctx.issue_number,
        )
        .await?;
        let new_frontmatter = IssueFrontmatter::from_issue(&new_remote_issue);
        let body = new_remote_issue.body.as_deref().unwrap_or("");
        ctx.storage.save_issue(&new_frontmatter, body)?;

        // Clean up .approve files after successful push
        cleanup_approve_files(&changeset, ctx);
    }

    // Show result
    print_result(args.dry_run, has_changes);

    Ok(())
}

/// Collect file paths that require approval for the given changeset.
///
/// Deletions are excluded because the file no longer exists locally.
fn collect_approval_paths(changeset: &ChangeSet<'_>, ctx: &IssueContext) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    let dir = ctx.storage.dir();

    // Both body and metadata (title, labels, sub-issues, parent-issue) are
    // stored in issue.md's YAML frontmatter, so any change requires issue.md approval.
    let issue_md_changed = changeset.body.is_some()
        || changeset.title.is_some()
        || changeset.labels.is_some()
        || changeset.sub_issues.is_some()
        || changeset.parent_issue.is_some();
    if issue_md_changed {
        paths.push(dir.join("issue.md"));
    }

    for change in &changeset.comments {
        match change {
            CommentChange::New { filename, .. } | CommentChange::Updated { filename, .. } => {
                paths.push(dir.join("comments").join(filename));
            }
            CommentChange::Deleted { .. } => {
                // Deletions don't have a local file to approve
            }
        }
    }

    paths
}

/// Verify that all changed files have been approved via `a gh issue-agent review`.
fn verify_changeset_approval(changeset: &ChangeSet<'_>, ctx: &IssueContext) -> anyhow::Result<()> {
    let paths = collect_approval_paths(changeset, ctx);
    let mut unapproved = Vec::new();

    for path in &paths {
        let manager = ApprovalManager::new(path);
        if let Err(e) = manager.verify() {
            use crate::shared::human_in_the_loop::HumanInTheLoopError;
            match e {
                HumanInTheLoopError::NotApproved | HumanInTheLoopError::ModifiedAfterApproval => {
                    unapproved.push(path.display().to_string());
                }
                other => return Err(other.into()),
            }
        }
    }

    if !unapproved.is_empty() {
        let files = unapproved.join("\n  ");
        anyhow::bail!(
            "The following files have not been approved. Run 'a gh issue-agent review <file>' for each:\n  {}",
            files
        );
    }

    Ok(())
}

/// Remove .approve files for changed files after successful push.
fn cleanup_approve_files(changeset: &ChangeSet<'_>, ctx: &IssueContext) {
    let paths = collect_approval_paths(changeset, ctx);
    for path in &paths {
        let manager = ApprovalManager::new(path);
        let _ = manager.remove();
    }
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
        fn test_parse_path_without_issue_md_is_new() {
            let temp_dir = TempDir::new().unwrap();
            let path = temp_dir.path().to_string_lossy().to_string();

            let result = parse_target(&path).unwrap();
            assert!(matches!(result, PushTarget::NewIssuePath(_)));
        }

        #[rstest]
        fn test_parse_path_with_existing_issue_frontmatter_errors() {
            let temp_dir = TempDir::new().unwrap();
            let issue_md = indoc::indoc! {r#"
                ---
                title: Test Issue
                labels: []
                assignees: []
                milestone: null
                readonly:
                  number: 42
                  state: OPEN
                  author: testuser
                  createdAt: "2024-01-01T00:00:00Z"
                  updatedAt: "2024-01-02T00:00:00Z"
                ---

                Body
            "#};
            std::fs::write(temp_dir.path().join("issue.md"), issue_md).unwrap();
            let path = temp_dir.path().to_string_lossy().to_string();

            let err = parse_target(&path).unwrap_err();
            let expected = format!(
                indoc::indoc! {"
                    Directory '{}' contains existing issue #42. Use the issue number instead:
                      a gh issue-agent push 42"},
                temp_dir.path().display()
            );
            assert_eq!(err.to_string(), expected);
        }

        #[rstest]
        fn test_parse_path_with_new_issue_frontmatter() {
            // issue.md with NewIssueFrontmatter (no readonly.number) should be NewIssuePath
            let temp_dir = TempDir::new().unwrap();
            let issue_md = indoc::indoc! {"
                ---
                title: New Issue
                labels: [bug]
                assignees: []
                ---

                Body
            "};
            std::fs::write(temp_dir.path().join("issue.md"), issue_md).unwrap();
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
