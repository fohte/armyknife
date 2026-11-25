//! Push command for gh-issue-agent.

pub(crate) mod changeset;
mod create;
pub(crate) mod detect;
mod field_comparison;
#[cfg(test)]
mod integration_tests;
mod links;

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};

use clap::Args;

use super::common;
use super::common::IssueContext;
use crate::commands::gh::issue_agent::body_compare::comment_bodies_equal;
use crate::commands::gh::issue_agent::models::{Comment, IssueFrontmatter, IssueMetadata};
use crate::commands::gh::issue_agent::storage::{CommentSnapshot, LocalComment};
use crate::infra::github::GitHubClient;
use crate::shared::human_in_the_loop::{
    ApprovalManager, ReviewDocumentAction, successful_action_result,
    successful_action_results_in_directory,
};

use changeset::{ChangeSet, CommentChange, DetectOptions, IssueAddress, LocalState, RemoteState};
use detect::{ConflictCheckInput, check_conflicts};
use field_comparison::compare_issue_fields;

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

#[derive(Debug, Clone)]
pub(crate) enum ReviewDocumentPushScope {
    Comment {
        path: PathBuf,
        retry_started_at: DateTime<Utc>,
    },
    IssueBody {
        path: PathBuf,
    },
}

pub(crate) async fn push_review_document(
    issue_number: u64,
    repo: String,
    scope: ReviewDocumentPushScope,
) -> anyhow::Result<()> {
    let args = PushArgs {
        target: issue_number.to_string(),
        repo: Some(repo),
        dry_run: false,
        force: false,
        edit_others: false,
        allow_delete: false,
    };
    let issue_args = super::IssueArgs {
        issue_number,
        repo: args.repo.clone(),
    };
    let (ctx, client) = IssueContext::from_args(&issue_args).await?;
    run_with_context(&args, &ctx, client, Some(scope)).await
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
    run_with_context(args, &ctx, client, None).await
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
    run_with_context(args, &ctx, client, None).await
}

async fn run_with_context(
    args: &PushArgs,
    ctx: &IssueContext,
    client: &GitHubClient,
    scope: Option<ReviewDocumentPushScope>,
) -> anyhow::Result<()> {
    let remote = ctx.fetch_remote(client).await?;
    let mut local = ctx.load_local()?;
    if let Some(ReviewDocumentPushScope::Comment { path, .. }) = &scope {
        let selected = local.comments.iter().any(|comment| {
            paths_match(
                &ctx.storage.dir().join("comments").join(&comment.filename),
                path,
            )
        });
        if !selected {
            anyhow::bail!("The selected review comment is unavailable");
        }
        local.comments.retain(|comment| {
            paths_match(
                &ctx.storage.dir().join("comments").join(&comment.filename),
                path,
            )
        });
    }
    if let Some(ReviewDocumentPushScope::IssueBody { path }) = &scope
        && !paths_match(&ctx.storage.dir().join("issue.md"), path)
    {
        anyhow::bail!("The selected issue document is unavailable");
    }
    if matches!(scope, Some(ReviewDocumentPushScope::IssueBody { .. })) {
        local.comments.clear();
    }
    let field_comparisons = local.base_issue.as_ref().map(|base_issue| {
        compare_issue_fields(&local.metadata, &local.body, &remote.issue, base_issue)
    });

    // Check for field-level conflicts unless --force is specified
    if !args.force {
        let conflicts = if matches!(scope, Some(ReviewDocumentPushScope::Comment { .. })) {
            let remote_metadata = IssueMetadata::from(IssueFrontmatter::from_issue(&remote.issue));
            let remote_body = remote.issue.body.as_deref().unwrap_or("");
            let conflict_input = ConflictCheckInput {
                local_metadata: &remote_metadata,
                local_body: remote_body,
                local_comments: &local.comments,
                base_comments: local.base_comments.as_deref(),
                remote_issue: &remote.issue,
                remote_comments: &remote.comments,
            };
            check_conflicts(&conflict_input, None)
        } else {
            let conflict_input = ConflictCheckInput {
                local_metadata: &local.metadata,
                local_body: &local.body,
                local_comments: &local.comments,
                base_comments: local.base_comments.as_deref(),
                remote_issue: &remote.issue,
                remote_comments: &remote.comments,
            };
            check_conflicts(&conflict_input, field_comparisons)
        };
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
        base_comments: local.base_comments.as_deref(),
    };
    let remote_state = RemoteState {
        issue: &remote.issue,
        comments: &remote.comments,
    };
    let options = DetectOptions {
        current_user: &ctx.current_user,
        edit_others: args.edit_others || args.dry_run,
        allow_delete: args.allow_delete
            || args.dry_run
            || matches!(scope, Some(ReviewDocumentPushScope::Comment { .. }))
            || matches!(scope, Some(ReviewDocumentPushScope::IssueBody { .. })),
        retry_started_at: match &scope {
            Some(ReviewDocumentPushScope::Comment {
                retry_started_at, ..
            }) => Some(*retry_started_at),
            _ => None,
        },
    };
    let mut changeset = ChangeSet::detect_with_comparisons(
        &local_state,
        &remote_state,
        &options,
        field_comparisons,
    )?;
    restrict_changeset_to_review_document(
        &mut changeset,
        &scope,
        &ctx.storage.dir().join("comments"),
    );

    // Display and apply changes
    let has_changes = changeset.has_changes();
    changeset.display()?;

    let reported_watch_web_action = !args.dry_run
        && report_completed_watch_web_actions(
            &ctx.storage.dir().join("issue.md"),
            &ctx.storage.dir().join("comments"),
        )?;
    if reported_watch_web_action && !has_changes {
        return Ok(());
    }

    // Verify approval before applying (skip for dry-run)
    if !args.dry_run && has_changes {
        verify_changeset_approval(&changeset, ctx)?;
    }

    if !args.dry_run && has_changes {
        changeset
            .apply(
                client,
                IssueAddress {
                    owner: &ctx.owner,
                    repo: &ctx.repo_name,
                    number: ctx.issue_number,
                },
                &ctx.storage,
                &remote.issue,
                scope.is_none(),
            )
            .await?;

        if !matches!(scope, Some(ReviewDocumentPushScope::Comment { .. })) {
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
        }

        if scope.is_none() {
            // Refresh comment files so their IDs and update timestamps match GitHub.
            let new_remote_comments = client
                .get_comments(&ctx.owner, &ctx.repo_name, ctx.issue_number)
                .await?;
            ctx.storage.save_comments(&new_remote_comments)?;
        }

        // Clean up approval records after successful push
        cleanup_approval_records(&changeset, ctx);
    } else if scope.is_none()
        && !args.dry_run
        && field_comparisons.is_some_and(|comparison| comparison.all_sides_equal())
    {
        let frontmatter = IssueFrontmatter::from_issue(&remote.issue);
        let body = remote.issue.body.as_deref().unwrap_or("");
        ctx.storage.save_base_issue(&frontmatter, body)?;
    }

    if !args.dry_run
        && let Some(ReviewDocumentPushScope::Comment {
            path,
            retry_started_at,
        }) = &scope
    {
        reconcile_review_comment_push(ctx, client, &local, &changeset, path, *retry_started_at)
            .await?;
    }

    if scope.is_none()
        && !args.dry_run
        && !has_changes
        && let Some(base_comments) = local.base_comments.as_deref()
    {
        let advanced = advance_comment_snapshot(
            base_comments,
            &local.comments,
            &remote.comments,
            &changeset.comments,
        );
        if advanced.as_slice() != base_comments {
            ctx.storage.save_base_comments(&advanced)?;
        }
    }

    // Show result
    print_result(args.dry_run, has_changes);

    Ok(())
}

async fn reconcile_review_comment_push(
    ctx: &IssueContext,
    client: &GitHubClient,
    local: &common::LocalData,
    changeset: &ChangeSet<'_>,
    selected_path: &std::path::Path,
    retry_started_at: DateTime<Utc>,
) -> anyhow::Result<()> {
    let new_remote_comments = client
        .get_comments(&ctx.owner, &ctx.repo_name, ctx.issue_number)
        .await?;
    let mut advanced = advance_comment_snapshot(
        local.base_comments.as_deref().unwrap_or_default(),
        &local.comments,
        &new_remote_comments,
        &changeset.comments,
    );
    let mut posted_new_comment_paths = Vec::new();
    for local_comment in local.comments.iter().filter(|comment| comment.is_new()) {
        if let Some(remote_comment) = new_remote_comments.iter().find(|remote_comment| {
            comment_bodies_equal(&local_comment.body, &remote_comment.body)
                && remote_comment
                    .author
                    .as_ref()
                    .is_some_and(|author| author.login == ctx.current_user)
                && remote_comment.created_at >= retry_started_at
        }) {
            set_comment_snapshot(&mut advanced, remote_comment);
            posted_new_comment_paths.push(
                ctx.storage
                    .dir()
                    .join("comments")
                    .join(&local_comment.filename),
            );
        }
    }
    ctx.storage.save_base_comments(&advanced)?;
    for path in posted_new_comment_paths {
        std::fs::remove_file(path)?;
    }

    if let Some(local_comment) = local.comments.iter().find(|comment| {
        !comment.is_new()
            && paths_match(
                &ctx.storage.dir().join("comments").join(&comment.filename),
                selected_path,
            )
    }) && let Some(comment_id) = local_comment.metadata.id.as_deref()
        && let Some(remote_comment) = new_remote_comments
            .iter()
            .find(|comment| comment.id == comment_id)
    {
        let content = crate::commands::gh::issue_agent::storage::LocalComment::format_from_comment(
            remote_comment,
        );
        std::fs::write(
            ctx.storage
                .dir()
                .join("comments")
                .join(&local_comment.filename),
            content,
        )?;
    }

    Ok(())
}

fn paths_match(left: &std::path::Path, right: &std::path::Path) -> bool {
    left.canonicalize().unwrap_or_else(|_| left.to_path_buf())
        == right.canonicalize().unwrap_or_else(|_| right.to_path_buf())
}

fn restrict_changeset_to_review_document(
    changeset: &mut ChangeSet<'_>,
    scope: &Option<ReviewDocumentPushScope>,
    comments_dir: &std::path::Path,
) {
    match scope {
        Some(ReviewDocumentPushScope::Comment { path, .. }) => {
            changeset.body = None;
            changeset.title = None;
            changeset.labels = None;
            changeset.assignees = None;
            changeset.milestone = None;
            changeset.sub_issues = None;
            changeset.parent_issue = None;
            changeset.comments.retain(|change| {
                let filename = match change {
                    CommentChange::New { filename, .. }
                    | CommentChange::Updated { filename, .. } => *filename,
                    CommentChange::Deleted { .. } => return false,
                };
                paths_match(&comments_dir.join(filename), path)
            });
        }
        Some(ReviewDocumentPushScope::IssueBody { .. }) => changeset.comments.clear(),
        None => {}
    }
}

fn advance_comment_snapshot(
    base_comments: &[CommentSnapshot],
    local_comments: &[LocalComment],
    remote_comments: &[Comment],
    changes: &[CommentChange<'_>],
) -> Vec<CommentSnapshot> {
    let mut snapshot = base_comments.to_vec();

    for change in changes {
        match change {
            CommentChange::Updated {
                filename,
                local_body,
                ..
            } => {
                let Some(comment_id) = local_comments
                    .iter()
                    .find(|comment| comment.filename == *filename)
                    .and_then(|comment| comment.metadata.id.as_deref())
                else {
                    continue;
                };
                if let Some(base_comment) = snapshot.iter_mut().find(|c| c.id == comment_id) {
                    base_comment.body = (*local_body).to_string();
                }
            }
            CommentChange::Deleted { database_id, .. } => {
                if let Some(remote_comment) = remote_comments
                    .iter()
                    .find(|comment| comment.database_id == *database_id)
                {
                    snapshot.retain(|comment| comment.id != remote_comment.id);
                }
            }
            CommentChange::New { .. } => {}
        }
    }

    for local_comment in local_comments {
        if local_comment.is_new() {
            continue;
        }
        let Some(comment_id) = local_comment.metadata.id.as_deref() else {
            continue;
        };
        let Some(remote_comment) = remote_comments.iter().find(|c| c.id == comment_id) else {
            continue;
        };
        if comment_bodies_equal(&local_comment.body, &remote_comment.body) {
            if let Some(base_comment) = snapshot.iter_mut().find(|c| c.id == comment_id) {
                base_comment.body = remote_comment.body.clone();
            } else {
                set_comment_snapshot(&mut snapshot, remote_comment);
            }
        }
    }

    snapshot
}

fn set_comment_snapshot(snapshot: &mut Vec<CommentSnapshot>, comment: &Comment) {
    if let Some(existing) = snapshot
        .iter_mut()
        .find(|existing| existing.id == comment.id)
    {
        existing.body = comment.body.clone();
    } else {
        snapshot.push(CommentSnapshot {
            id: comment.id.clone(),
            body: comment.body.clone(),
        });
    }
}

/// Collect file paths that require approval for the given changeset.
///
/// Deletions are excluded because the file no longer exists locally.
fn collect_approval_paths(changeset: &ChangeSet<'_>, ctx: &IssueContext) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    let dir = ctx.storage.dir();

    // Both body and metadata (title, labels, assignees, milestone, sub-issues, parent-issue) are
    // stored in issue.md's YAML frontmatter, so any change requires issue.md approval.
    let issue_md_changed = changeset.body.is_some()
        || changeset.title.is_some()
        || changeset.labels.is_some()
        || changeset.assignees.is_some()
        || changeset.milestone.is_some()
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
    verify_approval_paths(&paths)
}

pub(super) fn verify_approval_paths(paths: &[PathBuf]) -> anyhow::Result<()> {
    let mut unapproved = Vec::new();

    for path in paths {
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

/// Remove approval records for changed files after successful push.
fn cleanup_approval_records(changeset: &ChangeSet<'_>, ctx: &IssueContext) {
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

fn report_completed_watch_web_actions(
    issue_path: &Path,
    comments_dir: &Path,
) -> anyhow::Result<bool> {
    let mut completed_results = Vec::new();
    if let Some(result) = successful_action_result(issue_path, ReviewDocumentAction::UpdateIssue)? {
        completed_results.push(result);
    }
    completed_results.extend(successful_action_results_in_directory(
        comments_dir,
        ReviewDocumentAction::PostComment,
    )?);
    if completed_results.is_empty() {
        return Ok(false);
    }
    for result in completed_results {
        result.print_for_agent();
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shared::human_in_the_loop::{
        ReviewDocumentActionResult, document_hash, save_action_result, successful_action_result,
    };
    use rstest::rstest;
    use tempfile::TempDir;

    #[rstest]
    #[case::issue_comment(ReviewDocumentAction::PostComment, "comments/new_example.md")]
    #[case::issue_body(ReviewDocumentAction::UpdateIssue, "issue.md")]
    fn issue_push_reports_watch_web_actions_without_pushing_twice(
        #[case] action: ReviewDocumentAction,
        #[case] relative_path: &str,
    ) {
        let directory = TempDir::new().expect("fixture directory should be created");
        let issue_path = directory.path().join("issue.md");
        let comments_dir = directory.path().join("comments");
        let document_path = directory.path().join(relative_path);
        std::fs::create_dir_all(document_path.parent().expect("document has a parent"))
            .expect("document directory should be created");
        std::fs::write(&document_path, "Approved fixture document.")
            .expect("review document should be written");
        let result = ReviewDocumentActionResult::new(
            action,
            true,
            "Operation result.".to_owned(),
            document_hash(&document_path).expect("document hash should be computed"),
            None,
        );
        save_action_result(&document_path, &result).expect("action result should be saved");

        let observed = report_completed_watch_web_actions(&issue_path, &comments_dir)
            .expect("completed action result should be reported");

        assert_eq!(
            (
                observed,
                successful_action_result(&document_path, action)
                    .expect("successful action result should remain available"),
            ),
            (true, Some(result)),
        );
    }

    mod advance_comment_snapshot_tests {
        use super::*;
        use crate::commands::gh::issue_agent::testing::factories;

        fn local_comment(id: &str, body: &str) -> LocalComment {
            factories::local_comment_with(|comment| {
                comment.filename = "001_comment_12345.md".to_string();
                comment.body = body.to_string();
                comment.metadata.id = Some(id.to_string());
            })
        }

        fn remote_comment(id: &str, database_id: i64, body: &str) -> Comment {
            factories::comment_with(|comment| {
                comment.id = id.to_string();
                comment.database_id = database_id;
                comment.body = body.to_string();
            })
        }

        #[rstest]
        fn test_converged_comment_advances_snapshot() {
            let base_comments = vec![CommentSnapshot {
                id: "IC_123".to_string(),
                body: "original".to_string(),
            }];
            let local_comments = vec![local_comment("IC_123", "shared")];
            let remote_comments = vec![remote_comment("IC_123", 12345, "shared")];

            assert_eq!(
                advance_comment_snapshot(&base_comments, &local_comments, &remote_comments, &[],),
                vec![CommentSnapshot {
                    id: "IC_123".to_string(),
                    body: "shared".to_string(),
                }],
            );
        }

        #[rstest]
        fn test_deleted_comment_is_removed_from_snapshot() {
            let base_comments = vec![CommentSnapshot {
                id: "IC_123".to_string(),
                body: "original".to_string(),
            }];
            let remote_comments = vec![remote_comment("IC_123", 12345, "original")];
            let changes = vec![CommentChange::Deleted {
                database_id: 12345,
                body: "original",
                author: "alice",
            }];

            assert_eq!(
                advance_comment_snapshot(&base_comments, &[], &remote_comments, &changes),
                Vec::<CommentSnapshot>::new(),
            );
        }
    }

    mod review_document_scope_tests {
        use super::*;
        use crate::commands::gh::issue_agent::commands::push::changeset::{
            BodyChange, LabelChange, ParentIssueChange, SubIssueChange, TitleChange,
        };

        fn changeset() -> ChangeSet<'static> {
            ChangeSet {
                body: Some(BodyChange {
                    local: "Updated issue body",
                    remote: "Current issue body",
                }),
                title: Some(TitleChange {
                    local: "Updated issue title",
                    remote: "Current issue title",
                }),
                labels: Some(LabelChange {
                    to_add: vec!["sample-label".to_string()],
                    to_remove: vec![],
                    local_sorted: vec!["sample-label".to_string()],
                    remote_sorted: vec![],
                }),
                assignees: None,
                milestone: None,
                sub_issues: Some(SubIssueChange {
                    to_add: vec!["sample-owner/sample-repo#7".to_string()],
                    to_remove: vec![],
                    local_sorted: vec!["sample-owner/sample-repo#7".to_string()],
                    remote_sorted: vec![],
                }),
                parent_issue: Some(ParentIssueChange {
                    local: Some("sample-owner/sample-repo#8".to_string()),
                    remote: None,
                }),
                comments: vec![
                    CommentChange::New {
                        filename: "new_selected.md",
                        body: "Selected comment",
                    },
                    CommentChange::New {
                        filename: "new_unrelated.md",
                        body: "Unrelated comment",
                    },
                    CommentChange::Deleted {
                        database_id: 17,
                        body: "Unrelated remote comment",
                        author: "sample-author",
                    },
                ],
            }
        }

        #[rstest]
        fn issue_comment_action_keeps_only_the_selected_comment() {
            let temp_dir = TempDir::new().expect("fixture directory should be created");
            let comments_dir = temp_dir.path().join("comments");
            let selected_path = comments_dir.join("new_selected.md");
            let mut actual = changeset();
            restrict_changeset_to_review_document(
                &mut actual,
                &Some(ReviewDocumentPushScope::Comment {
                    path: selected_path,
                    retry_started_at: chrono::Utc::now(),
                }),
                &comments_dir,
            );

            assert_eq!(
                actual,
                ChangeSet {
                    body: None,
                    title: None,
                    labels: None,
                    assignees: None,
                    milestone: None,
                    sub_issues: None,
                    parent_issue: None,
                    comments: vec![CommentChange::New {
                        filename: "new_selected.md",
                        body: "Selected comment",
                    }],
                },
            );
        }

        #[rstest]
        fn issue_body_action_does_not_include_comment_changes() {
            let mut actual = changeset();
            restrict_changeset_to_review_document(
                &mut actual,
                &Some(ReviewDocumentPushScope::IssueBody {
                    path: PathBuf::from("issue.md"),
                }),
                std::path::Path::new("comments"),
            );

            assert_eq!(
                actual,
                ChangeSet {
                    body: Some(BodyChange {
                        local: "Updated issue body",
                        remote: "Current issue body",
                    }),
                    title: Some(TitleChange {
                        local: "Updated issue title",
                        remote: "Current issue title",
                    }),
                    labels: Some(LabelChange {
                        to_add: vec!["sample-label".to_string()],
                        to_remove: vec![],
                        local_sorted: vec!["sample-label".to_string()],
                        remote_sorted: vec![],
                    }),
                    assignees: None,
                    milestone: None,
                    sub_issues: Some(SubIssueChange {
                        to_add: vec!["sample-owner/sample-repo#7".to_string()],
                        to_remove: vec![],
                        local_sorted: vec!["sample-owner/sample-repo#7".to_string()],
                        remote_sorted: vec![],
                    }),
                    parent_issue: Some(ParentIssueChange {
                        local: Some("sample-owner/sample-repo#8".to_string()),
                        remote: None,
                    }),
                    comments: vec![],
                },
            );
        }
    }

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
