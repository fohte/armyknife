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
    let repo_info = RepoInfo::from_current_dir()?;

    let draft_path = match &args.filepath {
        Some(path) => path.clone(),
        None => DraftFile::path_for(&repo_info),
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

    let window_title = format!(
        "PR: {}/{} @ {}",
        repo_info.owner, repo_info.repo, repo_info.branch
    );

    // Get tmux session info for later restoration
    let tmux_target = get_tmux_target();

    // Get the path to the current executable
    let exe_path = std::env::current_exe()?;

    // Build the command for review-complete
    let mut review_cmd = format!(
        "{} ai pr-draft review-complete {}",
        exe_path.display(),
        draft_path.display()
    );
    if let Some(ref target) = tmux_target {
        review_cmd.push_str(&format!(" --tmux-target {}", target));
    }

    // Launch WezTerm with the review-complete command
    let status = Command::new("open")
        .args([
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
            &window_title,
            "--",
            "bash",
            "-c",
            &review_cmd,
        ])
        .status();

    if let Err(e) = status {
        // Cleanup lock on error
        let _ = fs::remove_file(&lock_path);
        return Err(Box::new(PrDraftError::CommandFailed(format!(
            "Failed to launch WezTerm: {}",
            e
        ))));
    }

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
        .map_err(|e| PrDraftError::CommandFailed(format!("Failed to launch nvim: {}", e)))?;

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

    let session = run_tmux_command(&["display-message", "-p", "#{session_name}"])?;
    let window = run_tmux_command(&["display-message", "-p", "#{window_index}"])?;
    let pane = run_tmux_command(&["display-message", "-p", "#{pane_index}"])?;

    Some(format!("{}:{}.{}", session, window, pane))
}

fn run_tmux_command(args: &[&str]) -> Option<String> {
    Command::new("tmux").args(args).output().ok().and_then(|o| {
        if o.status.success() {
            Some(String::from_utf8_lossy(&o.stdout).trim().to_string())
        } else {
            None
        }
    })
}
