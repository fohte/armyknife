use anyhow::{Context, bail};
use clap::Args;
use std::path::{Path, PathBuf};

use super::error::{Result, WmError};
use super::git::branch_to_worktree_name;
use crate::commands::cc::store as cc_store;
use crate::commands::name_branch::{detect_backend, generate_branch_name};
use crate::infra::git::GitRepo;
use crate::infra::git::cmd::run_git;
use crate::infra::git::fetch_with_prune;
use crate::infra::git::{get_main_branch_for_repo, get_repo_root, get_repo_root_in, open_repo_at};
use crate::infra::tmux;
use crate::shared::cache;
use crate::shared::command;
use crate::shared::config::{Config, load_config};
use crate::shared::env_var::EnvVars;
use crate::shared::hooks;

/// Get the cache path for prompt recovery.
fn get_prompt_cache_path(repo_root: &str) -> Option<PathBuf> {
    let repo_name = Path::new(repo_root).file_name()?.to_str()?;
    cache::wm_prompt(repo_name)
}

/// Save prompt to cache directory for recovery.
fn save_prompt_cache(repo_root: &str, prompt: &str) -> Result<PathBuf> {
    let path = get_prompt_cache_path(repo_root).context("Failed to determine cache directory")?;
    save_prompt_cache_to(&path, prompt)?;
    Ok(path)
}

/// Internal implementation for saving prompt to a specific path.
/// Allows testing with temporary directories.
fn save_prompt_cache_to(path: &Path, prompt: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).context("Failed to create cache directory")?;
    }

    std::fs::write(path, prompt).context("Failed to save prompt")?;

    Ok(())
}

/// Delete the saved prompt cache after successful completion.
fn delete_prompt_cache(repo_root: &str) {
    if let Some(path) = get_prompt_cache_path(repo_root) {
        delete_prompt_cache_at(&path);
    }
}

/// Internal implementation for deleting prompt cache at a specific path.
fn delete_prompt_cache_at(path: &Path) {
    let _ = std::fs::remove_file(path);
}

/// Open $EDITOR to let user input a prompt.
/// Returns the prompt text, or None if the user didn't provide any input.
fn open_editor_for_prompt() -> Result<Option<String>> {
    let editor = std::env::var("EDITOR").unwrap_or_else(|_| "vi".to_string());

    // Create an empty temp file for the prompt
    let temp_file = tempfile::Builder::new()
        .prefix("wm-prompt-")
        .suffix(".md")
        .tempfile()
        .context("Failed to create temp file")?;

    let temp_path = temp_file.path().to_path_buf();

    // Launch editor
    let status = command::new(&editor)
        .arg(&temp_path)
        .status()
        .with_context(|| format!("Failed to launch editor '{editor}'"))?;

    if !status.success() {
        bail!("Editor exited with status: {status}");
    }

    // Read the content
    let content = std::fs::read_to_string(&temp_path).context("Failed to read temp file")?;

    let prompt = content.trim().to_string();

    if prompt.is_empty() {
        Ok(None)
    } else {
        Ok(Some(prompt))
    }
}

/// Mode for creating a worktree
enum WorktreeAddMode<'a> {
    /// Checkout an existing local branch
    LocalBranch { branch: &'a str },
    /// Create a tracking branch from remote
    TrackRemote { branch: &'a str },
    /// Create a new branch from base
    NewBranch { branch: &'a str, base: &'a str },
    /// Force create/reset a branch from base
    ForceNewBranch { branch: &'a str, base: &'a str },
}

/// Run `git worktree add` with the specified mode.
fn git_worktree_add(repo: &GitRepo, worktree_dir: &Path, mode: WorktreeAddMode) -> Result<()> {
    let path = worktree_dir.to_str().context("Invalid worktree path")?;

    match mode {
        WorktreeAddMode::LocalBranch { branch } => {
            run_git(repo.workdir(), ["worktree", "add", path, branch])
                .context("Failed to add worktree")?;
        }
        WorktreeAddMode::TrackRemote { branch } => {
            let remote_name = format!("origin/{branch}");
            run_git(
                repo.workdir(),
                [
                    "worktree",
                    "add",
                    "--track",
                    "-b",
                    branch,
                    path,
                    &remote_name,
                ],
            )
            .context("Failed to add worktree")?;
        }
        WorktreeAddMode::NewBranch { branch, base } => {
            run_git(
                repo.workdir(),
                ["worktree", "add", "-b", branch, path, base],
            )
            .context("Failed to add worktree")?;
        }
        WorktreeAddMode::ForceNewBranch { branch, base } => {
            // `--force -B` overrides both "branch already used by worktree"
            // and "destination path exists" checks so the reset succeeds even
            // when the branch is checked out elsewhere.
            run_git(
                repo.workdir(),
                ["worktree", "add", "--force", "-B", branch, path, base],
            )
            .context("Failed to add worktree")?;
        }
    }

    Ok(())
}

/// Check if a branch exists (local or remote) in the given repository.
fn repo_branch_exists(repo: &GitRepo, branch: &str) -> bool {
    repo.local_branch_exists(branch) || repo.remote_branch_exists(&format!("origin/{branch}"))
}

/// Add a worktree for an existing branch (local or remote)
fn add_worktree_for_branch(repo: &GitRepo, worktree_dir: &Path, branch: &str) -> Result<()> {
    if repo.local_branch_exists(branch) {
        git_worktree_add(repo, worktree_dir, WorktreeAddMode::LocalBranch { branch })
    } else if repo.remote_branch_exists(&format!("origin/{branch}")) {
        git_worktree_add(repo, worktree_dir, WorktreeAddMode::TrackRemote { branch })
    } else {
        // Fallback: use as-is (should not normally happen)
        git_worktree_add(repo, worktree_dir, WorktreeAddMode::LocalBranch { branch })
    }
}

#[derive(Args, Clone, PartialEq, Eq)]
pub struct NewArgs {
    /// Branch name (existing branch will be checked out,
    /// non-existing branch will be created with fohte/ prefix).
    /// Optional when --prompt is provided (auto-generated from prompt).
    pub name: Option<String>,

    /// Base branch for new branch creation (default: origin/main or origin/master)
    #[arg(long)]
    pub from: Option<String>,

    /// Force create new branch even if it already exists
    #[arg(long)]
    pub force: bool,

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

    /// Skip the post-worktree-create hook.
    /// Useful when the hook itself is broken and needs to be fixed inside the new worktree.
    #[arg(long)]
    pub skip_hooks: bool,
}

/// Context information injected into the prompt when --agent is used
struct DelegationContext<'a> {
    branch: &'a str,
    base: &'a str,
    delegator_cwd: &'a str,
    worktree_cwd: &'a str,
}

/// Resolve the final prompt, optionally wrapping it with delegation context.
/// When `agent` is true, wraps the prompt in a `<delegated-task>` XML envelope.
fn resolve_prompt(
    agent: bool,
    prompt: Option<&str>,
    branch: &str,
    base: &str,
    delegator_cwd: &str,
    worktree_cwd: &str,
) -> Option<String> {
    match (agent, prompt) {
        (true, Some(p)) => Some(build_delegated_prompt(
            p,
            &DelegationContext {
                branch,
                base,
                delegator_cwd,
                worktree_cwd,
            },
        )),
        (_, p) => p.map(String::from),
    }
}

/// Build a delegated prompt by wrapping the original prompt with context XML
fn build_delegated_prompt(prompt: &str, ctx: &DelegationContext) -> String {
    indoc::formatdoc! {"
        <delegated-task>
        <context>
        - Source: Delegated from another Claude Code session
        - Branch: {branch}
        - Base: {base}
        - Delegator CWD: {delegator_cwd}
        - Worktree CWD: {worktree_cwd}
        </context>
        <instructions>
        {prompt}
        </instructions>
        </delegated-task>",
        branch = ctx.branch,
        base = ctx.base,
        delegator_cwd = ctx.delegator_cwd,
        worktree_cwd = ctx.worktree_cwd,
        prompt = prompt,
    }
    .trim_start()
    .to_string()
}

/// Build comma-separated ancestor chain for a child session.
///
/// Loads the parent session from the store and prepends its own ancestor chain,
/// producing: `grandparent_id,parent_id` (root-to-immediate-parent order).
/// Falls back to just the parent_session_id if the parent session cannot be loaded.
fn build_ancestor_chain(parent_session_id: &str) -> Result<String> {
    match cc_store::load_session(parent_session_id) {
        Ok(Some(parent)) => {
            let mut ancestors = parent.ancestor_session_ids;
            ancestors.push(parent_session_id.to_string());
            Ok(ancestors.join(","))
        }
        _ => {
            // Parent session not found or load error; use parent_id alone
            Ok(parent_session_id.to_string())
        }
    }
}

/// Resolved branch name and prompt information
#[derive(Debug)]
struct ResolvedArgs {
    branch_name: String,
    prompt: Option<String>,
}

/// Resolve branch name: use provided name or generate from prompt.
/// If no name and no prompt provided, opens editor to get prompt.
fn resolve_args(args: &NewArgs) -> Result<ResolvedArgs> {
    resolve_args_with_deps(args, || detect_backend(), open_editor_for_prompt)
}

/// Internal implementation that accepts dependencies for testability.
fn resolve_args_with_deps<F, E>(
    args: &NewArgs,
    backend_factory: F,
    editor_fn: E,
) -> Result<ResolvedArgs>
where
    F: FnOnce() -> Box<dyn crate::commands::name_branch::Backend>,
    E: FnOnce() -> Result<Option<String>>,
{
    match (&args.name, &args.prompt) {
        (Some(name), prompt) => Ok(ResolvedArgs {
            branch_name: name.clone(),
            prompt: prompt.clone(),
        }),
        (None, Some(prompt)) => {
            let backend = backend_factory();
            let generated = generate_branch_name(prompt, backend.as_ref())?;
            Ok(ResolvedArgs {
                branch_name: generated,
                prompt: Some(prompt.clone()),
            })
        }
        (None, None) => {
            // Open editor to get prompt
            let prompt = editor_fn()?.ok_or(WmError::Cancelled)?;
            let backend = backend_factory();
            let generated = generate_branch_name(&prompt, backend.as_ref())?;
            Ok(ResolvedArgs {
                branch_name: generated,
                prompt: Some(prompt),
            })
        }
    }
}

pub fn run(args: &NewArgs) -> Result<()> {
    run_inner(args)
}

fn run_inner(args: &NewArgs) -> Result<()> {
    let config = load_config()?;
    let resolved = resolve_args(args)?;
    let name = resolved.branch_name;
    let prompt = resolved.prompt;

    let repo_root = match &args.repo {
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
    let repo = open_repo_at(Path::new(repo_root)).map_err(|_| WmError::NotInGitRepo)?;
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
        actual_base = if args.agent {
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
            actual_base = if args.agent {
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
    let final_prompt = if args.agent {
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
    if let Some(ref label) = args.label {
        env_vars.push((EnvVars::session_label_name().to_string(), label.clone()));
    }
    // Resolve parent session ID: explicit flag > ARMYKNIFE_SESSION_ID env var.
    // ARMYKNIFE_SESSION_ID is set by the SessionStart hook via CLAUDE_ENV_FILE,
    // so `a wm new` called from a Claude Code Bash tool automatically inherits
    // the parent session ID without requiring --parent-session-id.
    let env = EnvVars::load();
    let parent_id = args.parent_session_id.clone().or(env.session_id);
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

/// How to roll back the branch associated with a worktree after a
/// post-worktree-create hook failure.
enum BranchRollback {
    /// Branch was created in this invocation; delete it.
    Delete,
    /// Branch pre-existed and was not modified; leave it alone.
    Keep,
    /// Branch was force-reset to a new base; restore its previous tip.
    RestoreTip(String),
}

fn rollback_worktree(
    repo: &GitRepo,
    worktree_name: &str,
    branch: &str,
    branch_rollback: &BranchRollback,
) {
    eprintln!("post-worktree-create hook failed; rolling back worktree '{worktree_name}'");

    let removed = match super::worktree::delete_worktree(repo, worktree_name) {
        Ok(true) => true,
        Ok(false) => {
            eprintln!(
                "warning: worktree '{worktree_name}' could not be removed. \
                 Run `a wm delete` or remove it manually before re-running `a wm new`."
            );
            false
        }
        Err(e) => {
            eprintln!(
                "warning: failed to remove worktree '{worktree_name}': {e}. \
                 Run `a wm delete` or remove it manually before re-running `a wm new`."
            );
            false
        }
    };

    // Skip branch mutation while the worktree still references it; git would
    // refuse the delete/update anyway, and leaving state intact lets the user
    // retry after manual worktree cleanup.
    if !removed {
        if let BranchRollback::RestoreTip(tip) = branch_rollback {
            eprintln!(
                "warning: branch '{branch}' was force-reset and cannot be restored \
                 while the worktree remains. After removing the worktree, run \
                 `git update-ref refs/heads/{branch} {tip}`."
            );
        }
        return;
    }

    match branch_rollback {
        BranchRollback::Delete => {
            if super::worktree::delete_branch_if_exists(repo, branch) {
                eprintln!("Deleted branch '{branch}'");
            }
        }
        BranchRollback::Keep => {}
        BranchRollback::RestoreTip(tip) => {
            match run_git(
                repo.workdir(),
                ["update-ref", &format!("refs/heads/{branch}"), tip],
            ) {
                Ok(_) => eprintln!("Restored branch '{branch}' to {tip}"),
                Err(e) => eprintln!(
                    "warning: failed to restore branch '{branch}' to {tip}: {e}. \
                     Recover with `git update-ref refs/heads/{branch} {tip}` or `git reflog`."
                ),
            }
        }
    }
}

/// Setup a tmux window using the configured layout.
fn setup_tmux_window(
    repo_root: &str,
    worktree_dir: &str,
    worktree_name: &str,
    prompt: Option<&str>,
    config: &Config,
    env_vars: &[(&str, &str)],
    background: bool,
) -> Result<()> {
    let target_session = tmux::get_session_name(repo_root, &config.wm.worktrees_dir);

    tmux::ensure_session(&target_session, repo_root).context("Failed to ensure tmux session")?;

    tmux::layout::build_layout(
        &target_session,
        worktree_dir,
        worktree_name,
        &config.wm.layout,
        prompt,
        env_vars,
        background,
    )
    .context("Failed to create tmux layout")?;

    if !background {
        tmux::switch_to_session(&target_session).context("Failed to switch to tmux session")?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shared::testing::TestRepo;
    use indoc::indoc;
    use std::process::Command;
    use tempfile::TempDir;

    fn git_in(dir: &Path, args: &[&str]) {
        let status = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .env("GIT_AUTHOR_NAME", "Test")
            .env("GIT_AUTHOR_EMAIL", "test@example.com")
            .env("GIT_COMMITTER_NAME", "Test")
            .env("GIT_COMMITTER_EMAIL", "test@example.com")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null")
            .status()
            .unwrap_or_else(|e| panic!("git {args:?}: {e}"));
        assert!(status.success(), "git {args:?} failed");
    }

    #[test]
    fn git_worktree_add_creates_worktree_with_new_branch() {
        let test_repo = TestRepo::new();
        let repo = test_repo.open();

        let worktrees_dir = test_repo.path().join(".worktrees");
        std::fs::create_dir_all(&worktrees_dir).unwrap();

        let worktree_dir = worktrees_dir.join("test-branch");
        git_worktree_add(
            &repo,
            &worktree_dir,
            WorktreeAddMode::NewBranch {
                branch: "test-branch",
                base: "HEAD",
            },
        )
        .unwrap();

        assert!(worktree_dir.exists());
        assert!(repo.local_branch_exists("test-branch"));
    }

    #[test]
    fn git_worktree_add_with_local_branch() {
        let test_repo = TestRepo::new();
        let repo = test_repo.open();

        git_in(&test_repo.path(), &["branch", "existing-branch"]);

        let worktrees_dir = test_repo.path().join(".worktrees");
        std::fs::create_dir_all(&worktrees_dir).unwrap();

        let worktree_dir = worktrees_dir.join("existing-branch");
        git_worktree_add(
            &repo,
            &worktree_dir,
            WorktreeAddMode::LocalBranch {
                branch: "existing-branch",
            },
        )
        .unwrap();

        assert!(worktree_dir.exists());
    }

    #[test]
    fn git_worktree_add_force_resets_existing_branch() {
        let test_repo = TestRepo::new();
        let repo = test_repo.open();

        git_in(&test_repo.path(), &["branch", "force-test"]);
        git_in(
            &test_repo.path(),
            &["commit", "--allow-empty", "-q", "-m", "Second commit"],
        );

        let worktrees_dir = test_repo.path().join(".worktrees");
        std::fs::create_dir_all(&worktrees_dir).unwrap();

        let worktree_dir = worktrees_dir.join("force-test");
        git_worktree_add(
            &repo,
            &worktree_dir,
            WorktreeAddMode::ForceNewBranch {
                branch: "force-test",
                base: "HEAD",
            },
        )
        .unwrap();

        assert!(worktree_dir.exists());
    }

    #[rstest]
    #[case::local_exists("existing-local", true)]
    #[case::remote_exists("existing-remote", true)]
    #[case::nonexistent("nonexistent", false)]
    fn repo_branch_exists_checks_local_and_remote(#[case] branch: &str, #[case] expected: bool) {
        let test_repo = TestRepo::new();
        let repo = test_repo.open();

        git_in(&test_repo.path(), &["branch", "existing-local"]);
        let head = run_git(&test_repo.path(), ["rev-parse", "HEAD"]).unwrap();
        run_git(
            &test_repo.path(),
            ["update-ref", "refs/remotes/origin/existing-remote", &head],
        )
        .unwrap();

        assert_eq!(repo_branch_exists(&repo, branch), expected);
    }

    #[rstest]
    fn add_worktree_for_branch_uses_local_branch() {
        let test_repo = TestRepo::new();
        let repo = test_repo.open();
        git_in(&test_repo.path(), &["branch", "local-branch"]);

        let worktrees_dir = test_repo.path().join(".worktrees");
        std::fs::create_dir_all(&worktrees_dir).unwrap();
        let worktree_dir = worktrees_dir.join("local-branch");

        add_worktree_for_branch(&repo, &worktree_dir, "local-branch").unwrap();
        assert!(worktree_dir.exists());
    }

    #[rstest]
    fn add_worktree_for_branch_tracks_remote_branch() {
        let test_repo = TestRepo::new();
        let repo = test_repo.open();
        let repo_path = test_repo.path();

        // Simulate a remote-tracking branch without spawning a real remote:
        // point refs/remotes/origin/remote-branch at HEAD, and register `origin`
        // so `git worktree add --track` can set the upstream config.
        let head = run_git(&repo_path, ["rev-parse", "HEAD"]).unwrap();
        run_git(
            &repo_path,
            ["update-ref", "refs/remotes/origin/remote-branch", &head],
        )
        .unwrap();
        run_git(&repo_path, ["config", "remote.origin.url", "."]).unwrap();
        run_git(
            &repo_path,
            [
                "config",
                "remote.origin.fetch",
                "+refs/heads/*:refs/remotes/origin/*",
            ],
        )
        .unwrap();

        let worktrees_dir = repo_path.join(".worktrees");
        std::fs::create_dir_all(&worktrees_dir).unwrap();
        let worktree_dir = worktrees_dir.join("remote-branch");

        add_worktree_for_branch(&repo, &worktree_dir, "remote-branch").unwrap();
        assert!(worktree_dir.exists());
        assert!(repo.local_branch_exists("remote-branch"));
    }

    use crate::commands::name_branch::{Backend, Result as NameBranchResult};
    use rstest::{fixture, rstest};

    struct RollbackEnv {
        // hold the TempDir so files survive the test
        _test_repo: TestRepo,
        repo: GitRepo,
        worktree_dir: PathBuf,
    }

    #[fixture]
    fn rollback_env() -> RollbackEnv {
        let test_repo = TestRepo::new();
        let repo = test_repo.open();
        let worktrees_dir = test_repo.path().join(".worktrees");
        std::fs::create_dir_all(&worktrees_dir).unwrap();
        let worktree_dir = worktrees_dir.join("rollback-branch");
        RollbackEnv {
            _test_repo: test_repo,
            repo,
            worktree_dir,
        }
    }

    #[rstest]
    fn rollback_worktree_deletes_created_branch(rollback_env: RollbackEnv) {
        git_worktree_add(
            &rollback_env.repo,
            &rollback_env.worktree_dir,
            WorktreeAddMode::NewBranch {
                branch: "rollback-branch",
                base: "HEAD",
            },
        )
        .unwrap();

        rollback_worktree(
            &rollback_env.repo,
            "rollback-branch",
            "rollback-branch",
            &BranchRollback::Delete,
        );

        assert!(!rollback_env.worktree_dir.exists());
        assert!(!rollback_env.repo.local_branch_exists("rollback-branch"));
    }

    #[rstest]
    fn rollback_worktree_keeps_preexisting_branch(rollback_env: RollbackEnv) {
        git_in(rollback_env.repo.workdir(), &["branch", "rollback-branch"]);

        add_worktree_for_branch(
            &rollback_env.repo,
            &rollback_env.worktree_dir,
            "rollback-branch",
        )
        .unwrap();

        rollback_worktree(
            &rollback_env.repo,
            "rollback-branch",
            "rollback-branch",
            &BranchRollback::Keep,
        );

        assert!(!rollback_env.worktree_dir.exists());
        assert!(rollback_env.repo.local_branch_exists("rollback-branch"));
    }

    #[rstest]
    fn rollback_worktree_restores_force_reset_branch_tip(rollback_env: RollbackEnv) {
        let workdir = rollback_env.repo.workdir().to_path_buf();
        let original_tip = run_git(&workdir, ["rev-parse", "HEAD"]).unwrap();

        git_worktree_add(
            &rollback_env.repo,
            &rollback_env.worktree_dir,
            WorktreeAddMode::NewBranch {
                branch: "rollback-branch",
                base: "HEAD",
            },
        )
        .unwrap();

        let output = Command::new("git")
            .arg("-C")
            .arg(&workdir)
            .args([
                "commit-tree",
                "-m",
                "advance",
                &format!("{original_tip}^{{tree}}"),
            ])
            .env("GIT_AUTHOR_NAME", "Test")
            .env("GIT_AUTHOR_EMAIL", "test@example.com")
            .env("GIT_COMMITTER_NAME", "Test")
            .env("GIT_COMMITTER_EMAIL", "test@example.com")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null")
            .output()
            .unwrap();
        assert!(output.status.success(), "commit-tree failed");
        let advanced_tip = String::from_utf8(output.stdout).unwrap().trim().to_string();
        run_git(
            &workdir,
            ["update-ref", "refs/heads/rollback-branch", &advanced_tip],
        )
        .unwrap();
        assert_ne!(original_tip, advanced_tip);

        rollback_worktree(
            &rollback_env.repo,
            "rollback-branch",
            "rollback-branch",
            &BranchRollback::RestoreTip(original_tip.clone()),
        );

        assert!(!rollback_env.worktree_dir.exists());
        assert!(rollback_env.repo.local_branch_exists("rollback-branch"));
        assert_eq!(
            run_git(&workdir, ["rev-parse", "rollback-branch"]).unwrap(),
            original_tip,
        );
    }

    /// Mock backend for testing
    struct MockBackend {
        response: String,
    }

    impl Backend for MockBackend {
        fn generate(&self, _prompt: &str) -> NameBranchResult<String> {
            Ok(self.response.clone())
        }
    }

    fn mock_backend(response: &str) -> Box<dyn Backend> {
        Box::new(MockBackend {
            response: response.to_string(),
        })
    }

    #[rstest]
    #[case::explicit_name(Some("my-branch"), None, "my-branch", None)]
    #[case::name_takes_priority_over_prompt(
        Some("my-branch"),
        Some("some task"),
        "my-branch",
        Some("some task")
    )]
    #[case::generate_from_prompt(
        None,
        Some("fix login bug"),
        "fix-login-bug",
        Some("fix login bug")
    )]
    fn resolve_args_returns_expected(
        #[case] name: Option<&str>,
        #[case] prompt: Option<&str>,
        #[case] expected_branch: &str,
        #[case] expected_prompt: Option<&str>,
    ) {
        let args = NewArgs {
            name: name.map(String::from),
            from: None,
            force: false,
            prompt: prompt.map(String::from),
            agent: false,
            label: None,
            parent_session_id: None,
            repo: None,
            skip_hooks: false,
        };
        let result = resolve_args_with_deps(
            &args,
            || mock_backend("fix-login-bug"),
            || panic!("editor should not be called"),
        )
        .unwrap();

        assert_eq!(result.branch_name, expected_branch);
        assert_eq!(result.prompt.as_deref(), expected_prompt);
    }

    #[rstest]
    #[case::editor_returns_prompt(
        Some("prompt from editor"),
        Ok(("editor-branch", Some("prompt from editor")))
    )]
    #[case::editor_returns_empty(None, Err(true))]
    fn resolve_args_with_editor(
        #[case] editor_input: Option<&str>,
        #[case] expected: std::result::Result<(&str, Option<&str>), bool>,
    ) {
        let args = NewArgs {
            name: None,
            from: None,
            force: false,
            prompt: None,
            agent: false,
            label: None,
            parent_session_id: None,
            repo: None,
            skip_hooks: false,
        };
        let result = resolve_args_with_deps(
            &args,
            || mock_backend("editor-branch"),
            || Ok(editor_input.map(String::from)),
        );

        match expected {
            Ok((branch, prompt)) => {
                let resolved = result.unwrap();
                assert_eq!(resolved.branch_name, branch);
                assert_eq!(resolved.prompt.as_deref(), prompt);
            }
            Err(_) => {
                let err = result.unwrap_err();
                assert!(
                    err.downcast_ref::<WmError>()
                        .is_some_and(|e| matches!(e, WmError::Cancelled))
                );
            }
        }
    }

    #[rstest]
    #[case::extracts_repo_name("/home/user/projects/my-repo", Some("my-repo"))]
    #[case::root_returns_none("/", None)]
    fn get_prompt_cache_path_behavior(
        #[case] repo_root: &str,
        #[case] expected_repo: Option<&str>,
    ) {
        let path = get_prompt_cache_path(repo_root);
        match expected_repo {
            Some(repo) => {
                let p = path.unwrap();
                assert!(p.ends_with("prompt.md"));
                assert!(p.parent().unwrap().ends_with(repo));
            }
            None => assert!(path.is_none()),
        }
    }

    #[rstest]
    fn save_and_delete_prompt_cache() {
        let temp_dir = TempDir::new().unwrap();
        let cache_path = temp_dir
            .path()
            .join("wm")
            .join("test-repo")
            .join("prompt.md");

        let prompt = "test prompt content";
        save_prompt_cache_to(&cache_path, prompt).unwrap();

        // File should exist with correct content
        assert!(cache_path.exists());
        assert_eq!(std::fs::read_to_string(&cache_path).unwrap(), prompt);

        // Delete should remove the file
        delete_prompt_cache_at(&cache_path);
        assert!(!cache_path.exists());
    }

    #[rstest]
    fn save_prompt_cache_creates_parent_directories() {
        let temp_dir = TempDir::new().unwrap();
        let cache_path = temp_dir
            .path()
            .join("wm")
            .join("nested")
            .join("path")
            .join("repo")
            .join("prompt.md");

        save_prompt_cache_to(&cache_path, "test").unwrap();
        assert!(cache_path.exists());
    }

    #[rstest]
    fn delete_prompt_cache_does_not_fail_for_nonexistent() {
        let temp_dir = TempDir::new().unwrap();
        let repo_root = temp_dir.path().join("nonexistent-repo");
        std::fs::create_dir_all(&repo_root).unwrap();

        // Should not panic or error
        delete_prompt_cache(repo_root.to_str().unwrap());
    }

    #[rstest]
    #[case::single_line(
        "fohte/fix-auth-bug",
        "origin/master",
        "/home/user/repo",
        "/home/user/repo/.worktrees/fix-auth-bug",
        "Fix the auth bug",
        indoc! {"
            <delegated-task>
            <context>
            - Source: Delegated from another Claude Code session
            - Branch: fohte/fix-auth-bug
            - Base: origin/master
            - Delegator CWD: /home/user/repo
            - Worktree CWD: /home/user/repo/.worktrees/fix-auth-bug
            </context>
            <instructions>
            Fix the auth bug
            </instructions>
            </delegated-task>"},
    )]
    #[case::multiline_prompt(
        "fohte/feature-x",
        "origin/main",
        "/tmp/repo",
        "/tmp/repo/.worktrees/feature-x",
        indoc! {"
            ## Background
            Some context

            ## Goal
            Implement feature X"},
        indoc! {"
            <delegated-task>
            <context>
            - Source: Delegated from another Claude Code session
            - Branch: fohte/feature-x
            - Base: origin/main
            - Delegator CWD: /tmp/repo
            - Worktree CWD: /tmp/repo/.worktrees/feature-x
            </context>
            <instructions>
            ## Background
            Some context

            ## Goal
            Implement feature X
            </instructions>
            </delegated-task>"},
    )]
    fn build_delegated_prompt_wraps_with_xml(
        #[case] branch: &str,
        #[case] base: &str,
        #[case] delegator_cwd: &str,
        #[case] worktree_cwd: &str,
        #[case] prompt: &str,
        #[case] expected: &str,
    ) {
        let ctx = DelegationContext {
            branch,
            base,
            delegator_cwd,
            worktree_cwd,
        };
        let result = build_delegated_prompt(prompt.trim_start(), &ctx);

        assert_eq!(result, expected.trim_start());
    }

    #[rstest]
    #[case::agent_wraps_prompt(true, Some("do something"), true)]
    #[case::no_agent_passes_through(false, Some("do something"), false)]
    #[case::agent_without_prompt(true, None, false)]
    #[case::no_agent_no_prompt(false, None, false)]
    fn resolve_prompt_wraps_only_when_agent_and_prompt(
        #[case] agent: bool,
        #[case] prompt: Option<&str>,
        #[case] expect_wrapped: bool,
    ) {
        let result = resolve_prompt(
            agent,
            prompt,
            "fohte/test",
            "origin/main",
            "/cwd",
            "/worktree",
        );

        match (prompt, expect_wrapped) {
            (None, _) => assert_eq!(result, None),
            (Some(p), true) => {
                let expected = build_delegated_prompt(
                    p,
                    &DelegationContext {
                        branch: "fohte/test",
                        base: "origin/main",
                        delegator_cwd: "/cwd",
                        worktree_cwd: "/worktree",
                    },
                );
                assert_eq!(result, Some(expected));
            }
            (Some(p), false) => {
                assert_eq!(result, Some(p.to_string()));
            }
        }
    }
}
