use anyhow::{Context, Result, bail};
use std::io::{self, BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};

use crate::infra::git::{
    GitRepo, MergeStatus, get_main_branch_for_repo, get_merge_status_for_repo, get_repo_root,
};
use crate::shared::cleanup;
use crate::shared::env_var::EnvVars;
use crate::shared::hooks;
use crate::shared::merge_notify::notify_delegator_of_merge;
use crate::shared::worktree::{
    branch_to_worktree_name, ensure_clean_for_deletion, find_worktree_name, get_main_repo,
    get_worktree_branch,
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

    let result = cleanup::cleanup_worktree_by_name(
        &main_repo,
        &worktree_name,
        &worktree_path,
        force,
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

fn close_status_for_branch(repo: &GitRepo, branch: &str, status: MergeStatus) -> MergeStatus {
    if !matches!(
        &status,
        MergeStatus::NotMerged { reason } if reason == "No PR found"
    ) {
        return status;
    }

    let Ok(default_branch) = get_main_branch_for_repo(repo) else {
        return status;
    };
    let branch_ref = format!("refs/heads/{branch}");
    let default_branch_ref = format!("refs/remotes/origin/{default_branch}");
    if repo.is_ancestor(&branch_ref, &default_branch_ref) {
        MergeStatus::Closed {
            reason: "No commits unique to branch".to_string(),
        }
    } else {
        status
    }
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
    use crate::infra::git::cmd::run_git;
    use crate::shared::testing::TestRepo;

    #[test]
    fn close_allows_a_branch_without_commits_unique_to_origin_default() {
        let test_repo = TestRepo::new();
        let repo = test_repo.open();
        run_git(
            repo.workdir(),
            ["update-ref", "refs/remotes/origin/master", "HEAD"],
        )
        .unwrap();
        run_git(repo.workdir(), ["branch", "agent-branch"]).unwrap();

        let status = close_status_for_branch(
            &repo,
            "agent-branch",
            MergeStatus::NotMerged {
                reason: "No PR found".to_string(),
            },
        );

        assert_eq!(
            (status.is_merged(), status.should_cleanup(), status,),
            (
                false,
                true,
                MergeStatus::Closed {
                    reason: "No commits unique to branch".to_string(),
                },
            ),
        );
    }

    #[test]
    fn close_keeps_a_branch_with_unique_commits_unmerged() {
        let test_repo = TestRepo::new();
        let repo = test_repo.open();
        let default_head = run_git(repo.workdir(), ["rev-parse", "HEAD"]).unwrap();
        run_git(
            repo.workdir(),
            [
                "update-ref",
                "refs/remotes/origin/master",
                default_head.as_str(),
            ],
        )
        .unwrap();
        test_repo.commit_file("branch-file", "sample content");
        run_git(repo.workdir(), ["branch", "agent-branch"]).unwrap();

        let status = close_status_for_branch(
            &repo,
            "agent-branch",
            MergeStatus::NotMerged {
                reason: "No PR found".to_string(),
            },
        );

        assert_eq!(
            status,
            MergeStatus::NotMerged {
                reason: "No PR found".to_string(),
            },
        );
    }

    #[test]
    fn close_keeps_a_branch_unmerged_when_origin_default_is_missing() {
        let test_repo = TestRepo::new();
        let repo = test_repo.open();
        run_git(repo.workdir(), ["branch", "agent-branch"]).unwrap();

        assert_eq!(
            close_status_for_branch(
                &repo,
                "agent-branch",
                MergeStatus::NotMerged {
                    reason: "No PR found".to_string(),
                },
            ),
            MergeStatus::NotMerged {
                reason: "No PR found".to_string(),
            },
        );
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
