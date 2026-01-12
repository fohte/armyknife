use clap::Args;
use git2::{BranchType, Repository, WorktreeAddOptions};
use std::path::Path;

use super::error::{Result, WmError};
use super::git::{
    BRANCH_PREFIX, branch_exists, branch_to_worktree_name, get_main_branch, get_repo_root,
    local_branch_exists, remote_branch_exists,
};
use crate::git::fetch_with_prune;
use crate::name_branch::{detect_backend, generate_branch_name};
use crate::tmux;

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

/// Run `git worktree add` with the specified mode using git2
fn git_worktree_add(repo: &Repository, worktree_dir: &Path, mode: WorktreeAddMode) -> Result<()> {
    let worktree_name = worktree_dir
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| WmError::CommandFailed("Invalid worktree path".into()))?;

    match mode {
        WorktreeAddMode::LocalBranch { branch } => {
            // Checkout existing local branch
            let local_branch = repo
                .find_branch(branch, BranchType::Local)
                .map_err(|e| WmError::CommandFailed(format!("Failed to find branch: {e}")))?;
            let reference = local_branch.into_reference();

            let mut opts = WorktreeAddOptions::new();
            opts.reference(Some(&reference));

            repo.worktree(worktree_name, worktree_dir, Some(&opts))
                .map_err(|e| WmError::CommandFailed(format!("Failed to add worktree: {e}")))?;
        }
        WorktreeAddMode::TrackRemote { branch } => {
            // Create a tracking branch from remote
            let remote_name = format!("origin/{branch}");
            let remote_branch =
                repo.find_branch(&remote_name, BranchType::Remote)
                    .map_err(|e| {
                        WmError::CommandFailed(format!("Failed to find remote branch: {e}"))
                    })?;

            let commit = remote_branch.get().peel_to_commit().map_err(|e| {
                WmError::CommandFailed(format!("Failed to get commit from remote branch: {e}"))
            })?;

            // Create local branch tracking remote
            let mut local_branch = repo
                .branch(branch, &commit, false)
                .map_err(|e| WmError::CommandFailed(format!("Failed to create branch: {e}")))?;

            local_branch
                .set_upstream(Some(&remote_name))
                .map_err(|e| WmError::CommandFailed(format!("Failed to set upstream: {e}")))?;

            let reference = local_branch.into_reference();
            let mut opts = WorktreeAddOptions::new();
            opts.reference(Some(&reference));

            repo.worktree(worktree_name, worktree_dir, Some(&opts))
                .map_err(|e| WmError::CommandFailed(format!("Failed to add worktree: {e}")))?;
        }
        WorktreeAddMode::NewBranch { branch, base } => {
            // Create new branch from base
            let base_commit = repo
                .revparse_single(base)
                .map_err(|e| WmError::CommandFailed(format!("Failed to resolve base: {e}")))?
                .peel_to_commit()
                .map_err(|e| WmError::CommandFailed(format!("Failed to get commit: {e}")))?;

            let new_branch = repo
                .branch(branch, &base_commit, false)
                .map_err(|e| WmError::CommandFailed(format!("Failed to create branch: {e}")))?;

            let reference = new_branch.into_reference();
            let mut opts = WorktreeAddOptions::new();
            opts.reference(Some(&reference));

            repo.worktree(worktree_name, worktree_dir, Some(&opts))
                .map_err(|e| WmError::CommandFailed(format!("Failed to add worktree: {e}")))?;
        }
        WorktreeAddMode::ForceNewBranch { branch, base } => {
            // Force create/reset branch from base
            let base_commit = repo
                .revparse_single(base)
                .map_err(|e| WmError::CommandFailed(format!("Failed to resolve base: {e}")))?
                .peel_to_commit()
                .map_err(|e| WmError::CommandFailed(format!("Failed to get commit: {e}")))?;

            // Delete existing branch if it exists
            if let Ok(mut existing) = repo.find_branch(branch, BranchType::Local) {
                existing.delete().ok();
            }

            let new_branch = repo
                .branch(branch, &base_commit, true)
                .map_err(|e| WmError::CommandFailed(format!("Failed to create branch: {e}")))?;

            let reference = new_branch.into_reference();
            let mut opts = WorktreeAddOptions::new();
            opts.reference(Some(&reference));

            repo.worktree(worktree_name, worktree_dir, Some(&opts))
                .map_err(|e| WmError::CommandFailed(format!("Failed to add worktree: {e}")))?;
        }
    }

    Ok(())
}

/// Add a worktree for an existing branch (local or remote)
fn add_worktree_for_branch(repo: &Repository, worktree_dir: &Path, branch: &str) -> Result<()> {
    if local_branch_exists(branch) {
        git_worktree_add(repo, worktree_dir, WorktreeAddMode::LocalBranch { branch })
    } else if remote_branch_exists(branch) {
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
}

/// Resolve branch name: use provided name or generate from prompt.
fn resolve_branch_name(args: &NewArgs) -> Result<String> {
    match (&args.name, &args.prompt) {
        (Some(name), _) => Ok(name.clone()),
        (None, Some(prompt)) => {
            let backend = detect_backend();
            let generated = generate_branch_name(prompt, backend.as_ref())?;
            Ok(generated)
        }
        (None, None) => Err(WmError::MissingBranchName),
    }
}

pub fn run(args: &NewArgs) -> std::result::Result<(), Box<dyn std::error::Error>> {
    run_inner(args)?;
    Ok(())
}

fn run_inner(args: &NewArgs) -> Result<()> {
    let name = resolve_branch_name(args)?;
    let repo_root = get_repo_root()?;

    let repo = Repository::open_from_env().map_err(|_| WmError::NotInGitRepo)?;

    // Determine worktree directory name from branch name
    let worktree_name = branch_to_worktree_name(&name);
    let worktrees_dir = format!("{repo_root}/.worktrees");
    let worktree_dir = Path::new(&worktrees_dir).join(&worktree_name);

    // Ensure .worktrees directory exists
    std::fs::create_dir_all(&worktrees_dir).map_err(|e| {
        WmError::CommandFailed(format!("Failed to create .worktrees directory: {e}"))
    })?;

    // Fetch with prune
    fetch_with_prune(&repo).map_err(|e| WmError::CommandFailed(e.to_string()))?;

    // Remove BRANCH_PREFIX to avoid double prefix
    let name_no_prefix = name.strip_prefix(BRANCH_PREFIX).unwrap_or(&name);

    // Determine action based on branch existence and flags
    if args.force {
        // Force create new branch with BRANCH_PREFIX
        let main_branch = get_main_branch()?;
        let base_branch = args
            .from
            .clone()
            .unwrap_or_else(|| format!("origin/{main_branch}"));
        let branch = format!("{BRANCH_PREFIX}{name_no_prefix}");

        git_worktree_add(
            &repo,
            &worktree_dir,
            WorktreeAddMode::ForceNewBranch {
                branch: &branch,
                base: &base_branch,
            },
        )?;
    } else if branch_exists(&name) {
        // Branch exists with the exact name provided
        add_worktree_for_branch(&repo, &worktree_dir, &name)?;
    } else {
        let branch_with_prefix = format!("{BRANCH_PREFIX}{name_no_prefix}");
        if branch_exists(&branch_with_prefix) {
            // Branch exists with BRANCH_PREFIX
            add_worktree_for_branch(&repo, &worktree_dir, &branch_with_prefix)?;
        } else {
            // Branch doesn't exist, create new one with BRANCH_PREFIX
            let main_branch = get_main_branch()?;
            let base_branch = args
                .from
                .clone()
                .unwrap_or_else(|| format!("origin/{main_branch}"));
            let branch = format!("{BRANCH_PREFIX}{name_no_prefix}");

            git_worktree_add(
                &repo,
                &worktree_dir,
                WorktreeAddMode::NewBranch {
                    branch: &branch,
                    base: &base_branch,
                },
            )?;
        }
    }

    // Setup tmux window with nvim + claude
    setup_tmux_window(
        &repo_root,
        worktree_dir.to_str().unwrap_or(&worktree_name),
        &worktree_name,
        args.prompt.as_deref(),
    )?;

    Ok(())
}

/// Build the claude command, optionally with an initial prompt via temp file
fn build_claude_command(prompt: Option<&str>) -> Result<String> {
    let Some(prompt) = prompt else {
        return Ok("claude".to_string());
    };

    // Write prompt to a temp file that persists until shell reads it
    let prompt_file = tempfile::Builder::new()
        .prefix("claude-prompt-")
        .suffix(".txt")
        .tempfile()
        .map_err(|e| WmError::CommandFailed(format!("Failed to create temp file: {e}")))?;

    std::fs::write(prompt_file.path(), prompt)
        .map_err(|e| WmError::CommandFailed(format!("Failed to write prompt: {e}")))?;

    let prompt_path = prompt_file
        .into_temp_path()
        .keep()
        .map_err(|e| WmError::CommandFailed(format!("Failed to persist temp file: {e}")))?;

    let path_str = prompt_path.display().to_string();
    let escaped_path = shlex::try_quote(&path_str)
        .map_err(|_| WmError::CommandFailed("Failed to escape path".into()))?;

    // Command reads prompt, passes to claude, then deletes temp file
    Ok(format!(
        "claude \"$(cat {escaped_path})\" ; rm {escaped_path}"
    ))
}

/// Setup a tmux window with split panes for nvim and claude
fn setup_tmux_window(
    repo_root: &str,
    worktree_dir: &str,
    worktree_name: &str,
    prompt: Option<&str>,
) -> Result<()> {
    let claude_cmd = build_claude_command(prompt)?;
    let target_session = tmux::get_session_name(repo_root);

    tmux::ensure_session(&target_session, repo_root)
        .map_err(|e| WmError::CommandFailed(e.to_string()))?;

    tmux::create_split_window(
        &target_session,
        worktree_dir,
        worktree_name,
        "nvim",
        &claude_cmd,
    )
    .map_err(|e| WmError::CommandFailed(e.to_string()))?;

    tmux::switch_to_session(&target_session).map_err(|e| WmError::CommandFailed(e.to_string()))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::TestRepo;
    use git2::Signature;

    #[test]
    fn build_claude_command_without_prompt_returns_claude() {
        let cmd = build_claude_command(None).unwrap();
        assert_eq!(cmd, "claude");
    }

    #[test]
    fn build_claude_command_with_prompt_creates_temp_file() {
        let cmd = build_claude_command(Some("test prompt")).unwrap();

        // Should contain cat and rm commands
        assert!(cmd.contains("cat"));
        assert!(cmd.contains("rm"));
        assert!(cmd.contains("claude-prompt-"));
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

        // Worktree should exist
        assert!(worktree_dir.exists());
        // Branch should be created
        assert!(repo.find_branch("test-branch", BranchType::Local).is_ok());
    }

    #[test]
    fn git_worktree_add_with_local_branch() {
        let test_repo = TestRepo::new();
        let repo = test_repo.open();

        // Create a branch first
        let head = repo.head().unwrap().peel_to_commit().unwrap();
        repo.branch("existing-branch", &head, false).unwrap();

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

        // Create initial branch
        let head = repo.head().unwrap().peel_to_commit().unwrap();
        repo.branch("force-test", &head, false).unwrap();

        // Create a new commit
        let sig = Signature::now("Test", "test@test.com").unwrap();
        let tree_id = repo.index().unwrap().write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "Second commit", &tree, &[&head])
            .unwrap();

        let worktrees_dir = test_repo.path().join(".worktrees");
        std::fs::create_dir_all(&worktrees_dir).unwrap();

        // Force create should delete old branch and create from new HEAD
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

    #[test]
    fn resolve_branch_name_with_explicit_name() {
        let args = NewArgs {
            name: Some("my-branch".to_string()),
            from: None,
            force: false,
            prompt: None,
        };
        let result = resolve_branch_name(&args).unwrap();
        assert_eq!(result, "my-branch");
    }

    #[test]
    fn resolve_branch_name_with_name_and_prompt_uses_name() {
        let args = NewArgs {
            name: Some("my-branch".to_string()),
            from: None,
            force: false,
            prompt: Some("some task".to_string()),
        };
        let result = resolve_branch_name(&args).unwrap();
        assert_eq!(result, "my-branch");
    }

    #[test]
    fn resolve_branch_name_without_name_and_prompt_returns_error() {
        let args = NewArgs {
            name: None,
            from: None,
            force: false,
            prompt: None,
        };
        let result = resolve_branch_name(&args);
        assert!(matches!(result, Err(WmError::MissingBranchName)));
    }
}
