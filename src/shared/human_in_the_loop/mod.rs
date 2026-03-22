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
pub use editor::{LaunchOptions, launch_terminal, run_editor};
pub use error::{HumanInTheLoopError, Result};
pub use lock::{CleanupGuard, LockGuard};
pub use tmux::get_tmux_target;

use std::ffi::OsString;
use std::path::Path;
use std::time::Duration;

use crate::shared::config::EditorConfig;

/// Polling interval for waiting on lock file removal.
const LOCK_POLL_INTERVAL: Duration = Duration::from_millis(300);

/// Maximum time to wait for the lock file to be removed before giving up.
/// Set generously since users may leave the editor open for extended periods.
const LOCK_POLL_TIMEOUT: Duration = Duration::from_secs(24 * 60 * 60);

/// Trait for types that handle the review completion callback.
///
/// Each use case (PR draft, issue comment, PR review reply, or simple file editing)
/// implements this trait to define how to handle the document after the user finishes editing.
pub trait ReviewHandler<S: DocumentSchema> {
    /// Build the command-line arguments for the review-complete subcommand.
    ///
    /// This is called when launching WezTerm to specify what command should run
    /// after the user closes Neovim.
    fn build_complete_args(
        &self,
        document_path: &Path,
        tmux_target: Option<&str>,
        window_title: &str,
    ) -> Vec<OsString>;

    /// Called after the user finishes editing and closes Neovim.
    ///
    /// This is where you check the document's approval status and take appropriate action.
    /// Default implementation does nothing, suitable for simple editing workflows.
    fn on_review_complete(&self, _document: &Document<S>) -> Result<()> {
        Ok(())
    }
}

/// Start a review session by launching a terminal with the configured editor.
///
/// This function:
/// 1. Checks for an existing lock (another editor open)
/// 2. Creates a lock file
/// 3. Launches the configured terminal to run the review-complete command after the editor exits
/// 4. Blocks until the review-complete process finishes (detected by lock file removal)
///
/// The handler's `build_complete_args` is used to construct the command that the terminal will run.
/// Returns the final document state after the user finishes editing, or `None` if the editor
/// was already open (lock existed).
pub fn start_review<S, H>(
    document_path: &Path,
    window_title: &str,
    handler: &H,
    editor_config: &EditorConfig,
) -> Result<Option<Document<S>>>
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
        return Ok(None);
    }

    // Create lock file with RAII guard
    let mut lock_guard = LockGuard::acquire(document_path)?;

    // Get tmux session info for later restoration
    let tmux_target = get_tmux_target();

    // Get the path to the current executable
    let exe_path = std::env::current_exe()?;

    // Build the review-complete command arguments
    let review_args =
        handler.build_complete_args(document_path, tmux_target.as_deref(), window_title);

    // Launch terminal emulator
    let options = LaunchOptions {
        window_title: window_title.to_string(),
        ..Default::default()
    };

    let status = launch_terminal(&editor_config.terminal, &options, &exe_path, &review_args)?;

    if !status.success() {
        return Err(HumanInTheLoopError::CommandFailed(format!(
            "Terminal exited with status: {status}"
        )));
    }

    // Terminal launched successfully - disarm guard so lock file remains
    // (review-complete will handle cleanup when it finishes)
    lock_guard.disarm();

    // Wait for the review-complete process to finish by polling for lock file removal.
    // The CleanupGuard in complete_review removes the lock file when the editor exits.
    let start = std::time::Instant::now();
    while LockGuard::is_locked(document_path) {
        if start.elapsed() > LOCK_POLL_TIMEOUT {
            return Err(HumanInTheLoopError::CommandFailed(format!(
                "Timed out waiting for editor to close. Lock file may be stale: {}",
                LockGuard::lock_path(document_path).display()
            )));
        }
        std::thread::sleep(LOCK_POLL_INTERVAL);
    }

    // Review complete - read the final document state
    let document = Document::<S>::from_path(document_path.to_path_buf())?;
    Ok(Some(document))
}

/// Complete a review session after the user closes the editor.
///
/// This function:
/// 1. Sets up cleanup guards for lock file and tmux restoration
/// 2. Launches the configured editor for the user to edit the document
/// 3. After the editor exits, parses the document and calls the handler's callback
///
/// If `window_title` is provided and the editor is nvim, it will be displayed in the title bar.
///
/// This is typically called by the review-complete subcommand that the terminal runs.
pub fn complete_review<S, H>(
    document_path: &Path,
    tmux_target: Option<&str>,
    window_title: Option<&str>,
    handler: &H,
    editor_config: &EditorConfig,
) -> Result<()>
where
    S: DocumentSchema,
    H: ReviewHandler<S>,
{
    // Ensure cleanup happens even on panic
    let _cleanup_guard = CleanupGuard::new(document_path, tmux_target.map(String::from));

    // Launch editor
    let status = run_editor(&editor_config.editor_command, document_path, window_title)?;

    if !status.success() {
        eprintln!("Editor exited with non-zero status");
        return Ok(());
    }

    // After editor exits, parse the document and call the handler
    let document = Document::<S>::from_path(document_path.to_path_buf())?;
    handler.on_review_complete(&document)
}
