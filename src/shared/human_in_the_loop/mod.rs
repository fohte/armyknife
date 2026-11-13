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
mod launch;
mod lock;
mod notification;
mod pending_review;
mod queue_entry;
mod review_completion;
mod review_finish;
mod session_watch;
mod tmux;
pub(crate) use pending_review::{
    ReviewDocumentMarker, review_document_marker, review_document_path,
};
pub(crate) use review_completion::complete_review;
pub(crate) use review_finish::{
    FINISH_ACK_TIMEOUT, ReviewDocumentOperationLock, finish_ack_fifo_path,
    finish_request_fifo_path, open_fifo_reader, wait_for_finish_ack,
};
use review_finish::{
    create_fifo_at_path, path_with_suffix, remove_fifo_if_present, try_read_fifo_signal,
};

pub use approval::{ApprovalManager, TEST_APPROVAL_DIR_OVERRIDE};
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
use crate::shared::diff::write_unified_diff;

const DIFF_CONTEXT_LINES: usize = 3;

/// Distinguishes PR draft reviews from other Human-in-the-Loop reviews.
pub(crate) const PR_DRAFT_REVIEW_MARKER: &str = "pr-draft-review:";

/// How long to wait for the terminal to signal it has started running the
/// review-complete command before giving up. Covers cases like macOS being
/// asleep when Claude Code invokes the command, where Ghostty's AppleScript
/// call succeeds but the terminal window never initializes.
const TERMINAL_STARTUP_TIMEOUT: Duration = Duration::from_secs(10);
const FIFO_POLL_INTERVAL: Duration = Duration::from_millis(100);
const REVIEW_SESSION_CHECK_INTERVAL: Duration = Duration::from_secs(2);
const MAX_CONSECUTIVE_SESSION_CHECK_FAILURES: usize = 3;

/// Trait for types that handle the review completion callback.
///
/// Each use case (PR draft, issue comment, PR review reply, or simple file editing)
/// implements this trait to define how to handle the document after the user finishes editing.
pub trait ReviewHandler<S: DocumentSchema> {
    /// Command label shown in review notification titles.
    const NOTIFICATION_COMMAND_NAME: &'static str = "review";
    /// Marker recorded on the parent agent session while this review is open.
    const PENDING_REVIEW_MARKER: Option<&'static str> = None;

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

/// Start a review session in a tmux floating pane or configured terminal window.
///
/// This function:
/// 1. Checks for an existing lock (another editor already open)
/// 2. Creates a FIFO for completion signaling
/// 3. Launches the review-complete command in a tmux floating pane or terminal
/// 4. Blocks until the review-complete process signals completion via the FIFO
///
/// The parent and child both record their process IDs. The lock remains active
/// while either process is alive and is reclaimed when all recorded processes exit.
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
    notifications_enabled: bool,
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

    let tmux_pane_id = crate::shared::caller_pane::resolve_caller_pane_id();
    let document_path = if tmux_pane_id.is_some() {
        std::fs::canonicalize(document_path)?
    } else {
        document_path.to_path_buf()
    };

    let mut session_watch = session_watch::SessionWatch::new();
    // Held until the review ends, then shows the next queued review.
    let mut queue_entry = None;
    start_review_with_marker::<S, _, _>(
        &document_path,
        <H as ReviewHandler<S>>::PENDING_REVIEW_MARKER,
        |document_path, done_fifo_path, finish_fifo_path, lock_id| {
            let context = launch::ReviewContext {
                lock_id,
                done_fifo_path,
                finish_fifo_path,
                window_title,
                command_name: <H as ReviewHandler<S>>::NOTIFICATION_COMMAND_NAME,
            };
            let (session, entry) = launch::launch_review::<S, H>(
                tmux_pane_id.as_deref(),
                document_path,
                &context,
                handler,
                editor_config,
                notifications_enabled,
            )?;
            queue_entry = entry;
            Ok(ReviewLaunch {
                session,
                queue_key: queue_entry.as_ref().map(|entry| entry.key().to_string()),
            })
        },
        |session| {
            session_watch.is_running(std::time::Instant::now(), || {
                crate::infra::shpool::session_is_alive(session)
                    .map_err(|error| HumanInTheLoopError::CommandFailed(format!("{error:#}")))
            })
        },
    )
}

#[cfg(test)]
fn start_review_with_launcher<S, L, P>(
    document_path: &Path,
    launch_review: L,
    is_session_alive: P,
) -> Result<Option<Document<S>>>
where
    S: DocumentSchema,
    L: FnOnce(&Path, &Path, &str) -> Result<Option<String>>,
    P: FnMut(&str) -> Result<bool>,
{
    start_review_with_marker(
        document_path,
        None,
        |document_path, done_fifo_path, _finish_fifo_path, lock_id| {
            launch_review(document_path, done_fifo_path, lock_id).map(ReviewLaunch::without_queue)
        },
        is_session_alive,
    )
}

struct ReviewLaunch {
    session: Option<String>,
    queue_key: Option<String>,
}

impl ReviewLaunch {
    #[cfg(test)]
    fn without_queue(session: Option<String>) -> Self {
        Self {
            session,
            queue_key: None,
        }
    }
}

fn start_review_with_marker<S, L, P>(
    document_path: &Path,
    pending_review_marker: Option<&str>,
    launch_review: L,
    is_session_alive: P,
) -> Result<Option<Document<S>>>
where
    S: DocumentSchema,
    L: FnOnce(&Path, &Path, &Path, &str) -> Result<ReviewLaunch>,
    P: FnMut(&str) -> Result<bool>,
{
    let lock_path = LockGuard::lock_path(document_path);
    let lock_guard = match LockGuard::acquire(document_path) {
        Ok(guard) => guard,
        Err(HumanInTheLoopError::Io(error))
            if error.kind() == std::io::ErrorKind::AlreadyExists =>
        {
            eprintln!("{}", editor_already_open_message(&lock_path));
            return Ok(None);
        }
        Err(error) => return Err(error),
    };

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

    // Open the reader before launching either surface so a fast completion
    // signal cannot race with the read call.
    let done_fifo_reader = open_fifo_reader(&done_fifo_path)?;

    let finish_fifo_path = create_fifo_at_path(&finish_request_fifo_path(document_path))?;
    let mut finish_fifo_cleanup = FifoCleanupGuard::new(&finish_fifo_path);
    let finish_fifo_reader = open_fifo_reader(&finish_fifo_path)?;
    let finish_ack_fifo_path = create_fifo_at_path(&finish_ack_fifo_path(document_path))?;
    let mut finish_ack_fifo_cleanup = FifoCleanupGuard::new(&finish_ack_fifo_path);

    let review = launch_review(
        document_path,
        &done_fifo_path,
        &finish_fifo_path,
        lock_guard.lock_id(),
    )?;

    {
        let _pending_review = pending_review::PendingReviewGuard::for_current_session(
            pending_review_marker,
            document_path,
            review.queue_key.as_deref(),
        );
        if let Some(session) = review.session.as_deref() {
            wait_for_fifo_signal_or_session_exit(done_fifo_reader, session, is_session_alive)?;
        } else {
            // GUI terminal launches have no tmux pane to monitor.
            wait_for_fifo_signal(done_fifo_reader, &done_fifo_path)?;
        }
    }

    drop(finish_fifo_reader);
    finish_fifo_cleanup.disarm();
    let _ = remove_fifo_if_present(&finish_fifo_path);
    finish_ack_fifo_cleanup.disarm();
    let _ = remove_fifo_if_present(&finish_ack_fifo_path);

    // Clean up the FIFO (disarm guard since we're cleaning up explicitly)
    done_fifo_cleanup.disarm();
    let _ = std::fs::remove_file(&done_fifo_path);

    // Emit a diff of what the user edited so background callers can pick it
    // up from stdout without re-reading the file.
    let post_edit = std::fs::read_to_string(document_path)?;
    if let Err(e) = print_edit_diff(&pre_edit, &post_edit)
        && e.kind() != std::io::ErrorKind::BrokenPipe
    {
        return Err(e.into());
    }

    // Reuse the in-memory `post_edit` for the document parse to avoid a
    // second read of the same file.
    let document = Document::<S>::from_content(document_path.to_path_buf(), &post_edit)?;
    Ok(Some(document))
}

fn editor_already_open_message(lock_path: &Path) -> String {
    format!(
        "Skipped: Editor is already open for this file (lock: {}).",
        lock_path.display()
    )
}

/// Write a unified diff of pre/post edit content to stdout, or `(no edits)`
/// when unchanged. Color is enabled only when stdout is a TTY.
fn print_edit_diff(pre: &str, post: &str) -> std::io::Result<()> {
    use crossterm::tty::IsTty;
    let use_color = std::io::stdout().is_tty();
    let mut stdout = std::io::stdout().lock();
    write_edit_diff(&mut stdout, pre, post, use_color)
}

/// Emit a hunk-based unified diff between `pre` and `post` to `writer`, or a
/// "(no edits)" notice when they match. Factored out for unit testing.
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
    write_unified_diff(writer, pre, post, DIFF_CONTEXT_LINES, use_color)
}

/// Create a FIFO (named pipe) next to `document_path` with the given suffix.
///
/// Example: suffix `.done` for `/tmp/foo.md` yields `/tmp/foo.md.done`.
#[cfg(unix)]
fn create_fifo_with_suffix(document_path: &Path, suffix: &str) -> Result<PathBuf> {
    create_fifo_at_path(&path_with_suffix(document_path, suffix))
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

/// Wait for the completion signal while periodically checking the editor session.
#[cfg(unix)]
fn wait_for_fifo_signal_or_session_exit(
    file: std::fs::File,
    session: &str,
    is_session_alive: impl FnMut(&str) -> Result<bool>,
) -> Result<()> {
    wait_for_fifo_signal_or_session_exit_with_interval(
        file,
        session,
        REVIEW_SESSION_CHECK_INTERVAL,
        is_session_alive,
    )
}

fn wait_for_fifo_signal_or_session_exit_with_interval(
    file: std::fs::File,
    session: &str,
    check_interval: Duration,
    mut is_session_alive: impl FnMut(&str) -> Result<bool>,
) -> Result<()> {
    use std::time::Instant;

    let mut next_session_check = Instant::now();
    let mut consecutive_check_failures = 0;
    poll_fifo_signal(file, |file| {
        if Instant::now() >= next_session_check {
            match is_session_alive(session) {
                Ok(true) => consecutive_check_failures = 0,
                Ok(false) => {
                    consecutive_check_failures = 0;
                    return Ok(if try_read_fifo_signal(file)? {
                        FifoPollAction::Signaled
                    } else {
                        FifoPollAction::Abort(HumanInTheLoopError::ReviewSessionClosed)
                    });
                }
                Err(error) => {
                    consecutive_check_failures += 1;
                    tracing::warn!(
                        session,
                        consecutive_check_failures,
                        max_consecutive_failures = MAX_CONSECUTIVE_SESSION_CHECK_FAILURES,
                        error = %error,
                        "Review session liveness check failed"
                    );
                    if consecutive_check_failures >= MAX_CONSECUTIVE_SESSION_CHECK_FAILURES {
                        return Err(error);
                    }
                }
            }
            next_session_check = Instant::now() + check_interval;
        }
        Ok(FifoPollAction::Continue)
    })
}

enum FifoPollAction {
    Continue,
    Signaled,
    Abort(HumanInTheLoopError),
}

fn poll_fifo_signal(
    mut file: std::fs::File,
    mut check: impl FnMut(&mut std::fs::File) -> Result<FifoPollAction>,
) -> Result<()> {
    use std::thread::sleep;

    loop {
        if try_read_fifo_signal(&mut file)? {
            return Ok(());
        }

        match check(&mut file)? {
            FifoPollAction::Continue => sleep(FIFO_POLL_INTERVAL),
            FifoPollAction::Signaled => return Ok(()),
            FifoPollAction::Abort(error) => return Err(error),
        }
    }
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
    use std::time::Instant;

    let deadline = Instant::now() + timeout;
    poll_fifo_signal(file, |_| {
        if Instant::now() >= deadline {
            Ok(FifoPollAction::Abort(
                HumanInTheLoopError::TerminalLaunchFailed {
                    timeout_secs: timeout.as_secs(),
                },
            ))
        } else {
            Ok(FifoPollAction::Continue)
        }
    })
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
            let _ = remove_fifo_if_present(&self.fifo_path);
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
pub(crate) fn signal_fifo(fifo_path: &Path) {
    let _ = try_signal_fifo(fifo_path);
}

pub(crate) fn try_signal_fifo(fifo_path: &Path) -> std::io::Result<()> {
    try_signal_fifo_with_byte(fifo_path, b'0')
}

pub(crate) fn try_signal_fifo_with_byte(fifo_path: &Path, byte: u8) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::{FileTypeExt, OpenOptionsExt};

        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW)
            .open(fifo_path)?;
        if !file.metadata()?.file_type().is_fifo() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "review signal path is not a FIFO",
            ));
        }
        file.write_all(&[byte])?;
    }

    #[cfg(not(unix))]
    let _ = (fifo_path, byte);

    Ok(())
}

#[cfg(test)]
#[cfg(unix)]
mod tests {
    use std::cell::Cell;

    use super::*;
    use indoc::indoc;
    use rstest::{fixture, rstest};
    use serde::{Deserialize, Serialize};
    use tempfile::TempDir;

    #[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
    struct ReviewTestSchema;

    impl DocumentSchema for ReviewTestSchema {
        fn is_approved(&self) -> bool {
            false
        }
    }

    struct ReviewDocumentFixture {
        _temp_dir: TempDir,
        path: PathBuf,
    }

    #[fixture]
    fn review_document() -> ReviewDocumentFixture {
        let temp_dir = TempDir::new().expect("tempdir");
        let path = temp_dir.path().join("review.md");
        std::fs::write(&path, "draft").expect("write document");
        ReviewDocumentFixture {
            _temp_dir: temp_dir,
            path,
        }
    }

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
    fn wait_for_fifo_signal_or_session_exit_prefers_a_completion_signal(
        fifo_reader: FifoReaderFixture,
    ) {
        signal_fifo(&fifo_reader.fifo_path);

        let result =
            wait_for_fifo_signal_or_session_exit(fifo_reader.reader, "review-session", |_| {
                Ok(false)
            })
            .map_err(|error| error.to_string());

        assert_eq!(result, Ok(()));
    }

    #[rstest]
    fn wait_for_fifo_signal_or_session_exit_reads_signal_written_before_session_ends(
        fifo_reader: FifoReaderFixture,
    ) {
        let writer_path = fifo_reader.fifo_path.clone();
        let result =
            wait_for_fifo_signal_or_session_exit(fifo_reader.reader, "review-session", move |_| {
                signal_fifo(&writer_path);
                Ok(false)
            })
            .map_err(|error| error.to_string());

        assert_eq!(result, Ok(()));
    }

    #[rstest]
    fn wait_for_fifo_signal_or_session_exit_completes_while_session_is_alive(
        fifo_reader: FifoReaderFixture,
    ) {
        let writer_path = fifo_reader.fifo_path.clone();
        let writer = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            signal_fifo(&writer_path);
        });

        let result =
            wait_for_fifo_signal_or_session_exit(fifo_reader.reader, "review-session", |_| {
                Ok(true)
            })
            .map_err(|error| error.to_string());
        writer.join().expect("writer thread");

        assert_eq!(result, Ok(()));
    }

    #[rstest]
    fn wait_for_fifo_signal_or_session_exit_retries_a_transient_session_query_failure(
        fifo_reader: FifoReaderFixture,
    ) {
        let checks = Cell::new(0);
        let writer_path = fifo_reader.fifo_path.clone();
        let result = wait_for_fifo_signal_or_session_exit_with_interval(
            fifo_reader.reader,
            "review-session",
            Duration::ZERO,
            |_| {
                let check_count = checks.get() + 1;
                checks.set(check_count);
                if check_count == 1 {
                    return Err(HumanInTheLoopError::CommandFailed(
                        "tmux is unavailable".into(),
                    ));
                }

                signal_fifo(&writer_path);
                Ok(true)
            },
        )
        .map_err(|error| error.to_string());

        assert_eq!((result, checks.get()), (Ok(()), 2));
    }

    #[rstest]
    fn wait_for_fifo_signal_or_session_exit_resets_failures_after_a_successful_check(
        fifo_reader: FifoReaderFixture,
    ) {
        let checks = Cell::new(0);
        let writer_path = fifo_reader.fifo_path.clone();
        let result = wait_for_fifo_signal_or_session_exit_with_interval(
            fifo_reader.reader,
            "review-session",
            Duration::ZERO,
            |_| {
                let check_count = checks.get() + 1;
                checks.set(check_count);
                match check_count {
                    1 | 3 | 4 => Err(HumanInTheLoopError::CommandFailed(
                        "tmux is unavailable".into(),
                    )),
                    5 => {
                        signal_fifo(&writer_path);
                        Ok(true)
                    }
                    _ => Ok(true),
                }
            },
        )
        .map_err(|error| error.to_string());

        assert_eq!((result, checks.get()), (Ok(()), 5));
    }

    #[rstest]
    fn wait_for_fifo_signal_or_session_exit_stops_after_consecutive_session_query_failures(
        fifo_reader: FifoReaderFixture,
    ) {
        let checks = Cell::new(0);
        let result = wait_for_fifo_signal_or_session_exit_with_interval(
            fifo_reader.reader,
            "review-session",
            Duration::ZERO,
            |_| {
                checks.set(checks.get() + 1);
                Err(HumanInTheLoopError::CommandFailed(
                    "tmux is unavailable".into(),
                ))
            },
        )
        .map_err(|error| error.to_string());

        assert_eq!(
            (result, checks.get()),
            (
                Err("Command failed: tmux is unavailable".into()),
                MAX_CONSECUTIVE_SESSION_CHECK_FAILURES,
            ),
        );
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

    #[rstest]
    fn create_fifo_at_path_preserves_an_existing_regular_file() {
        let temp = TempDir::new().expect("tempdir");
        let fifo_path = temp.path().join("example-review.md.finish");
        let original_contents = "keep this file";
        std::fs::write(&fifo_path, original_contents).expect("write existing file");

        let result = create_fifo_at_path(&fifo_path);
        let existing_contents = std::fs::read_to_string(&fifo_path).expect("read existing file");
        let rejected_existing_file = matches!(
            result,
            Err(HumanInTheLoopError::Io(error))
                if error.kind() == std::io::ErrorKind::AlreadyExists
        );

        assert_eq!(
            (rejected_existing_file, existing_contents),
            (true, original_contents.to_string()),
        );
    }

    #[rstest]
    fn start_review_releases_parent_lock_after_done_fifo_signal(
        review_document: ReviewDocumentFixture,
    ) {
        let document_path = review_document.path;
        let mut lock_was_held_during_launch = false;

        let result = start_review_with_launcher::<ReviewTestSchema, _, _>(
            &document_path,
            |path, done_fifo_path, _lock_id| {
                lock_was_held_during_launch = LockGuard::is_locked(path);
                signal_fifo(done_fifo_path);
                Ok(None)
            },
            |_| Ok(true),
        )
        .expect("start review")
        .map(|document| (document.path, document.frontmatter));

        assert_eq!(
            (
                result,
                lock_was_held_during_launch,
                LockGuard::is_locked(&document_path),
            ),
            (Some((document_path, ReviewTestSchema)), true, false,),
        );
    }

    #[rstest]
    fn start_review_replaces_a_stale_lock_and_proceeds(review_document: ReviewDocumentFixture) {
        let document_path = review_document.path;
        let lock_path = LockGuard::lock_path(&document_path);
        std::fs::write(
            &lock_path,
            indoc! {"
                armyknife-review-lock-v1
                id=expired-session
                pid=4294967295
            "},
        )
        .expect("write stale lock");
        let mut lock_was_held_during_launch = false;

        let result = start_review_with_launcher::<ReviewTestSchema, _, _>(
            &document_path,
            |path, done_fifo_path, _lock_id| {
                lock_was_held_during_launch = LockGuard::is_locked(path);
                signal_fifo(done_fifo_path);
                Ok(None)
            },
            |_| Ok(true),
        )
        .expect("start review")
        .map(|document| (document.path, document.frontmatter));

        assert_eq!(
            (
                result,
                lock_was_held_during_launch,
                LockGuard::is_locked(&document_path),
            ),
            (Some((document_path, ReviewTestSchema)), true, false,),
        );
    }

    #[rstest]
    fn start_review_releases_parent_lock_when_launch_fails(review_document: ReviewDocumentFixture) {
        let document_path = review_document.path;
        let result = start_review_with_launcher::<ReviewTestSchema, _, _>(
            &document_path,
            |_, _, _| Err(HumanInTheLoopError::CommandFailed("launch failed".into())),
            |_| Ok(true),
        )
        .map(|document| document.map(|document| document.path))
        .map_err(|error| error.to_string());

        assert_eq!(
            (result, LockGuard::is_locked(&document_path)),
            (Err("Command failed: launch failed".into()), false),
        );
    }

    #[rstest]
    fn start_review_does_not_remove_a_newer_review_lock(review_document: ReviewDocumentFixture) {
        let document_path = review_document.path;
        let mut lock_was_held_during_launch = false;
        let mut newer_review_lock = None;

        let result = start_review_with_launcher::<ReviewTestSchema, _, _>(
            &document_path,
            |path, done_fifo_path, lock_id| {
                lock_was_held_during_launch = LockGuard::is_locked(path);
                drop(CleanupGuard::new(path, lock_id, None).expect("register child lock"));
                newer_review_lock = Some(LockGuard::acquire(path).expect("new review lock"));
                signal_fifo(done_fifo_path);
                Ok(None)
            },
            |_| Ok(true),
        )
        .expect("start review")
        .map(|document| (document.path, document.frontmatter));
        let lock_after_start_review = LockGuard::is_locked(&document_path);
        drop(newer_review_lock);

        assert_eq!(
            (
                result,
                lock_was_held_during_launch,
                lock_after_start_review,
                LockGuard::is_locked(&document_path),
            ),
            (Some((document_path, ReviewTestSchema)), true, true, false,),
        );
    }

    #[rstest]
    fn start_review_skips_without_removing_an_existing_lock(
        review_document: ReviewDocumentFixture,
    ) {
        let document_path = review_document.path;
        let lock_path = LockGuard::lock_path(&document_path);
        std::fs::write(&lock_path, "another review").expect("write existing lock");
        let mut launch_called = false;

        let result = start_review_with_launcher::<ReviewTestSchema, _, _>(
            &document_path,
            |_, _, _| {
                launch_called = true;
                Ok(None)
            },
            |_| Ok(true),
        )
        .expect("skip existing review")
        .map(|document| document.path);
        let lock_contents = std::fs::read_to_string(lock_path).expect("read existing lock");

        assert_eq!(
            (result, launch_called, lock_contents),
            (None, false, "another review".into())
        );
    }

    #[rstest]
    fn editor_already_open_message_names_the_lock_file(review_document: ReviewDocumentFixture) {
        let lock_path = LockGuard::lock_path(&review_document.path);

        assert_eq!(
            editor_already_open_message(&lock_path),
            format!(
                "Skipped: Editor is already open for this file (lock: {}).",
                lock_path.display()
            ),
        );
    }

    #[rstest]
    fn start_review_cleans_lock_and_review_fifos_when_review_session_disappears(
        review_document: ReviewDocumentFixture,
    ) {
        let document_path = review_document.path;
        let mut done_fifo_path = None;
        let mut finish_fifo_path = None;

        let result = start_review_with_marker::<ReviewTestSchema, _, _>(
            &document_path,
            None,
            |_, done_path, finish_path, _lock_id| {
                done_fifo_path = Some(done_path.to_path_buf());
                finish_fifo_path = Some(finish_path.to_path_buf());
                Ok(ReviewLaunch::without_queue(Some(
                    "review-session".to_string(),
                )))
            },
            |_| Ok(false),
        )
        .map(|document| document.is_some())
        .map_err(|error| error.to_string());
        let done_fifo_path = done_fifo_path.expect("launcher receives done FIFO path");
        let finish_fifo_path = finish_fifo_path.expect("launcher receives finish FIFO path");

        assert_eq!(
            (
                result,
                LockGuard::is_locked(&document_path),
                done_fifo_path.exists(),
                finish_fifo_path.exists(),
                exit_code::REVIEW_SESSION_CLOSED,
            ),
            (
                Err("Review editor session ended before the review completed".into()),
                false,
                false,
                false,
                4,
            ),
        );
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
    fn write_edit_diff_emits_hunk_when_changed() {
        let mut buf = Vec::new();
        write_edit_diff(&mut buf, "old\n", "new\n", false).expect("write");
        assert_eq!(
            String::from_utf8(buf).expect("utf8"),
            indoc::indoc! {"
                @@ -1 +1 @@
                -old
                +new
            "}
        );
    }
}
