//! Simple editor workflow without document schema or approval flow.
//!
//! This module provides a simplified version of the review workflow that:
//! - Opens a file in WezTerm + Neovim
//! - Handles lock file to prevent multiple editors
//! - Restores tmux session after editor closes
//!
//! Unlike the full review workflow, this does not:
//! - Parse document frontmatter
//! - Handle approval status
//! - Call any completion callbacks

use std::ffi::OsString;
use std::path::Path;

use super::editor::{LaunchOptions, launch_wezterm, run_neovim};
use super::error::{HumanInTheLoopError, Result};
use super::lock::{CleanupGuard, LockGuard};
use super::tmux::get_tmux_target;

/// Arguments needed to complete a simple edit session.
#[derive(Debug, Clone)]
pub struct SimpleEditCompleteArgs {
    pub tmux_target: Option<String>,
    pub window_title: Option<String>,
}

/// Start a simple edit session by launching WezTerm with Neovim.
///
/// This function:
/// 1. Checks for an existing lock (another editor open)
/// 2. Creates a lock file
/// 3. Launches WezTerm configured to run the edit-complete command after Neovim exits
///
/// # Arguments
/// * `document_path` - Path to the file to edit
/// * `window_title` - Title for the WezTerm window
/// * `complete_command` - The command to run after Neovim exits (e.g., "ai draft edit-complete")
pub fn start_simple_edit(
    document_path: &Path,
    window_title: &str,
    complete_command: &[&str],
) -> Result<()> {
    if !document_path.exists() {
        return Err(HumanInTheLoopError::FileNotFound(
            document_path.to_path_buf(),
        ));
    }

    // Check for existing lock
    if LockGuard::is_locked(document_path) {
        eprintln!("Skipped: Editor is already open for this file.");
        return Ok(());
    }

    // Create lock file with RAII guard
    let mut lock_guard = LockGuard::acquire(document_path)?;

    // Get tmux session info for later restoration
    let tmux_target = get_tmux_target();

    // Get the path to the current executable
    let exe_path = std::env::current_exe()?;

    // Build the edit-complete command arguments
    let mut args: Vec<OsString> = complete_command.iter().map(|&s| s.into()).collect();
    args.push(document_path.as_os_str().to_os_string());

    if let Some(ref target) = tmux_target {
        args.push("--tmux-target".into());
        args.push(target.into());
    }

    args.push("--title".into());
    args.push(window_title.into());

    // Launch WezTerm
    let options = LaunchOptions {
        window_title: window_title.to_string(),
        ..Default::default()
    };

    let status = launch_wezterm(&options, &exe_path, &args)?;

    if !status.success() {
        return Err(HumanInTheLoopError::CommandFailed(format!(
            "WezTerm exited with status: {status}"
        )));
    }

    // WezTerm launched successfully - disarm guard so lock file remains
    // (edit-complete will handle cleanup when it finishes)
    lock_guard.disarm();

    Ok(())
}

/// Complete a simple edit session after the user closes Neovim.
///
/// This function:
/// 1. Sets up cleanup guards for lock file and tmux restoration
/// 2. Launches Neovim for the user to edit the file
/// 3. After Neovim exits, cleans up and restores tmux session
///
/// This is typically called by the edit-complete subcommand that WezTerm runs.
pub fn complete_simple_edit(document_path: &Path, args: &SimpleEditCompleteArgs) -> Result<()> {
    // Ensure cleanup happens even on panic
    let _cleanup_guard = CleanupGuard::new(document_path, args.tmux_target.clone());

    // Launch Neovim
    let status = run_neovim(document_path, args.window_title.as_deref())?;

    if !status.success() {
        eprintln!("Neovim exited with non-zero status");
    }

    Ok(())
}
