use anyhow::{Context, bail};
use clap::Args;
use git2::{BranchType, Repository, WorktreeAddOptions};
use std::path::{Path, PathBuf};
use std::process::Command;

use super::error::{Result, WmError};
use super::git::{
    branch_exists, branch_to_worktree_name, get_main_branch, get_repo_root, local_branch_exists,
    remote_branch_exists,
};
use crate::commands::name_branch::{detect_backend, generate_branch_name};
use crate::infra::git::fetch_with_prune;
use crate::infra::tmux;
use crate::shared::cache;
use crate::shared::config::{Config, load_config};

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
    let status = Command::new(&editor)
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

/// Run `git worktree add` with the specified mode using git2
fn git_worktree_add(repo: &Repository, worktree_dir: &Path, mode: WorktreeAddMode) -> Result<()> {
    let worktree_name = worktree_dir
        .file_name()
        .and_then(|n| n.to_str())
        .context("Invalid worktree path")?;

    match mode {
        WorktreeAddMode::LocalBranch { branch } => {
            // Checkout existing local branch
            let local_branch = repo
                .find_branch(branch, BranchType::Local)
                .context("Failed to find branch")?;
            let reference = local_branch.into_reference();

            let mut opts = WorktreeAddOptions::new();
            opts.reference(Some(&reference));

            repo.worktree(worktree_name, worktree_dir, Some(&opts))
                .context("Failed to add worktree")?;
        }
        WorktreeAddMode::TrackRemote { branch } => {
            // Create a tracking branch from remote
            let remote_name = format!("origin/{branch}");
            let remote_branch = repo
                .find_branch(&remote_name, BranchType::Remote)
                .context("Failed to find remote branch")?;

            let commit = remote_branch
                .get()
                .peel_to_commit()
                .context("Failed to get commit from remote branch")?;

            // Create local branch tracking remote
            let mut local_branch = repo
                .branch(branch, &commit, false)
                .context("Failed to create branch")?;

            local_branch
                .set_upstream(Some(&remote_name))
                .context("Failed to set upstream")?;

            let reference = local_branch.into_reference();
            let mut opts = WorktreeAddOptions::new();
            opts.reference(Some(&reference));

            repo.worktree(worktree_name, worktree_dir, Some(&opts))
                .context("Failed to add worktree")?;
        }
        WorktreeAddMode::NewBranch { branch, base } => {
            // Create new branch from base
            let base_commit = repo
                .revparse_single(base)
                .context("Failed to resolve base")?
                .peel_to_commit()
                .context("Failed to get commit")?;

            let new_branch = repo
                .branch(branch, &base_commit, false)
                .context("Failed to create branch")?;

            let reference = new_branch.into_reference();
            let mut opts = WorktreeAddOptions::new();
            opts.reference(Some(&reference));

            repo.worktree(worktree_name, worktree_dir, Some(&opts))
                .context("Failed to add worktree")?;
        }
        WorktreeAddMode::ForceNewBranch { branch, base } => {
            // Force create/reset branch from base
            let base_commit = repo
                .revparse_single(base)
                .context("Failed to resolve base")?
                .peel_to_commit()
                .context("Failed to get commit")?;

            // Delete existing branch if it exists
            if let Ok(mut existing) = repo.find_branch(branch, BranchType::Local) {
                existing.delete().ok();
            }

            let new_branch = repo
                .branch(branch, &base_commit, true)
                .context("Failed to create branch")?;

            let reference = new_branch.into_reference();
            let mut opts = WorktreeAddOptions::new();
            opts.reference(Some(&reference));

            repo.worktree(worktree_name, worktree_dir, Some(&opts))
                .context("Failed to add worktree")?;
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

    /// Mark this invocation as coming from another Claude Code session.
    /// Wraps the prompt with delegation context (branch, base, directories).
    #[arg(long)]
    pub agent: bool,
}

/// Context information injected into the prompt when --agent is used
struct DelegationContext<'a> {
    branch: &'a str,
    base: &'a str,
    delegator_cwd: &'a str,
    worktree_cwd: &'a str,
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

    let repo_root = get_repo_root()?;

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
    let repo = Repository::open_from_env().map_err(|_| WmError::NotInGitRepo)?;
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

    if args.force {
        // Force create new branch with prefix
        let main_branch = get_main_branch()?;
        let base_branch = args
            .from
            .clone()
            .unwrap_or_else(|| format!("origin/{main_branch}"));
        let branch = format!("{branch_prefix}{name_no_prefix}");

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
    } else if branch_exists(name) {
        // Branch exists with the exact name provided
        add_worktree_for_branch(&repo, &worktree_dir, name)?;

        let main_branch = get_main_branch()?;
        actual_branch = name.to_string();
        actual_base = format!("origin/{main_branch}");
    } else {
        let branch_with_prefix = format!("{branch_prefix}{name_no_prefix}");
        if branch_exists(&branch_with_prefix) {
            // Branch exists with prefix
            add_worktree_for_branch(&repo, &worktree_dir, &branch_with_prefix)?;

            let main_branch = get_main_branch()?;
            actual_branch = branch_with_prefix;
            actual_base = format!("origin/{main_branch}");
        } else {
            // Branch doesn't exist, create new one with prefix
            let main_branch = get_main_branch()?;
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
        }
    }

    // Wrap prompt with delegation context when --agent is used
    let final_prompt = match (args.agent, prompt) {
        (true, Some(p)) => {
            let delegator_cwd = std::env::current_dir()
                .context("Failed to get current directory")?
                .to_string_lossy()
                .to_string();
            let worktree_cwd = worktree_dir
                .to_str()
                .context("Invalid worktree path")?
                .to_string();

            Some(build_delegated_prompt(
                p,
                &DelegationContext {
                    branch: &actual_branch,
                    base: &actual_base,
                    delegator_cwd: &delegator_cwd,
                    worktree_cwd: &worktree_cwd,
                },
            ))
        }
        (_, p) => p.map(String::from),
    };

    // Setup tmux window using config layout
    setup_tmux_window(
        repo_root,
        worktree_dir.to_str().unwrap_or(&worktree_name),
        &worktree_name,
        final_prompt.as_deref(),
        config,
    )?;

    println!(
        "Created worktree '{}' and opened tmux window",
        worktree_name
    );

    Ok(())
}

/// Setup a tmux window using the configured layout.
fn setup_tmux_window(
    repo_root: &str,
    worktree_dir: &str,
    worktree_name: &str,
    prompt: Option<&str>,
    config: &Config,
) -> Result<()> {
    let target_session = tmux::get_session_name(repo_root, &config.wm.worktrees_dir);

    tmux::ensure_session(&target_session, repo_root).context("Failed to ensure tmux session")?;

    tmux::layout::build_layout(
        &target_session,
        worktree_dir,
        worktree_name,
        &config.wm.layout,
        prompt,
    )
    .context("Failed to create tmux layout")?;

    tmux::switch_to_session(&target_session).context("Failed to switch to tmux session")?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shared::testing::TestRepo;
    use git2::Signature;
    use indoc::indoc;
    use tempfile::TempDir;

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

    use crate::commands::name_branch::{Backend, Result as NameBranchResult};
    use rstest::rstest;

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
}
