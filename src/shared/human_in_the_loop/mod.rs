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
pub mod exit_code;
mod lock;
mod tmux;

pub use approval::ApprovalManager;
pub use document::{Document, DocumentSchema};
pub use editor::{LaunchOptions, launch_terminal, run_editor};
pub use error::{HumanInTheLoopError, Result};
pub use lock::{CleanupGuard, LockGuard};
pub use tmux::get_tmux_target;

use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::shared::config::EditorConfig;
use crate::shared::diff::write_diff;

/// How long to wait for the terminal to signal it has started running the
/// review-complete command before giving up. Covers cases like macOS being
/// asleep when Claude Code invokes the command, where Ghostty's AppleScript
/// call succeeds but the terminal window never initializes.
const TERMINAL_STARTUP_TIMEOUT: Duration = Duration::from_secs(10);

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
/// 1. Checks for an existing lock (another editor already open)
/// 2. Creates FIFOs for startup and completion signaling
/// 3. Launches the configured terminal to run the review-complete command
/// 4. Waits (with timeout) for the terminal to signal it actually started
/// 5. Blocks until the review-complete process signals completion via the FIFO
///
/// The lock file is written by `complete_review`, not here, so a failed
/// terminal launch never leaves a stale lock behind.
///
/// Returns the final document state after the user finishes editing, or `None`
/// if the editor was already open. Returns `TerminalLaunchFailed` if the
/// terminal emulator doesn't start within the startup timeout (e.g., macOS
/// asleep).
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

    // Check for existing lock. The lock is written by `complete_review` once
    // the editor actually launches, so its presence means another session is
    // in progress for this document.
    if LockGuard::is_locked(document_path) {
        eprintln!("Skipped: Editor is already open for this file.");
        return Ok(None);
    }

    // Snapshot the document before the editor opens so we can emit a diff
    // afterwards. Background callers (e.g. Claude Code skills launching this
    // command via run_in_background) read the diff from stdout instead of
    // re-reading the whole file, which keeps token usage proportional to the
    // edit size rather than the file size.
    let pre_edit = std::fs::read_to_string(document_path)?;

    // Create a FIFO for the review-complete process to signal completion.
    // The FifoCleanupGuard ensures the FIFO is removed on early return.
    let done_fifo_path = create_fifo_with_suffix(document_path, ".done")?;
    let mut done_fifo_cleanup = FifoCleanupGuard::new(&done_fifo_path);

    // Create a second FIFO the wrapper shell writes to just before exec'ing
    // the review-complete command. If the terminal emulator fails to launch
    // (e.g., macOS is asleep and Ghostty can't initialize), this signal never
    // arrives and we bail out with TerminalLaunchFailed instead of hanging
    // forever waiting on the done FIFO.
    let started_fifo_path = create_fifo_with_suffix(document_path, ".started")?;
    let mut started_fifo_cleanup = FifoCleanupGuard::new(&started_fifo_path);

    // Open the FIFO reader *before* launching the terminal to prevent a race
    // condition: on Linux, launch_terminal blocks until the terminal closes,
    // so complete_review signals the FIFO before we reach the read call.
    // By opening the reader first (with O_NONBLOCK to avoid blocking on open),
    // the writer's signal is buffered in the pipe and read picks it up later.
    let done_fifo_reader = open_fifo_reader(&done_fifo_path)?;
    let started_fifo_reader = open_fifo_reader(&started_fifo_path)?;

    // Get tmux session info for later restoration
    let tmux_target = get_tmux_target();

    // Get the path to the current executable
    let exe_path = std::env::current_exe()?;

    // Build the review-complete command arguments and append --done-fifo
    let mut review_args =
        handler.build_complete_args(document_path, tmux_target.as_deref(), window_title);
    review_args.push("--done-fifo".into());
    review_args.push(done_fifo_path.as_os_str().to_os_string());

    // Launch terminal emulator
    let options = LaunchOptions {
        window_title: window_title.to_string(),
        ..Default::default()
    };

    let outcome = launch_terminal(
        &editor_config.terminal,
        &options,
        &exe_path,
        &review_args,
        Some(&started_fifo_path),
    )?;

    if !outcome.status.success() {
        return Err(HumanInTheLoopError::CommandFailed(format!(
            "Terminal exited with status: {}",
            outcome.status
        )));
    }

    // If the launcher supports started-FIFO signaling, wait up to the startup
    // timeout for the wrapper shell to announce itself. A timeout here means
    // the terminal window never actually opened — report that distinctly so
    // callers can retry (and, since no lock was created, nothing to clean up).
    if outcome.signals_started {
        wait_for_fifo_signal_with_timeout(
            started_fifo_reader,
            &started_fifo_path,
            TERMINAL_STARTUP_TIMEOUT,
        )?;
    } else {
        drop(started_fifo_reader);
    }

    // Started FIFO has done its job — let the cleanup guard remove it.
    started_fifo_cleanup.disarm();
    let _ = std::fs::remove_file(&started_fifo_path);

    // Wait for the review-complete process to finish via FIFO.
    // This blocks with no CPU usage until the other process writes to the FIFO.
    wait_for_fifo_signal(done_fifo_reader, &done_fifo_path)?;

    // Clean up the FIFO (disarm guard since we're cleaning up explicitly)
    done_fifo_cleanup.disarm();
    let _ = std::fs::remove_file(&done_fifo_path);

    // Emit a diff of what the user edited so background callers can pick it
    // up from stdout without re-reading the file. Errors writing to stdout
    // (e.g. BrokenPipe when piped to `head`) are intentionally ignored.
    let post_edit = std::fs::read_to_string(document_path).unwrap_or_default();
    let _ = print_edit_diff(&pre_edit, &post_edit);

    // Review complete - read the final document state
    let document = Document::<S>::from_path(document_path.to_path_buf())?;
    Ok(Some(document))
}

/// Write a unified diff of pre/post edit content to stdout, or `(no edits)`
/// when unchanged. Color is enabled only when stdout is a TTY.
fn print_edit_diff(pre: &str, post: &str) -> std::io::Result<()> {
    use crossterm::tty::IsTty;
    let use_color = std::io::stdout().is_tty();
    let mut stdout = std::io::stdout().lock();
    write_edit_diff(&mut stdout, pre, post, use_color)
}

/// Emit a diff between `pre` and `post` to `writer`, or a "(no edits)" notice
/// when they match. Factored out for unit testing.
fn write_edit_diff<W: Write>(
    writer: &mut W,
    pre: &str,
    post: &str,
    use_color: bool,
) -> std::io::Result<()> {
    if pre == post {
        writeln!(writer, "(no edits)")?;
        return Ok(());
    }
    write_diff(writer, pre, post, use_color)
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
/// The caller is responsible for creating a `FifoSignalGuard` before calling this
/// function to ensure the FIFO is signaled even if earlier initialization fails.
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
    // Create the lock file here (not in start_review) so it only exists once
    // the terminal has actually launched and we're about to run the editor.
    // If the terminal never starts (e.g., macOS asleep), no lock is written
    // and the next invocation isn't blocked by a stale `.lock`.
    let mut lock_guard = LockGuard::acquire(document_path)?;
    lock_guard.disarm();

    // Ensure cleanup happens even on panic.
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

/// Create a FIFO (named pipe) next to `document_path` with the given suffix.
///
/// Example: suffix `.done` for `/tmp/foo.md` yields `/tmp/foo.md.done`.
#[cfg(unix)]
fn create_fifo_with_suffix(document_path: &Path, suffix: &str) -> Result<PathBuf> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let mut p = document_path.as_os_str().to_os_string();
    p.push(suffix);
    let fifo_path = PathBuf::from(p);

    // Remove stale FIFO if it exists
    let _ = std::fs::remove_file(&fifo_path);

    let c_path = CString::new(fifo_path.as_os_str().as_bytes()).map_err(|e| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("FIFO path contains interior NUL byte: {e}"),
        )
    })?;

    // SAFETY: `c_path` is a valid NUL-terminated C string owned by this scope.
    // Mode 0o600 restricts the FIFO to the current user, matching the access
    // the document itself has.
    let ret = unsafe { libc::mkfifo(c_path.as_ptr(), 0o600) };
    if ret != 0 {
        return Err(std::io::Error::last_os_error().into());
    }

    Ok(fifo_path)
}

/// Open a FIFO for reading in non-blocking mode.
///
/// Must be called *before* launching the terminal so the reader end is
/// connected when the writer signals. `O_NONBLOCK` is used so the open
/// returns immediately even without a writer.
#[cfg(unix)]
fn open_fifo_reader(fifo_path: &Path) -> Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;

    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(fifo_path)?;

    Ok(file)
}

/// Block until the writer sends a signal byte on the FIFO.
///
/// On Linux, clearing `O_NONBLOCK` and reading blocks until data arrives.
/// On macOS, however, reading a FIFO with no writer attached returns `Ok(0)`
/// (spurious EOF) even after clearing `O_NONBLOCK`. To handle this portably,
/// when `read` returns `Ok(0)` we close the fd and re-open the FIFO in
/// blocking mode. The blocking `open` itself will wait until a writer connects,
/// and the subsequent `read` will return the signal byte.
#[cfg(unix)]
fn wait_for_fifo_signal(file: std::fs::File, fifo_path: &Path) -> Result<()> {
    use std::io::{self, Read};
    use std::os::unix::io::AsRawFd;

    // Clear O_NONBLOCK so read blocks until data arrives
    let fd = file.as_raw_fd();
    unsafe {
        let flags = libc::fcntl(fd, libc::F_GETFL);
        if flags != -1 {
            libc::fcntl(fd, libc::F_SETFL, flags & !libc::O_NONBLOCK);
        }
    }

    let mut file = file;
    let mut buf = [0u8; 1];
    loop {
        match file.read(&mut buf) {
            Ok(n) if n > 0 => break,
            Ok(_) => {
                // Ok(0) = spurious EOF (macOS: no writer connected yet).
                // Re-open in blocking mode; the open itself blocks until a writer connects.
                // Open the new fd before dropping the old one to maintain continuous
                // reader presence on the FIFO, preventing the writer's O_NONBLOCK open
                // from getting ENXIO.
                file = std::fs::File::open(fifo_path)?;
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        }
    }
    Ok(())
}

/// Like `wait_for_fifo_signal` but gives up after `timeout`, returning a
/// `TerminalLaunchFailed` error. Used for the started-FIFO so a dead terminal
/// emulator doesn't cause the parent process to hang.
///
/// On macOS a FIFO with no writer attached returns `Ok(0)` from `read` (spurious
/// EOF). To handle that while honoring a deadline we stay in non-blocking mode
/// and poll with short sleeps until either data arrives or the deadline passes.
#[cfg(unix)]
fn wait_for_fifo_signal_with_timeout(
    file: std::fs::File,
    _fifo_path: &Path,
    timeout: Duration,
) -> Result<()> {
    use std::io::{self, Read};
    use std::thread::sleep;
    use std::time::Instant;

    let deadline = Instant::now() + timeout;
    let mut file = file;
    let mut buf = [0u8; 1];
    let poll_interval = Duration::from_millis(100);
    loop {
        match file.read(&mut buf) {
            Ok(n) if n > 0 => return Ok(()),
            Ok(_) => {
                // Ok(0) = no writer yet (macOS) or EOF. Keep waiting.
            }
            Err(e)
                if e.kind() == io::ErrorKind::WouldBlock
                    || e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e.into()),
        }
        if Instant::now() >= deadline {
            return Err(HumanInTheLoopError::TerminalLaunchFailed {
                timeout_secs: timeout.as_secs(),
            });
        }
        sleep(poll_interval);
    }
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
///
/// Should be created as early as possible in the review-complete process
/// to ensure signaling even if later initialization (e.g., config loading) fails.
pub struct FifoSignalGuard {
    fifo_path: PathBuf,
}

impl FifoSignalGuard {
    pub fn new(fifo_path: &Path) -> Self {
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

#[cfg(test)]
#[cfg(unix)]
mod tests {
    use super::*;
    use rstest::{fixture, rstest};
    use tempfile::TempDir;

    struct FifoReaderFixture {
        _temp: TempDir,
        fifo_path: PathBuf,
        reader: std::fs::File,
    }

    #[fixture]
    fn fifo_reader() -> FifoReaderFixture {
        let temp = TempDir::new().expect("tempdir");
        let doc = temp.path().join("doc.md");
        std::fs::write(&doc, "").expect("write doc");
        let fifo_path = create_fifo_with_suffix(&doc, ".started").expect("create fifo");
        let reader = open_fifo_reader(&fifo_path).expect("open reader");
        FifoReaderFixture {
            _temp: temp,
            fifo_path,
            reader,
        }
    }

    #[rstest]
    fn wait_for_fifo_signal_with_timeout_times_out_without_writer(fifo_reader: FifoReaderFixture) {
        let start = std::time::Instant::now();
        let err = wait_for_fifo_signal_with_timeout(
            fifo_reader.reader,
            &fifo_reader.fifo_path,
            Duration::from_millis(300),
        )
        .expect_err("expected timeout");
        let elapsed = start.elapsed();

        assert!(matches!(
            err,
            HumanInTheLoopError::TerminalLaunchFailed { .. }
        ));
        assert!(
            elapsed >= Duration::from_millis(250),
            "returned too early: {elapsed:?}"
        );
    }

    #[rstest]
    fn wait_for_fifo_signal_with_timeout_returns_ok_when_signaled(fifo_reader: FifoReaderFixture) {
        // Spawn a thread that signals the FIFO after a short delay.
        let writer_path = fifo_reader.fifo_path.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            signal_fifo(&writer_path);
        });

        wait_for_fifo_signal_with_timeout(
            fifo_reader.reader,
            &fifo_reader.fifo_path,
            Duration::from_secs(2),
        )
        .expect("expected Ok");
    }

    #[rstest]
    fn create_fifo_with_suffix_appends_suffix() {
        let temp = TempDir::new().expect("tempdir");
        let doc = temp.path().join("foo.md");
        std::fs::write(&doc, "").expect("write doc");

        let fifo = create_fifo_with_suffix(&doc, ".started").expect("create fifo");
        assert_eq!(
            fifo.file_name().and_then(|s| s.to_str()),
            Some("foo.md.started")
        );

        use std::os::unix::fs::FileTypeExt;
        let metadata = std::fs::metadata(&fifo).expect("stat fifo");
        assert!(metadata.file_type().is_fifo(), "expected FIFO file type");
    }

    // The exact diff format is covered by `shared::diff` tests; here we only
    // verify the unique behavior layered on top: emit "(no edits)" for
    // identical content, otherwise delegate to the underlying diff writer.

    #[rstest]
    #[case::identical_content("same\n", "same\n")]
    #[case::both_empty("", "")]
    fn write_edit_diff_emits_no_edits_marker_when_unchanged(#[case] pre: &str, #[case] post: &str) {
        let mut buf = Vec::new();
        write_edit_diff(&mut buf, pre, post, false).expect("write");
        assert_eq!(String::from_utf8(buf).expect("utf8"), "(no edits)\n");
    }

    #[rstest]
    fn write_edit_diff_delegates_to_write_diff_when_changed() {
        let mut buf = Vec::new();
        write_edit_diff(&mut buf, "old\n", "new\n", false).expect("write");
        let out = String::from_utf8(buf).expect("utf8");
        assert_ne!(out, "(no edits)\n");
        assert_ne!(out, "");
    }
}
