use clap::Args;
use indoc::formatdoc;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

use super::common::{DraftFile, PrDraftError, RepoInfo};

#[derive(Args, Clone, PartialEq, Eq)]
pub struct ReviewArgs {
    /// Path to the draft file (auto-detected if not specified)
    pub filepath: Option<PathBuf>,
}

/// Internal command to complete the review process after Neovim exits.
/// This is called by WezTerm, not directly by users.
#[derive(Args, Clone, PartialEq, Eq)]
pub struct ReviewCompleteArgs {
    /// Path to the draft file
    pub filepath: PathBuf,

    /// tmux session to restore after review
    #[arg(long)]
    pub tmux_target: Option<String>,
}

pub fn run(args: &ReviewArgs) -> Result<(), Box<dyn std::error::Error>> {
    let (draft_path, owner, repo, branch) = match &args.filepath {
        Some(path) => {
            let (owner, repo, branch) = DraftFile::parse_path(path).ok_or_else(|| {
                let display = path.display();
                PrDraftError::CommandFailed(format!("Invalid draft path: {display}"))
            })?;
            (path.clone(), owner, repo, branch)
        }
        None => {
            let repo_info = RepoInfo::from_git_only()?;
            let path = DraftFile::path_for(&repo_info);
            (path, repo_info.owner, repo_info.repo, repo_info.branch)
        }
    };

    if !draft_path.exists() {
        return Err(Box::new(PrDraftError::FileNotFound(draft_path)));
    }

    // Check for existing lock
    let lock_path = DraftFile::lock_path(&draft_path);
    if lock_path.exists() {
        eprintln!("Skipped: Editor is already open for this file.");
        return Ok(());
    }

    // Create lock file
    fs::write(&lock_path, "")?;

    let window_title = format!("PR: {owner}/{repo} @ {branch}");

    // Get tmux session info for later restoration
    let tmux_target = get_tmux_target();

    // Get the path to the current executable
    let exe_path = std::env::current_exe()?;

    // Build arguments for review-complete command (avoid shell injection)
    let mut review_args = vec![
        "ai".to_string(),
        "pr-draft".to_string(),
        "review-complete".to_string(),
        draft_path.display().to_string(),
    ];
    if let Some(ref target) = tmux_target {
        review_args.push("--tmux-target".to_string());
        review_args.push(target.clone());
    }

    // RAII guard ensures lock cleanup on error paths
    let mut lock_guard = LockGuard::new(lock_path);

    // Launch WezTerm with the review-complete command
    let status = launch_wezterm(&window_title, &exe_path, &review_args)
        .map_err(|e| PrDraftError::CommandFailed(format!("Failed to launch WezTerm: {e}")))?;

    if !status.success() {
        // Guard will be dropped here, removing lock file
        return Err(Box::new(PrDraftError::CommandFailed(format!(
            "WezTerm exited with status: {status}"
        ))));
    }

    // WezTerm launched successfully - disarm guard so lock file remains
    // (review-complete will handle cleanup when it finishes)
    lock_guard.disarm();

    Ok(())
}

pub fn run_complete(args: &ReviewCompleteArgs) -> Result<(), Box<dyn std::error::Error>> {
    let draft_path = &args.filepath;

    // Ensure cleanup happens even on panic
    let _cleanup_guard = CleanupGuard {
        lock_path: DraftFile::lock_path(draft_path),
        tmux_target: args.tmux_target.clone(),
    };

    // Launch Neovim
    let status = Command::new("nvim")
        .arg(draft_path)
        .status()
        .map_err(|e| PrDraftError::CommandFailed(format!("Failed to launch nvim: {e}")))?;

    if !status.success() {
        eprintln!("Neovim exited with non-zero status");
        return Ok(());
    }

    // After Neovim exits, check if submit was approved
    let draft = DraftFile::from_path(draft_path.clone())?;

    if draft.frontmatter.steps.submit {
        // Save approval hash
        draft.save_approval()?;
        println!(
            "{}",
            formatdoc! {"
                PR approved. Run the following command to create the PR:

                    a ai pr-draft submit
            "}
        );
    } else {
        // Remove approval if exists
        draft.remove_approval()?;
        println!("PR not approved. Set 'steps.submit: true' and save to approve.");
    }

    Ok(())
}

/// RAII guard for lock file cleanup in run()
struct LockGuard {
    lock_path: PathBuf,
    /// If true, skip cleanup on drop (used when WezTerm will handle it)
    disarmed: bool,
}

impl LockGuard {
    fn new(lock_path: PathBuf) -> Self {
        Self {
            lock_path,
            disarmed: false,
        }
    }

    /// Prevent this guard from removing the lock file on drop
    fn disarm(&mut self) {
        self.disarmed = true;
    }
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        if !self.disarmed {
            let _ = fs::remove_file(&self.lock_path);
        }
    }
}

/// RAII guard for cleanup after review-complete (lock file + tmux restore)
struct CleanupGuard {
    lock_path: PathBuf,
    tmux_target: Option<String>,
}

impl Drop for CleanupGuard {
    fn drop(&mut self) {
        // Remove lock file
        let _ = fs::remove_file(&self.lock_path);

        // Restore tmux session
        if let Some(ref target) = self.tmux_target {
            let _ = Command::new("tmux")
                .args(["switch-client", "-t", target])
                .status();
        }
    }
}

fn get_tmux_target() -> Option<String> {
    if std::env::var("TMUX").is_err() {
        return None;
    }

    // Get session:window.pane in a single tmux call for consistency and performance
    let output = Command::new("tmux")
        .args([
            "display-message",
            "-p",
            "#{session_name}:#{window_index}.#{pane_index}",
        ])
        .output()
        .ok()?;

    if output.status.success() {
        Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        None
    }
}

#[cfg(target_os = "macos")]
fn launch_wezterm(
    window_title: &str,
    exe_path: &std::path::Path,
    args: &[String],
) -> std::io::Result<std::process::ExitStatus> {
    let mut cmd = Command::new("open");
    cmd.args([
        "-n",
        "-a",
        "WezTerm",
        "--args",
        "--config",
        "window_decorations=\"TITLE | RESIZE\"",
        "--config",
        "initial_cols=120",
        "--config",
        "initial_rows=40",
        "start",
        "--class",
        window_title,
        "--",
    ]);
    cmd.arg(exe_path);
    cmd.args(args);
    cmd.status()
}

#[cfg(not(target_os = "macos"))]
fn launch_wezterm(
    window_title: &str,
    exe_path: &std::path::Path,
    args: &[String],
) -> std::io::Result<std::process::ExitStatus> {
    let mut cmd = Command::new("wezterm");
    cmd.args([
        "--config",
        "window_decorations=\"TITLE | RESIZE\"",
        "--config",
        "initial_cols=120",
        "--config",
        "initial_rows=40",
        "start",
        "--class",
        window_title,
        "--",
    ]);
    cmd.arg(exe_path);
    cmd.args(args);
    cmd.status()
}
