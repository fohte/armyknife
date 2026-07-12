use anyhow::{Context, bail};
use clap::Args;
use std::io::{self, Write};

use super::error::{Result, WmError};
use super::git::{branch_to_worktree_name, get_merge_status, get_repo_root, local_branch_exists};
use super::worktree::{find_worktree_name, get_main_repo, get_worktree_branch};
use crate::infra::git::GitRepo;
use crate::infra::tmux;
use crate::shared::cleanup;
use crate::shared::config::load_config;
use crate::shared::env_var::EnvVars;
use crate::shared::hooks;

#[derive(Args, Clone, PartialEq, Eq)]
pub struct DeleteArgs {
    /// Worktree path or name (default: current directory)
    pub worktree: Option<String>,

    /// Force delete without confirmation even if the branch is neither merged nor closed
    #[arg(short, long)]
    pub force: bool,

    /// Skip the pre-worktree-delete hook
    #[arg(long)]
    pub skip_hooks: bool,
}

pub async fn run(args: &DeleteArgs) -> Result<()> {
    let config = load_config()?;
    let worktree_path = resolve_worktree_path(
        args.worktree.as_deref(),
        &config.wm.worktrees_dir,
        &config.wm.branch_prefix,
    )?;

    let repo = GitRepo::open_from_env().map_err(|_| WmError::NotInGitRepo)?;
    let main_repo = get_main_repo(&repo)?;

    let worktree_name = find_worktree_name(&main_repo, &worktree_path)?;
    let branch_name = get_worktree_branch(&main_repo, &worktree_name);

    // Check merge status before deletion (needs worktree to still exist)
    check_merge_status(branch_name.as_deref(), args.force).await?;

    let hook_ran = run_pre_delete_hook(
        &main_repo,
        branch_name.as_deref(),
        &worktree_path,
        args.skip_hooks,
    );

    // Capture the current tmux window ID before cleanup deletes it,
    // so we can close the window we're sitting in
    let current_window_id = tmux::get_window_id_if_in_path(&worktree_path);

    let worktree_abs = std::path::Path::new(&worktree_path);
    let result = cleanup::cleanup_worktree_by_name(&main_repo, &worktree_name, worktree_abs)?;

    if !result.worktree_deleted {
        if hook_ran {
            eprintln!(
                "note: the pre-worktree-delete hook already ran before this failure; \
                 any process it stopped will not be restarted automatically"
            );
        }
        bail!("Failed to remove worktree: {worktree_path}");
    }
    println!("Worktree removed: {worktree_path}");

    if let Some(branch) = &result.branch_deleted {
        println!("Branch deleted: {branch}");
    }
    if result.sessions_cleaned > 0 {
        println!("Sessions cleaned: {}", result.sessions_cleaned);
    }

    // Close the current tmux window if we're inside the deleted worktree.
    // cleanup_worktree_by_name uses get_window_ids_in_path which queries all
    // panes globally, but the current window may have already been captured
    // above via get_window_id_if_in_path. Ensure it's closed.
    if let Some(window_id) = current_window_id {
        let _ = tmux::kill_window(&window_id);
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
    if skip_hooks {
        eprintln!("Skipping pre-worktree-delete hook (--skip-hooks)");
        return false;
    }

    if !hooks::hook_exists("pre-worktree-delete") {
        return false;
    }

    let branch_name = branch_name.unwrap_or_default();
    if let Err(e) = hooks::run_hook(
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

async fn check_merge_status(branch_name: Option<&str>, force: bool) -> Result<()> {
    if let Some(branch) = branch_name.filter(|b| local_branch_exists(b)) {
        let merge_status = get_merge_status(branch).await;
        if !merge_status.should_cleanup() && !force {
            eprintln!(
                "Warning: Branch '{}' is not merged ({})",
                branch,
                merge_status.reason()
            );
            print!("Delete anyway? [y/N] ");
            io::stdout().flush().ok();

            let mut input = String::new();
            io::stdin().read_line(&mut input).ok();
            if !input.trim().eq_ignore_ascii_case("y") {
                println!("Cancelled.");
                return Err(WmError::Cancelled.into());
            }
        }
    }

    Ok(())
}

/// Resolve the worktree path from the argument or current directory
fn resolve_worktree_path(
    worktree_arg: Option<&str>,
    worktrees_dir: &str,
    branch_prefix: &str,
) -> Result<String> {
    if let Some(arg) = worktree_arg {
        // First, try to treat the argument as an existing path
        if let Ok(path) = std::fs::canonicalize(arg) {
            return Ok(path.to_string_lossy().to_string());
        }

        // Fall back to resolving the value as a branch/worktree name
        let repo_root = get_repo_root()?;
        let worktree_name = branch_to_worktree_name(arg, branch_prefix);
        let candidate_path = format!("{repo_root}/{worktrees_dir}/{worktree_name}");

        if std::path::Path::new(&candidate_path).exists() {
            let path = std::fs::canonicalize(&candidate_path)
                .context("Failed to canonicalize worktree path")?;
            return Ok(path.to_string_lossy().to_string());
        }

        Err(WmError::WorktreeNotFound(arg.to_string()).into())
    } else {
        // Use current directory
        Ok(std::env::current_dir()
            .context("Failed to get current directory")?
            .to_string_lossy()
            .to_string())
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use rstest::rstest;
    use tempfile::TempDir;

    use super::*;
    use crate::shared::testing::TestRepo;

    /// Installs an executable `pre-worktree-delete` hook under a fresh
    /// `XDG_CONFIG_HOME` that touches `marker` and exits non-zero, so tests
    /// can assert both "was it invoked" and "does a failing hook still not
    /// block the caller".
    fn install_failing_hook(config_home: &std::path::Path, marker: &std::path::Path) {
        let hooks_dir = config_home.join("armyknife").join("hooks");
        std::fs::create_dir_all(&hooks_dir).unwrap();
        let hook_file = hooks_dir.join("pre-worktree-delete");
        let script = indoc::formatdoc! {"
            #!/bin/sh
            touch {marker}
            exit 1
        ", marker = marker.display()};
        std::fs::write(&hook_file, script).unwrap();
        let mut perms = std::fs::metadata(&hook_file).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&hook_file, perms).unwrap();
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
        let test_repo = TestRepo::new();
        test_repo.create_worktree("feature");
        let repo = test_repo.open();
        let worktree_path = test_repo.worktree_path("feature");

        let config_home = TempDir::new().unwrap();
        let marker = config_home.path().join("marker");
        if hook_installed {
            install_failing_hook(config_home.path(), &marker);
        }

        let hook_ran = temp_env::with_vars(
            [(
                "XDG_CONFIG_HOME",
                Some(config_home.path().to_str().unwrap()),
            )],
            || {
                run_pre_delete_hook(
                    &repo,
                    Some("feature"),
                    worktree_path.to_str().unwrap(),
                    skip_hooks,
                )
            },
        );

        assert_eq!(marker.exists(), expect_invoked);
        assert_eq!(hook_ran, expect_invoked);
    }

    #[test]
    fn resolve_worktree_path_with_existing_path() {
        let test_repo = TestRepo::new();
        test_repo.create_worktree("feature");

        let wt_path = test_repo.worktree_path("feature");
        let result =
            resolve_worktree_path(Some(wt_path.to_str().unwrap()), ".worktrees", "fohte/").unwrap();

        assert_eq!(result, wt_path.to_string_lossy().to_string());
    }

    #[test]
    fn resolve_worktree_path_with_nonexistent_returns_error() {
        let result = resolve_worktree_path(
            Some("/nonexistent/path/to/worktree"),
            ".worktrees",
            "fohte/",
        );
        assert!(result.is_err());
    }

    #[test]
    fn resolve_worktree_path_with_none_returns_current_dir() {
        let current = std::env::current_dir().unwrap();
        let result = resolve_worktree_path(None, ".worktrees", "fohte/").unwrap();

        assert_eq!(result, current.to_string_lossy().to_string());
    }
}
