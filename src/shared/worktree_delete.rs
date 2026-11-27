use anyhow::{Context, Result, bail};
use std::io::{self, BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};

use crate::infra::git::{
    GitRepo, MergeStatus, get_default_branch_for_repo, get_merge_status_for_repo, get_repo_root,
};
use crate::shared::cleanup;
use crate::shared::env_var::EnvVars;
use crate::shared::hooks;
use crate::shared::merge_notify::notify_delegator_of_merge;
use crate::shared::worktree::{
    branch_to_worktree_name, ensure_clean_for_deletion, find_worktree_name, get_main_repo,
    get_worktree_branch, get_worktree_branch_checked,
};
use crate::shared::worktree_error::WmError;

pub(crate) struct WorktreeDeletePlan {
    target: WorktreeDeleteTarget,
    merge_status: Option<MergeStatus>,
    force: bool,
}

pub(crate) struct WorktreeDeleteTarget {
    pub(crate) main_repo: GitRepo,
    pub(crate) worktree_name: String,
    pub(crate) branch_name: Option<String>,
    pub(crate) worktree_path: PathBuf,
}

pub(crate) struct MergeGuardStatus {
    pub(crate) branch: String,
    pub(crate) status: MergeStatus,
    pub(crate) should_cleanup: bool,
}

pub(crate) fn resolve_merge_guard_branch(
    repo: &GitRepo,
    branch_name: Option<&str>,
) -> Option<String> {
    resolve_merge_guard_branch_with(branch_name, |branch| repo.local_branch_exists(branch))
}

fn resolve_merge_guard_branch_with(
    branch_name: Option<&str>,
    branch_exists: impl FnOnce(&str) -> bool,
) -> Option<String> {
    branch_name
        .filter(|branch| branch_exists(branch))
        .map(str::to_string)
}

pub(crate) fn classify_merge_guard_status(branch: String, status: MergeStatus) -> MergeGuardStatus {
    let should_cleanup = status.should_cleanup();
    MergeGuardStatus {
        branch,
        status,
        should_cleanup,
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum UnmergedWorktreePolicy {
    Confirm,
    Reject,
}

#[derive(Debug, thiserror::Error)]
#[error(
    "Branch `{branch}` is not merged ({reason}); close the session while keeping its worktree to continue"
)]
pub(crate) struct UnmergedWorktreeError {
    branch: String,
    reason: String,
}

impl UnmergedWorktreeError {
    pub(crate) fn new(branch: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            branch: branch.into(),
            reason: reason.into(),
        }
    }
}

pub(crate) async fn prepare(
    worktree_path: &Path,
    force: bool,
    unmerged_policy: UnmergedWorktreePolicy,
) -> Result<WorktreeDeletePlan> {
    let target = resolve_target(worktree_path)?;

    ensure_clean_for_deletion(&target.worktree_path, force)?;

    let merge_status = check_merge_status(
        &target.main_repo,
        target.branch_name.as_deref(),
        force,
        unmerged_policy,
    )
    .await?;

    Ok(WorktreeDeletePlan {
        target,
        merge_status,
        force,
    })
}

pub(crate) fn resolve_target(worktree_path: &Path) -> Result<WorktreeDeleteTarget> {
    let repo = GitRepo::open_at(worktree_path).map_err(|_| WmError::NotInGitRepo)?;
    let main_repo = get_main_repo(&repo)?;
    let worktree_path = repo.workdir().to_path_buf();
    let worktree_path_str = worktree_path.to_string_lossy();
    let worktree_name = find_worktree_name(&main_repo, &worktree_path_str)?;
    let branch_name = get_worktree_branch(&main_repo, &worktree_name);

    Ok(WorktreeDeleteTarget {
        main_repo,
        worktree_name,
        branch_name,
        worktree_path,
    })
}

pub(crate) async fn execute(plan: WorktreeDeletePlan, skip_hooks: bool) -> Result<()> {
    let WorktreeDeletePlan {
        target:
            WorktreeDeleteTarget {
                main_repo,
                worktree_name,
                branch_name,
                worktree_path,
            },
        merge_status,
        force,
    } = plan;
    let worktree_path_str = worktree_path.to_string_lossy();

    ensure_clean_for_deletion(&worktree_path, force)?;

    // Must complete before cleanup_worktree_by_name below: it deletes this
    // worktree's session records and, when the command runs from the
    // worktree's own pane, kills the very pane this process is running in.
    let merged = merge_status.as_ref().is_some_and(MergeStatus::is_merged);
    if let Some(branch) = branch_name.as_deref()
        && merged
    {
        notify_delegator_of_merge(&main_repo, branch, &worktree_path).await;
    }

    let hook_ran = run_pre_delete_hook(
        &main_repo,
        branch_name.as_deref(),
        &worktree_path_str,
        skip_hooks,
    );

    let result = cleanup::cleanup_worktree_by_name_with_before_delete(
        &main_repo,
        &worktree_name,
        &worktree_path,
        force,
        || {
            ensure_worktree_delete_plan_before_delete(
                merge_status.as_ref(),
                force,
                branch_name.as_deref(),
                || get_worktree_branch_checked(&main_repo, &worktree_name),
                |branch| branch_has_no_unique_commits(&main_repo, branch),
            )
        },
        || {
            if !skip_hooks {
                hooks::spawn_post_worktree_delete_hook(
                    main_repo.workdir(),
                    &worktree_path,
                    branch_name.as_deref(),
                    merged,
                );
            }
        },
    );

    let result = match result {
        Ok(result) => result,
        Err(error) => {
            if hook_ran {
                eprintln!(
                    "note: the pre-worktree-delete hook already ran before this failure; \
                     any process it stopped will not be restarted automatically"
                );
            }
            return Err(error);
        }
    };

    if !result.worktree_deleted {
        if hook_ran {
            eprintln!(
                "note: the pre-worktree-delete hook already ran before this failure; \
                 any process it stopped will not be restarted automatically"
            );
        }
        bail!("Failed to remove worktree: {worktree_path_str}");
    }
    println!("Worktree removed: {worktree_path_str}");

    if let Some(branch) = &result.branch_deleted {
        println!("Branch deleted: {branch}");
    }
    if result.sessions_cleaned > 0 {
        println!("Sessions cleaned: {}", result.sessions_cleaned);
    }
    if result.process_groups_signaled > 0 {
        println!(
            "Process groups signaled: {}",
            result.process_groups_signaled
        );
    }

    Ok(())
}

/// Runs the pre-worktree-delete hook, if configured. Best-effort: unlike
/// post-worktree-create, a hook failure here only logs a warning and never
/// blocks deletion.
///
/// Returns whether the hook actually ran (`false` when skipped via
/// `--skip-hooks` or when no hook is configured), so callers can note it in
/// later error messages.
fn run_pre_delete_hook(
    repo: &GitRepo,
    branch_name: Option<&str>,
    worktree_path: &str,
    skip_hooks: bool,
) -> bool {
    run_pre_delete_hook_with(
        repo,
        branch_name,
        worktree_path,
        skip_hooks,
        hooks::hook_exists,
        hooks::run_hook,
    )
}

fn run_pre_delete_hook_with(
    repo: &GitRepo,
    branch_name: Option<&str>,
    worktree_path: &str,
    skip_hooks: bool,
    hook_exists: impl FnOnce(&str) -> bool,
    run_hook: impl FnOnce(&str, &[(&str, &str)]) -> anyhow::Result<()>,
) -> bool {
    if skip_hooks {
        eprintln!("Skipping pre-worktree-delete hook (--skip-hooks)");
        return false;
    }

    if !hook_exists("pre-worktree-delete") {
        return false;
    }

    let branch_name = branch_name.unwrap_or_default();
    if let Err(e) = run_hook(
        "pre-worktree-delete",
        &[
            (EnvVars::worktree_path_name(), worktree_path),
            (EnvVars::branch_name_name(), branch_name),
            (EnvVars::repo_root_name(), &repo.workdir().to_string_lossy()),
        ],
    ) {
        eprintln!("Warning: pre-worktree-delete hook failed: {e}");
    }

    true
}

async fn check_merge_status(
    repo: &GitRepo,
    branch_name: Option<&str>,
    force: bool,
    unmerged_policy: UnmergedWorktreePolicy,
) -> Result<Option<MergeStatus>> {
    let repo_for_status = repo.clone();
    check_merge_status_with(
        branch_name,
        force,
        unmerged_policy,
        |branch| repo.local_branch_exists(branch),
        move |branch| {
            let branch = branch.to_string();
            async move { get_merge_status_for_branch(&repo_for_status, &branch).await }
        },
        || {
            let stdin = io::stdin();
            let stdout = io::stdout();
            confirm_with_terminal(
                "Delete anyway? [y/N] ",
                &mut stdin.lock(),
                &mut stdout.lock(),
                || stdin.is_terminal(),
            )
        },
    )
    .await
}

pub(crate) async fn get_merge_status_for_branch(repo: &GitRepo, branch: &str) -> MergeStatus {
    let status = get_merge_status_for_repo(repo, branch).await;
    close_status_for_branch(repo, branch, status)
}

pub(crate) async fn get_merge_status_for_branch_from_local_refs(
    repo: &GitRepo,
    branch: &str,
) -> MergeStatus {
    let status = get_merge_status_for_repo(repo, branch).await;
    let repo_for_default_branch = repo.clone();
    let repo_for_ancestry = repo.clone();
    close_status_for_branch_from_local_refs_with(
        status,
        branch.to_string(),
        move || get_default_branch_for_repo(&repo_for_default_branch).ok(),
        move |ancestor, descendant| repo_for_ancestry.is_ancestor(ancestor, descendant),
    )
    .await
}

fn close_status_for_branch(repo: &GitRepo, branch: &str, status: MergeStatus) -> MergeStatus {
    if !matches!(&status, MergeStatus::NotMerged { .. }) {
        return status;
    }

    let refreshed = repo.fetch_origin_prune().is_ok();
    let default_branch = refreshed
        .then(|| get_default_branch_for_repo(repo).ok())
        .flatten();
    close_status_for_branch_with(
        status,
        branch,
        default_branch.as_deref(),
        |ancestor, descendant| repo.is_ancestor(ancestor, descendant),
    )
}

async fn close_status_for_branch_from_local_refs_with(
    status: MergeStatus,
    branch: String,
    get_default_branch: impl FnOnce() -> Option<String> + Send + 'static,
    is_ancestor: impl FnOnce(&str, &str) -> bool + Send + 'static,
) -> MergeStatus {
    if !matches!(&status, MergeStatus::NotMerged { .. }) {
        return status;
    }

    let fallback_status = status.clone();
    tokio::task::spawn_blocking(move || {
        let default_branch = get_default_branch();
        close_status_for_branch_with(status, &branch, default_branch.as_deref(), is_ancestor)
    })
    .await
    .unwrap_or(fallback_status)
}

fn close_status_for_branch_with(
    status: MergeStatus,
    branch: &str,
    default_branch: Option<&str>,
    is_ancestor: impl FnOnce(&str, &str) -> bool,
) -> MergeStatus {
    if !matches!(&status, MergeStatus::NotMerged { .. })
        || !branch_has_no_unique_commits_with(branch, default_branch, is_ancestor)
    {
        return status;
    }

    MergeStatus::NoUniqueCommits {
        reason: "No commits unique to branch".to_string(),
    }
}

fn branch_has_no_unique_commits(repo: &GitRepo, branch: &str) -> bool {
    let refreshed = repo.fetch_origin_prune().is_ok();
    let default_branch = refreshed
        .then(|| get_default_branch_for_repo(repo).ok())
        .flatten();
    branch_has_no_unique_commits_with(branch, default_branch.as_deref(), |ancestor, descendant| {
        repo.is_ancestor(ancestor, descendant)
    })
}

fn ensure_worktree_delete_plan_before_delete(
    status: Option<&MergeStatus>,
    force: bool,
    planned_branch: Option<&str>,
    current_branch: impl FnOnce() -> Result<Option<String>>,
    verify_still_safe: impl FnOnce(&str) -> bool,
) -> Result<()> {
    let current_branch =
        current_branch().context("Could not inspect worktree branch before deletion")?;
    if planned_branch != current_branch.as_deref() {
        bail!(
            "Worktree branch changed from `{}` to `{}` before deletion; retry close to check the current branch",
            planned_branch.unwrap_or("detached"),
            current_branch.as_deref().unwrap_or("detached"),
        );
    }

    ensure_branch_no_unique_commits_before_delete(status, force, planned_branch, verify_still_safe)
}

fn ensure_branch_no_unique_commits_before_delete(
    status: Option<&MergeStatus>,
    force: bool,
    branch: Option<&str>,
    verify_still_safe: impl FnOnce(&str) -> bool,
) -> Result<()> {
    if force || !matches!(status, Some(MergeStatus::NoUniqueCommits { .. })) {
        return Ok(());
    }

    let Some(branch) = branch else {
        return Err(UnmergedWorktreeError::new(
            "unknown",
            "branch could not be checked again before deletion",
        )
        .into());
    };
    if verify_still_safe(branch) {
        return Ok(());
    }

    Err(UnmergedWorktreeError::new(
        branch,
        "branch could not be verified as part of the origin default branch before deletion",
    )
    .into())
}

fn branch_has_no_unique_commits_with(
    branch: &str,
    default_branch: Option<&str>,
    is_ancestor: impl FnOnce(&str, &str) -> bool,
) -> bool {
    let Some(default_branch) = default_branch else {
        return false;
    };
    let branch_ref = format!("refs/heads/{branch}");
    let default_branch_ref = format!("refs/remotes/origin/{default_branch}");
    is_ancestor(&branch_ref, &default_branch_ref)
}

async fn inspect_merge_guard_status_with<F, G, Fut>(
    branch_name: Option<&str>,
    branch_exists: F,
    get_status: G,
) -> Option<MergeGuardStatus>
where
    F: FnOnce(&str) -> bool,
    G: FnOnce(&str) -> Fut,
    Fut: std::future::Future<Output = MergeStatus>,
{
    let branch = resolve_merge_guard_branch_with(branch_name, branch_exists)?;
    let status = get_status(&branch).await;
    Some(classify_merge_guard_status(branch, status))
}

async fn check_merge_status_with<F, G, Fut>(
    branch_name: Option<&str>,
    force: bool,
    unmerged_policy: UnmergedWorktreePolicy,
    branch_exists: F,
    get_status: G,
    confirm: impl FnOnce() -> Option<bool>,
) -> Result<Option<MergeStatus>>
where
    F: FnOnce(&str) -> bool,
    G: FnOnce(&str) -> Fut,
    Fut: std::future::Future<Output = MergeStatus>,
{
    let Some(merge_guard_status) =
        inspect_merge_guard_status_with(branch_name, branch_exists, get_status).await
    else {
        return Ok(None);
    };
    let MergeGuardStatus {
        branch,
        status: merge_status,
        should_cleanup,
    } = merge_guard_status;

    if !should_cleanup && !force {
        if unmerged_policy == UnmergedWorktreePolicy::Reject {
            return Err(UnmergedWorktreeError::new(branch, merge_status.reason()).into());
        }
        eprintln!(
            "Warning: Branch '{}' is not merged ({})",
            branch,
            merge_status.reason()
        );
        match confirm() {
            Some(true) => {}
            Some(false) => return Err(WmError::Cancelled.into()),
            None => {
                return Err(anyhow::anyhow!(
                    "Cancelled: stdin is not a terminal; input was not accepted as confirmation."
                ));
            }
        }
    }

    Ok(Some(merge_status))
}

pub(crate) fn confirm_with_terminal<R: BufRead, W: Write>(
    prompt: &str,
    input: &mut R,
    output: &mut W,
    stdin_is_terminal: impl FnOnce() -> bool,
) -> Option<bool> {
    if !stdin_is_terminal() {
        return None;
    }

    if write!(output, "{prompt}")
        .and_then(|()| output.flush())
        .is_err()
    {
        return Some(false);
    }

    let mut line = String::new();
    input.read_line(&mut line).ok();
    Some(line.trim().eq_ignore_ascii_case("y"))
}

/// Resolve the worktree path from the argument or current directory.
pub(crate) fn resolve_worktree_path(
    arg: &str,
    worktrees_dir: &str,
    branch_prefix: &str,
) -> Result<String> {
    resolve_worktree_path_with_repo_root(arg, worktrees_dir, branch_prefix, get_repo_root)
}

fn resolve_worktree_path_with_repo_root(
    arg: &str,
    worktrees_dir: &str,
    branch_prefix: &str,
    repo_root: impl FnOnce() -> Result<String>,
) -> Result<String> {
    // First, try to treat the argument as an existing path
    if let Ok(path) = std::fs::canonicalize(arg) {
        return Ok(path.to_string_lossy().to_string());
    }

    if Path::new(arg).is_absolute() {
        return Err(WmError::WorktreeNotFound(arg.to_string()).into());
    }

    // Fall back to resolving the value as a branch/worktree name
    let repo_root = repo_root()?;
    let worktree_name = branch_to_worktree_name(arg, branch_prefix);
    let candidate_path = format!("{repo_root}/{worktrees_dir}/{worktree_name}");

    if std::path::Path::new(&candidate_path).exists() {
        let path = std::fs::canonicalize(&candidate_path)
            .context("Failed to canonicalize worktree path")?;
        return Ok(path.to_string_lossy().to_string());
    }

    Err(WmError::WorktreeNotFound(arg.to_string()).into())
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;
    use crate::shared::testing::TestRepo;
    use crate::shared::worktree::delete_worktree_if_clean;

    #[rstest]
    #[case::no_pr_without_unique_commits(
        MergeStatus::NotMerged { reason: "No PR found".to_string() },
        Some("main"),
        true,
        MergeStatus::NoUniqueCommits { reason: "No commits unique to branch".to_string() },
    )]
    #[case::open_pr_without_unique_commits(
        MergeStatus::NotMerged { reason: "pull request is open".to_string() },
        Some("main"),
        true,
        MergeStatus::NoUniqueCommits { reason: "No commits unique to branch".to_string() },
    )]
    #[case::branch_has_unique_commits(
        MergeStatus::NotMerged { reason: "No PR found".to_string() },
        Some("main"),
        false,
        MergeStatus::NotMerged { reason: "No PR found".to_string() },
    )]
    #[case::default_branch_missing(
        MergeStatus::NotMerged { reason: "No PR found".to_string() },
        None,
        true,
        MergeStatus::NotMerged { reason: "No PR found".to_string() },
    )]
    fn close_status_uses_origin_ancestry(
        #[case] status: MergeStatus,
        #[case] default_branch: Option<&str>,
        #[case] is_ancestor: bool,
        #[case] expected: MergeStatus,
    ) {
        assert_eq!(
            close_status_for_branch_with(status, "agent-branch", default_branch, |_, _| {
                is_ancestor
            },),
            expected,
        );
    }

    #[tokio::test]
    async fn close_status_checks_local_origin_ancestry_on_a_blocking_worker() {
        let caller_thread = std::thread::current().id();
        let (sender, receiver) = std::sync::mpsc::channel();
        let status = close_status_for_branch_from_local_refs_with(
            MergeStatus::NotMerged {
                reason: "No PR found".to_string(),
            },
            "agent-branch".to_string(),
            || Some("main".to_string()),
            move |ancestor, descendant| {
                sender
                    .send((
                        std::thread::current().id(),
                        ancestor.to_string(),
                        descendant.to_string(),
                    ))
                    .expect("send ancestry inputs");
                true
            },
        )
        .await;
        let observed_ancestry =
            receiver
                .try_recv()
                .ok()
                .map(|(worker_thread, ancestor, descendant)| {
                    (worker_thread != caller_thread, ancestor, descendant)
                });

        assert_eq!(
            (status, observed_ancestry),
            (
                MergeStatus::NoUniqueCommits {
                    reason: "No commits unique to branch".to_string(),
                },
                Some((
                    true,
                    "refs/heads/agent-branch".to_string(),
                    "refs/remotes/origin/main".to_string(),
                )),
            ),
        );
    }

    #[test]
    fn close_status_checks_the_branch_against_the_refreshed_default_branch() {
        let mut compared_refs = None;
        let status = close_status_for_branch_with(
            MergeStatus::NotMerged {
                reason: "pull request is open".to_string(),
            },
            "agent-branch",
            Some("main"),
            |ancestor, descendant| {
                compared_refs = Some((ancestor.to_string(), descendant.to_string()));
                true
            },
        );

        assert_eq!(
            (status, compared_refs),
            (
                MergeStatus::NoUniqueCommits {
                    reason: "No commits unique to branch".to_string(),
                },
                Some((
                    "refs/heads/agent-branch".to_string(),
                    "refs/remotes/origin/main".to_string(),
                )),
            ),
        );
    }

    #[rstest]
    #[case::unchanged_branch(
        Some("agent-branch"),
        Ok(Some("agent-branch".to_string())),
        Ok(()),
        true,
    )]
    #[case::changed_branch_with_force(
        Some("agent-branch"),
        Ok(Some("replacement-branch".to_string())),
        Err("Worktree branch changed from `agent-branch` to `replacement-branch` before deletion; retry close to check the current branch".to_string()),
        false,
    )]
    #[case::branch_inspection_failure(
        Some("agent-branch"),
        Err("git worktree list failed".to_string()),
        Err("Could not inspect worktree branch before deletion".to_string()),
        false,
    )]
    fn close_checks_the_branch_before_cleanup(
        #[case] planned_branch: Option<&str>,
        #[case] current_branch: std::result::Result<Option<String>, String>,
        #[case] expected: std::result::Result<(), String>,
        #[case] expected_deleted: bool,
    ) {
        let temp_dir = tempfile::tempdir().expect("temporary directory should be created");
        let mut deleted = false;
        let result = delete_worktree_if_clean(
            temp_dir.path(),
            true,
            || {
                ensure_worktree_delete_plan_before_delete(
                    Some(&MergeStatus::NoUniqueCommits {
                        reason: "No commits unique to branch".to_string(),
                    }),
                    true,
                    planned_branch,
                    || current_branch.map_err(anyhow::Error::msg),
                    |_| false,
                )
            },
            || {
                deleted = true;
                Ok(true)
            },
        )
        .map(|_| ())
        .map_err(|error| error.to_string());

        assert_eq!((result, deleted), (expected, expected_deleted));
    }

    #[rstest]
    #[case::branch_remains_safe(
        Some(MergeStatus::NoUniqueCommits { reason: "No commits unique to branch".to_string() }),
        false,
        Some("agent-branch"),
        true,
        Ok(()),
    )]
    #[case::branch_gains_unique_commit(
        Some(MergeStatus::NoUniqueCommits { reason: "No commits unique to branch".to_string() }),
        false,
        Some("agent-branch"),
        false,
        Err("Branch `agent-branch` is not merged (branch could not be verified as part of the origin default branch before deletion); close the session while keeping its worktree to continue".to_string()),
    )]
    #[case::other_unmerged_status(
        Some(MergeStatus::NotMerged { reason: "pull request is open".to_string() }),
        false,
        Some("agent-branch"),
        false,
        Ok(()),
    )]
    #[case::force_bypasses_revalidation(
        Some(MergeStatus::NoUniqueCommits { reason: "No commits unique to branch".to_string() }),
        true,
        Some("agent-branch"),
        false,
        Ok(()),
    )]
    fn close_revalidates_no_unique_commit_status_before_deletion(
        #[case] status: Option<MergeStatus>,
        #[case] force: bool,
        #[case] branch: Option<&str>,
        #[case] still_safe: bool,
        #[case] expected: std::result::Result<(), String>,
    ) {
        let result =
            ensure_branch_no_unique_commits_before_delete(status.as_ref(), force, branch, |_| {
                still_safe
            })
            .map_err(|error| error.to_string());

        assert_eq!(result, expected);
    }

    #[tokio::test]
    async fn check_merge_status_uses_the_selected_repository_branch() {
        let mut looked_up_branch = None;
        let result = check_merge_status_with(
            Some("feature"),
            false,
            UnmergedWorktreePolicy::Confirm,
            |branch| {
                looked_up_branch = Some(branch.to_string());
                true
            },
            |branch| {
                assert_eq!(branch, "feature");
                std::future::ready(MergeStatus::Merged {
                    reason: "fictional change merged".to_string(),
                })
            },
            || Some(true),
        )
        .await
        .expect("merge status");

        assert_eq!(
            (looked_up_branch, result),
            (
                Some("feature".to_string()),
                Some(MergeStatus::Merged {
                    reason: "fictional change merged".to_string(),
                }),
            ),
        );
    }

    #[tokio::test]
    async fn rejects_unmerged_worktrees_without_prompting() {
        let result = check_merge_status_with(
            Some("feature"),
            false,
            UnmergedWorktreePolicy::Reject,
            |_| true,
            |_| {
                std::future::ready(MergeStatus::NotMerged {
                    reason: "pull request is still open".to_string(),
                })
            },
            || Some(true),
        )
        .await;

        assert_eq!(
            result.err().map(|error| error.to_string()),
            Some(
                "Branch `feature` is not merged (pull request is still open); close the session while keeping its worktree to continue"
                    .to_string(),
            ),
        );
    }

    #[rstest]
    #[case::terminal_y_continues(
        true,
        "y\n",
        Ok(Some(MergeStatus::NotMerged {
            reason: "pull request is still open".to_string(),
        })),
        "Delete anyway? [y/N] ",
        "",
    )]
    #[case::terminal_n_cancels(
        true,
        "n\n",
        Err("Cancelled.".to_string()),
        "Delete anyway? [y/N] ",
        "",
    )]
    #[case::non_terminal_y_cancels(
        false,
        "y\n",
        Err("Cancelled: stdin is not a terminal; input was not accepted as confirmation.".to_string()),
        "",
        "y\n",
    )]
    #[tokio::test]
    async fn confirms_unmerged_worktrees_only_with_terminal_y(
        #[case] stdin_is_terminal: bool,
        #[case] input: &str,
        #[case] expected_result: std::result::Result<Option<MergeStatus>, String>,
        #[case] expected_output: &str,
        #[case] expected_unread: &str,
    ) {
        let mut reader = io::Cursor::new(input.as_bytes());
        let mut output = Vec::new();
        let result = check_merge_status_with(
            Some("feature"),
            false,
            UnmergedWorktreePolicy::Confirm,
            |_| true,
            |_| {
                std::future::ready(MergeStatus::NotMerged {
                    reason: "pull request is still open".to_string(),
                })
            },
            || {
                confirm_with_terminal("Delete anyway? [y/N] ", &mut reader, &mut output, || {
                    stdin_is_terminal
                })
            },
        )
        .await
        .map_err(|error| error.to_string());

        let mut unread = String::new();
        io::Read::read_to_string(&mut reader, &mut unread).expect("read remaining input");
        assert_eq!(
            (
                result,
                String::from_utf8(output).expect("utf8 output"),
                unread,
            ),
            (
                expected_result,
                expected_output.to_string(),
                expected_unread.to_string(),
            ),
        );
    }

    #[tokio::test]
    async fn force_skips_confirmation_for_unmerged_worktrees() {
        let result = check_merge_status_with(
            Some("feature"),
            true,
            UnmergedWorktreePolicy::Confirm,
            |_| true,
            |_| {
                std::future::ready(MergeStatus::NotMerged {
                    reason: "pull request is still open".to_string(),
                })
            },
            || panic!("force must skip the confirmation"),
        )
        .await
        .expect("force bypasses confirmation");

        assert_eq!(
            result,
            Some(MergeStatus::NotMerged {
                reason: "pull request is still open".to_string(),
            }),
        );
    }

    #[rstest]
    #[case::non_terminal_rejects_piped_y(false, "y\n", (None, "", "y\n"))]
    #[case::terminal_accepts_y(true, "y\n", (Some(true), "Delete anyway? [y/N] ", ""))]
    #[case::terminal_rejects_n(true, "n\n", (Some(false), "Delete anyway? [y/N] ", ""))]
    fn unmerged_confirmation_requires_terminal_input(
        #[case] stdin_is_terminal: bool,
        #[case] input: &str,
        #[case] expected: (Option<bool>, &str, &str),
    ) {
        let mut reader = io::Cursor::new(input.as_bytes());
        let mut output = Vec::new();

        let confirmed =
            confirm_with_terminal("Delete anyway? [y/N] ", &mut reader, &mut output, || {
                stdin_is_terminal
            });
        let mut unread = String::new();
        io::Read::read_to_string(&mut reader, &mut unread).expect("read remaining input");

        assert_eq!(
            (
                confirmed,
                String::from_utf8(output).expect("utf8 output").as_str(),
                unread.as_str(),
            ),
            expected,
        );
    }

    struct ClosedWriter;

    impl Write for ClosedWriter {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(io::ErrorKind::BrokenPipe.into())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn unmerged_confirmation_rejects_when_prompt_cannot_be_written() {
        let mut reader = io::Cursor::new("y\n".as_bytes());

        let result = confirm_with_terminal(
            "Delete anyway? [y/N] ",
            &mut reader,
            &mut ClosedWriter,
            || true,
        );
        let mut unread = String::new();
        io::Read::read_to_string(&mut reader, &mut unread).expect("read remaining input");

        assert_eq!((result, unread), (Some(false), "y\n".to_string()));
    }

    #[rstest]
    #[case::skip_hooks_true_never_invokes(true, true, false)]
    #[case::skip_hooks_false_invokes_and_does_not_panic(false, true, true)]
    #[case::hook_not_configured_returns_false(false, false, false)]
    fn run_pre_delete_hook_respects_skip_hooks(
        #[case] skip_hooks: bool,
        #[case] hook_installed: bool,
        #[case] expect_invoked: bool,
    ) {
        let repo = TestRepo::new().open();
        let mut hook_invoked = false;
        let hook_ran = run_pre_delete_hook_with(
            &repo,
            Some("feature"),
            "/path/to/worktree",
            skip_hooks,
            |_| hook_installed,
            |_, _| {
                hook_invoked = true;
                Err(anyhow::anyhow!("hook failed"))
            },
        );

        assert_eq!((hook_ran, hook_invoked), (expect_invoked, expect_invoked));
    }

    #[test]
    fn resolve_worktree_path_with_existing_path() {
        let test_repo = TestRepo::new();
        test_repo.create_worktree("feature");

        let wt_path = test_repo.worktree_path("feature");
        let result =
            resolve_worktree_path(wt_path.to_str().unwrap(), ".worktrees", "fohte/").unwrap();

        assert_eq!(result, wt_path.to_string_lossy().to_string());
    }

    #[test]
    fn resolve_worktree_path_with_missing_absolute_path_skips_repo_root_lookup() {
        let result = resolve_worktree_path_with_repo_root(
            "/nonexistent/path/to/worktree",
            ".worktrees",
            "fohte/",
            || Err(anyhow::anyhow!("repo root lookup was attempted")),
        );
        assert_eq!(
            result.map_err(|error| error.to_string()),
            Err("Worktree not found: /nonexistent/path/to/worktree".to_string()),
        );
    }
}
