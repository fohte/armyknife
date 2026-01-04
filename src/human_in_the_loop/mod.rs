//! Human-in-the-loop review module
//!
//! Provides a reusable framework for interactive editing workflows where:
//! 1. A document is prepared for user review
//! 2. An editor (WezTerm + Neovim) is launched
//! 3. User edits the document and saves
//! 4. The result is parsed and processed

mod approval;
mod document;
mod editor;
mod error;
mod lock;
mod tmux;

pub use document::{Document, DocumentSchema};
pub use editor::{LaunchOptions, launch_wezterm, run_neovim};
pub use error::{HumanInTheLoopError, Result};
pub use lock::{CleanupGuard, LockGuard};
pub use tmux::get_tmux_target;

use std::ffi::OsString;
use std::path::Path;

/// Trait for types that handle the review completion callback.
///
/// Each use case (PR draft, issue comment, PR review reply) implements this trait
/// to define how to handle the document after the user finishes editing.
pub trait ReviewHandler<S: DocumentSchema> {
    /// Build the command-line arguments for the review-complete subcommand.
    ///
    /// This is called when launching WezTerm to specify what command should run
    /// after the user closes Neovim.
    fn build_complete_args(&self, document_path: &Path, tmux_target: Option<&str>)
    -> Vec<OsString>;

    /// Called after the user finishes editing and closes Neovim.
    ///
    /// This is where you check the document's approval status and take appropriate action.
    fn on_review_complete(&self, document: &Document<S>) -> Result<()>;
}

/// Start a review session by launching WezTerm with Neovim.
///
/// This function:
/// 1. Checks for an existing lock (another editor open)
/// 2. Creates a lock file
/// 3. Launches WezTerm configured to run the review-complete command after Neovim exits
///
/// The handler's `build_complete_args` is used to construct the command that WezTerm will run.
pub fn start_review<S, H>(document_path: &Path, window_title: &str, handler: &H) -> Result<()>
where
    S: DocumentSchema,
    H: ReviewHandler<S>,
{
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

    // Build the review-complete command arguments
    let review_args = handler.build_complete_args(document_path, tmux_target.as_deref());

    // Launch WezTerm
    let options = LaunchOptions {
        window_title: window_title.to_string(),
        ..Default::default()
    };

    let status = launch_wezterm(&options, &exe_path, &review_args)?;

    if !status.success() {
        return Err(HumanInTheLoopError::CommandFailed(format!(
            "WezTerm exited with status: {status}"
        )));
    }

    // WezTerm launched successfully - disarm guard so lock file remains
    // (review-complete will handle cleanup when it finishes)
    lock_guard.disarm();

    Ok(())
}

/// Complete a review session after the user closes Neovim.
///
/// This function:
/// 1. Sets up cleanup guards for lock file and tmux restoration
/// 2. Launches Neovim for the user to edit the document
/// 3. After Neovim exits, parses the document and calls the handler's callback
///
/// This is typically called by the review-complete subcommand that WezTerm runs.
pub fn complete_review<S, H>(
    document_path: &Path,
    tmux_target: Option<&str>,
    handler: &H,
) -> Result<()>
where
    S: DocumentSchema,
    H: ReviewHandler<S>,
{
    // Ensure cleanup happens even on panic
    let _cleanup_guard = CleanupGuard::new(document_path, tmux_target.map(String::from));

    // Launch Neovim
    let status = run_neovim(document_path)?;

    if !status.success() {
        eprintln!("Neovim exited with non-zero status");
        return Ok(());
    }

    // After Neovim exits, parse the document and call the handler
    let document = Document::<S>::from_path(document_path.to_path_buf())?;
    handler.on_review_complete(&document)
}
