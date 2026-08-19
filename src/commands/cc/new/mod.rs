use anyhow::{Context, Result};
use clap::Args;
use std::path::{Path, PathBuf};

use super::error::CcError;
use crate::commands::wm::git::branch_to_worktree_name;
use crate::infra::git::cmd::run_git;
use crate::infra::git::fetch_with_prune;
use crate::infra::git::{get_main_branch_for_repo, get_repo_root, get_repo_root_in, open_repo_at};
use crate::shared::config::{Config, load_config};
use crate::shared::env_var::EnvVars;
use crate::shared::hooks;

mod delegation;
mod prompt;
mod tmux;
mod worktree;

use delegation::{build_ancestor_chain, resolve_prompt};
use prompt::{delete_prompt_cache, resolve_args, save_prompt_cache};
use tmux::setup_tmux_window;
use worktree::{
    BranchRollback, WorktreeAddMode, add_worktree_for_branch, git_worktree_add, repo_branch_exists,
    rollback_worktree,
};

/// Args shared between `a cc new` and the deprecated `a wm new` adapter.
#[derive(Args, Clone, PartialEq, Eq)]
pub struct CommonNewArgs {
    /// Initial prompt to send to Claude Code.
    /// When provided without a branch name, the branch name is auto-generated from this prompt.
    #[arg(long)]
    pub prompt: Option<String>,

    /// Mark this invocation as coming from another Claude Code session.
    /// Wraps the prompt with delegation context (branch, base, directories).
    #[arg(long)]
    pub agent: bool,

    /// Label for the new session (displayed in cc watch).
    /// When not specified, the session will get its label via the
    /// user-prompt-submit hook (auto-generation from prompt).
    #[arg(long)]
    pub label: Option<String>,

    /// Parent session ID for tree view hierarchy.
    /// Sets ARMYKNIFE_ANCESTOR_SESSION_IDS for the child session.
    #[arg(long)]
    pub parent_session_id: Option<String>,

    /// Path to the target repository.
    /// When specified, operates on the given repository instead of the current directory.
    #[arg(short = 'R', long)]
    pub repo: Option<PathBuf>,
}

#[derive(Args, Clone, PartialEq, Eq)]
pub struct NewArgs {
    /// Create a worktree for the branch and run the session there
    /// (existing branch will be checked out, non-existing branch will be
    /// created with fohte/ prefix). Value is optional: when omitted, the
    /// branch name is auto-generated from --prompt.
    #[arg(long, required = true, num_args = 0..=1, require_equals = true)]
    pub worktree: Option<String>,

    /// Base branch for new branch creation (requires --worktree;
    /// default: origin/main or origin/master)
    #[arg(long, requires = "worktree")]
    pub from: Option<String>,

    /// Force create new branch even if it already exists (requires --worktree)
    #[arg(long, requires = "worktree")]
    pub force: bool,

    #[command(flatten)]
    pub common: CommonNewArgs,

    /// Skip the post-worktree-create hook (requires --worktree).
    /// Useful when the hook itself is broken and needs to be fixed inside the new worktree.
    #[arg(long, requires = "worktree")]
    pub skip_hooks: bool,
}

pub fn run(args: &NewArgs) -> Result<()> {
    run_inner(args)
}

fn run_inner(args: &NewArgs) -> Result<()> {
    let config = load_config()?;
    let resolved = resolve_args(args)?;
    let name = resolved.branch_name;
    let prompt = resolved.prompt;

    let repo_root = match &args.common.repo {
        Some(path) => get_repo_root_in(path)?,
        None => get_repo_root()?,
    };

    // Save prompt to cache directory for recovery in case of failure
    let prompt_cache_path = prompt
        .as_ref()
        .map(|p| save_prompt_cache(&repo_root, p))
        .transpose()?;

    // Run the actual worktree creation, cleaning up prompt cache on success
    let result = run_worktree_creation(args, &name, prompt.as_deref(), &repo_root, &config);

    if result.is_ok() {
        delete_prompt_cache(&repo_root);
    } else if let Some(path) = prompt_cache_path {
        eprintln!("Prompt saved to: {}", path.display());
    }

    result
}

fn run_worktree_creation(
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
    let (actual_branch, actual_base);
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
        actual_base = base_branch;
    } else if repo_branch_exists(&repo, name) {
        // Branch exists with the exact name provided
        add_worktree_for_branch(&repo, &worktree_dir, name)?;

        actual_branch = name.to_string();
        branch_rollback = BranchRollback::Keep;
        // actual_base is only used when --agent is set
        actual_base = if args.common.agent {
            let main_branch = get_main_branch_for_repo(&repo)?;
            format!("origin/{main_branch}")
        } else {
            String::new()
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
                format!("origin/{main_branch}")
            } else {
                String::new()
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
            actual_base = base_branch;
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
            &actual_branch,
            &actual_base,
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

    // Build environment variables for child session
    let mut env_vars: Vec<(String, String)> = Vec::new();
    if let Some(ref label) = args.common.label {
        env_vars.push((EnvVars::session_label_name().to_string(), label.clone()));
    }
    // Resolve parent session ID: explicit flag > ARMYKNIFE_SESSION_ID env var.
    // ARMYKNIFE_SESSION_ID is set by the SessionStart hook via CLAUDE_ENV_FILE,
    // so `a cc new` called from a Claude Code Bash tool automatically inherits
    // the parent session ID without requiring --parent-session-id.
    let env = EnvVars::load();
    let parent_id = args.common.parent_session_id.clone().or(env.session_id);
    if let Some(ref parent_id) = parent_id {
        let ancestor_chain = build_ancestor_chain(parent_id)?;
        env_vars.push((
            EnvVars::ancestor_session_ids_name().to_string(),
            ancestor_chain,
        ));
    }

    let env_refs: Vec<(&str, &str)> = env_vars
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();

    // Avoid stealing the user's tmux focus when auto-invoked from Claude Code.
    let background = std::env::var("CLAUDECODE").is_ok();

    // Setup tmux window using config layout
    setup_tmux_window(
        repo_root,
        worktree_dir.to_str().unwrap_or(&worktree_name),
        &worktree_name,
        final_prompt.as_deref(),
        config,
        &env_refs,
        background,
    )?;

    let suffix = if background { " (background)" } else { "" };
    println!(
        "Created worktree '{}' and opened tmux window{}",
        worktree_name, suffix
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    use rstest::rstest;

    #[derive(Parser)]
    struct TestCli {
        #[command(flatten)]
        args: NewArgs,
    }

    #[rstest]
    #[case::explicit_value(&["a", "--worktree=my-branch"], Some("my-branch"))]
    #[case::value_omitted(&["a", "--worktree"], None)]
    fn worktree_value_parses(#[case] argv: &[&str], #[case] expected: Option<&str>) {
        let cli = TestCli::try_parse_from(argv).unwrap();
        assert_eq!(cli.args.worktree.as_deref(), expected);
    }

    #[rstest]
    #[case::worktree_missing(&["a"])]
    #[case::from_without_worktree(&["a", "--from", "origin/master"])]
    #[case::force_without_worktree(&["a", "--force"])]
    #[case::skip_hooks_without_worktree(&["a", "--skip-hooks"])]
    fn rejects_missing_or_misplaced_flags(#[case] argv: &[&str]) {
        assert!(TestCli::try_parse_from(argv).is_err());
    }
}
