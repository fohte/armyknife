use anyhow::{Context, Result};
use std::path::Path;

use super::NewArgs;
use super::delegation::resolve_prompt;
use super::tmux::{TmuxWindowSpec, setup_tmux_window};
use super::tmux_launch_inputs;
use super::worktree::{
    BranchRollback, WorktreeAddMode, add_worktree_for_branch, git_worktree_add, repo_branch_exists,
    rollback_worktree,
};
use crate::commands::cc::error::CcError;
use crate::commands::wm::git::branch_to_worktree_name;
use crate::infra::git::cmd::run_git;
use crate::infra::git::fetch_with_prune;
use crate::infra::git::{get_main_branch_for_repo, open_repo_at};
use crate::shared::config::Config;
use crate::shared::env_var::EnvVars;
use crate::shared::hooks;

pub(super) fn run_worktree_creation(
    args: &NewArgs,
    name: &str,
    prompt: Option<&str>,
    repo_root: &str,
    config: &Config,
) -> Result<()> {
    let repo = open_repo_at(Path::new(repo_root)).map_err(|_| CcError::NotInGitRepo)?;
    let branch_prefix = &config.wm.branch_prefix;

    // Determine worktree directory name from branch name
    let worktree_name = branch_to_worktree_name(name, branch_prefix);
    let worktrees_dir = format!("{repo_root}/{}", config.wm.worktrees_dir);
    let worktree_dir = Path::new(&worktrees_dir).join(&worktree_name);

    // Ensure worktrees directory exists
    std::fs::create_dir_all(&worktrees_dir).context("Failed to create worktrees directory")?;

    // Fetch with prune
    fetch_with_prune(&repo).context("Failed to fetch from remote")?;

    // Remove branch prefix to avoid double prefix
    let name_no_prefix = name.strip_prefix(branch_prefix).unwrap_or(name);

    // Determine action based on branch existence and flags.
    // Track the resolved branch/base for --agent context injection.
    // actual_base is only populated when --agent is set (except when a new
    // branch is created, where it is always known).
    let actual_branch;
    let actual_base: Option<String>;
    let branch_rollback;

    if args.force {
        // Force create new branch with prefix
        let main_branch = get_main_branch_for_repo(&repo)?;
        let base_branch = args
            .from
            .clone()
            .unwrap_or_else(|| format!("origin/{main_branch}"));
        let branch = format!("{branch_prefix}{name_no_prefix}");

        // ForceNewBranch resets a pre-existing local branch's tip; capture
        // it so rollback can restore the user's branch to its previous
        // commit on hook failure. Without the prior tip we cannot undo the
        // reset safely, so refuse rather than risk silent branch loss.
        branch_rollback = if repo.local_branch_exists(&branch) {
            let tip = run_git(repo.workdir(), ["rev-parse", &branch]).with_context(|| {
                format!("Failed to capture tip of '{branch}' before force reset")
            })?;
            BranchRollback::RestoreTip(tip)
        } else {
            BranchRollback::Delete
        };

        git_worktree_add(
            &repo,
            &worktree_dir,
            WorktreeAddMode::ForceNewBranch {
                branch: &branch,
                base: &base_branch,
            },
        )?;

        actual_branch = branch;
        actual_base = Some(base_branch);
    } else if repo_branch_exists(&repo, name) {
        // Branch exists with the exact name provided
        add_worktree_for_branch(&repo, &worktree_dir, name)?;

        actual_branch = name.to_string();
        branch_rollback = BranchRollback::Keep;
        actual_base = if args.common.agent {
            let main_branch = get_main_branch_for_repo(&repo)?;
            Some(format!("origin/{main_branch}"))
        } else {
            None
        };
    } else {
        let branch_with_prefix = format!("{branch_prefix}{name_no_prefix}");
        if repo_branch_exists(&repo, &branch_with_prefix) {
            // Branch exists with prefix
            add_worktree_for_branch(&repo, &worktree_dir, &branch_with_prefix)?;

            actual_branch = branch_with_prefix;
            branch_rollback = BranchRollback::Keep;
            actual_base = if args.common.agent {
                let main_branch = get_main_branch_for_repo(&repo)?;
                Some(format!("origin/{main_branch}"))
            } else {
                None
            };
        } else {
            // Branch doesn't exist, create new one with prefix
            let main_branch = get_main_branch_for_repo(&repo)?;
            let base_branch = args
                .from
                .clone()
                .unwrap_or_else(|| format!("origin/{main_branch}"));
            let branch = format!("{branch_prefix}{name_no_prefix}");

            git_worktree_add(
                &repo,
                &worktree_dir,
                WorktreeAddMode::NewBranch {
                    branch: &branch,
                    base: &base_branch,
                },
            )?;

            actual_branch = branch;
            actual_base = Some(base_branch);
            branch_rollback = BranchRollback::Delete;
        }
    }

    // Wrap prompt with delegation context when --agent is used
    let final_prompt = if args.common.agent {
        let delegator_cwd = std::env::current_dir()
            .context("Failed to get current directory")?
            .to_string_lossy()
            .to_string();
        let worktree_cwd_str = worktree_dir
            .to_str()
            .context("Invalid worktree path")?
            .to_string();

        resolve_prompt(
            true,
            prompt,
            Some(&actual_branch),
            actual_base.as_deref(),
            &delegator_cwd,
            &worktree_cwd_str,
        )
    } else {
        prompt.map(String::from)
    };

    // Run post-worktree-create hook. Hook failures roll back the worktree
    // (and the branch, if we created it) before propagating the error.
    if args.skip_hooks {
        eprintln!("Skipping post-worktree-create hook (--skip-hooks)");
    } else {
        let worktree_abs =
            std::fs::canonicalize(&worktree_dir).unwrap_or_else(|_| worktree_dir.to_path_buf());
        if let Err(hook_err) = hooks::run_hook(
            "post-worktree-create",
            &[
                (
                    EnvVars::worktree_path_name(),
                    &worktree_abs.to_string_lossy(),
                ),
                (EnvVars::branch_name_name(), &actual_branch),
                (EnvVars::repo_root_name(), repo_root),
            ],
        ) {
            rollback_worktree(&repo, &worktree_name, &actual_branch, &branch_rollback);
            return Err(hook_err);
        }
    }

    let (env_vars, background) = tmux_launch_inputs(&args.common)?;
    let env_refs: Vec<(&str, &str)> = env_vars
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();

    // Setup tmux window using config layout
    setup_tmux_window(
        TmuxWindowSpec {
            repo_root,
            cwd: worktree_dir.to_str().unwrap_or(&worktree_name),
            window_name: &worktree_name,
            layout: &config.wm.layout,
            model: args.common.model.as_deref(),
            prompt: final_prompt.as_deref(),
            env_vars: &env_refs,
            background,
            restore_automatic_rename: false,
        },
        config,
    )?;

    let suffix = if background { " (background)" } else { "" };
    println!(
        "Created worktree '{}' and opened tmux window{}",
        worktree_name, suffix
    );

    Ok(())
}
