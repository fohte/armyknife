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
use std::path::{Path, PathBuf};

use crate::shared::config::EditorConfig;

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
/// 3. Creates a FIFO for completion signaling
/// 4. Launches the configured terminal to run the review-complete command after the editor exits
/// 5. Blocks until the review-complete process signals completion via the FIFO
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

    // Create a FIFO for the review-complete process to signal completion.
    // The FifoCleanupGuard ensures the FIFO is removed on early return.
    let fifo_path = create_done_fifo(document_path)?;
    let mut fifo_cleanup = FifoCleanupGuard::new(&fifo_path);

    // Get tmux session info for later restoration
    let tmux_target = get_tmux_target();

    // Get the path to the current executable
    let exe_path = std::env::current_exe()?;

    // Build the review-complete command arguments and append --done-fifo
    let mut review_args =
        handler.build_complete_args(document_path, tmux_target.as_deref(), window_title);
    review_args.push("--done-fifo".into());
    review_args.push(fifo_path.as_os_str().to_os_string());

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

    // Wait for the review-complete process to finish via FIFO.
    // This blocks with no CPU usage until the other process writes to the FIFO.
    wait_for_fifo(&fifo_path)?;

    // Clean up the FIFO (disarm guard since we're cleaning up explicitly)
    fifo_cleanup.disarm();
    let _ = std::fs::remove_file(&fifo_path);

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
/// 4. If `done_fifo` is provided, writes to it to signal the waiting `start_review` process
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
    done_fifo: Option<&Path>,
) -> Result<()>
where
    S: DocumentSchema,
    H: ReviewHandler<S>,
{
    // Ensure cleanup happens even on panic
    let _cleanup_guard = CleanupGuard::new(document_path, tmux_target.map(String::from));
    let _fifo_guard = done_fifo.map(FifoSignalGuard::new);

    // Launch editor
    let status = run_editor(&editor_config.editor_command, document_path, window_title)?;

    if !status.success() {
        eprintln!("Editor exited with non-zero status");
        return Ok(());
    }

    // After editor exits, parse the document and call the handler
    let document = Document::<S>::from_path(document_path.to_path_buf())?;
    handler.on_review_complete(&document)
    // FifoSignalGuard writes to the FIFO on drop (even on error/panic)
}

/// Create a FIFO (named pipe) for signaling review completion.
///
/// The FIFO path is derived from the document path with a `.done` extension.
fn create_done_fifo(document_path: &Path) -> Result<PathBuf> {
    let fifo_path = fifo_path_for(document_path);

    // Remove stale FIFO if it exists
    let _ = std::fs::remove_file(&fifo_path);

    let status = std::process::Command::new("mkfifo")
        .arg(&fifo_path)
        .status()?;

    if !status.success() {
        return Err(HumanInTheLoopError::CommandFailed(
            "Failed to create FIFO".to_string(),
        ));
    }

    Ok(fifo_path)
}

/// Derive the FIFO path from a document path.
fn fifo_path_for(document_path: &Path) -> PathBuf {
    let mut p = document_path.as_os_str().to_os_string();
    p.push(".done");
    PathBuf::from(p)
}

/// Block until data is available on the FIFO, then consume it.
fn wait_for_fifo(fifo_path: &Path) -> Result<()> {
    // Opening a FIFO for reading blocks until a writer opens the other end.
    // Once the writer writes and closes, read returns and we unblock.
    use std::io::Read;
    let mut file = std::fs::File::open(fifo_path)?;
    let mut buf = [0u8; 1];
    let _ = file.read(&mut buf);
    Ok(())
}

/// RAII guard that removes the FIFO file on drop.
///
/// Used in `start_review` to ensure the FIFO is cleaned up on early-return
/// error paths (e.g., if `current_exe()` or `launch_terminal()` fails).
struct FifoCleanupGuard {
    fifo_path: PathBuf,
    disarmed: bool,
}

impl FifoCleanupGuard {
    fn new(fifo_path: &Path) -> Self {
        Self {
            fifo_path: fifo_path.to_path_buf(),
            disarmed: false,
        }
    }

    fn disarm(&mut self) {
        self.disarmed = true;
    }
}

impl Drop for FifoCleanupGuard {
    fn drop(&mut self) {
        if !self.disarmed {
            let _ = std::fs::remove_file(&self.fifo_path);
        }
    }
}

/// RAII guard that writes to a FIFO on drop to signal the waiting process.
///
/// Ensures the FIFO is signaled even if the review-complete process panics
/// or encounters an error, preventing the parent process from hanging.
///
/// Uses `O_NONBLOCK` when opening the FIFO so that if the parent reader
/// process has already exited (e.g., Ctrl+C), the write silently fails
/// instead of blocking forever.
struct FifoSignalGuard {
    fifo_path: PathBuf,
}

impl FifoSignalGuard {
    fn new(fifo_path: &Path) -> Self {
        Self {
            fifo_path: fifo_path.to_path_buf(),
        }
    }
}

impl Drop for FifoSignalGuard {
    fn drop(&mut self) {
        signal_fifo(&self.fifo_path);
    }
}

/// Write to a FIFO in non-blocking mode.
///
/// Uses `O_WRONLY | O_NONBLOCK` so the open returns ENXIO immediately
/// if no reader has the FIFO open, instead of blocking forever.
fn signal_fifo(fifo_path: &Path) {
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;

        let file = std::fs::OpenOptions::new()
            .write(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(fifo_path);

        if let Ok(mut f) = file {
            let _ = f.write_all(b"0");
        }
    }
}
