use std::path::Path;

use crate::shared::config::EditorConfig;

use super::{
    FifoCleanupGuard, HumanInTheLoopError, LaunchOptions, Result, ReviewHandler,
    TERMINAL_STARTUP_TIMEOUT, create_fifo_with_suffix, open_fifo_reader,
    wait_for_fifo_signal_with_timeout,
};

pub(super) fn launch_review<S, H>(
    tmux_pane_id: Option<&str>,
    tmux_target: Option<&str>,
    document_path: &Path,
    done_fifo_path: &Path,
    window_title: &str,
    handler: &H,
    editor_config: &EditorConfig,
) -> Result<()>
where
    S: super::DocumentSchema,
    H: ReviewHandler<S>,
{
    let exe_path = std::env::current_exe()?;
    let mut review_args = handler.build_complete_args(document_path, tmux_target, window_title);
    review_args.push("--done-fifo".into());
    review_args.push(done_fifo_path.as_os_str().to_os_string());

    if let Some(parent_pane_id) = tmux_pane_id {
        super::tmux::open_review_pane(
            parent_pane_id,
            window_title,
            exe_path.as_os_str(),
            &review_args,
            done_fifo_path,
        )?;
        return Ok(());
    }

    launch_in_terminal(
        &exe_path,
        &review_args,
        document_path,
        window_title,
        editor_config,
    )
}

fn launch_in_terminal(
    exe_path: &Path,
    review_args: &[std::ffi::OsString],
    document_path: &Path,
    window_title: &str,
    editor_config: &EditorConfig,
) -> Result<()> {
    // The wrapper shell signals startup after the terminal is ready to run the
    // review command, so a failed GUI launch does not leave the parent waiting.
    let started_fifo_path = create_fifo_with_suffix(document_path, ".started")?;
    let mut started_fifo_cleanup = FifoCleanupGuard::new(&started_fifo_path);
    let started_fifo_reader = open_fifo_reader(&started_fifo_path)?;

    let options = LaunchOptions {
        window_title: window_title.to_string(),
        ..Default::default()
    };
    let outcome = super::launch_terminal(
        &editor_config.terminal,
        &options,
        exe_path,
        review_args,
        Some(&started_fifo_path),
    )?;

    if !outcome.status.success() {
        return Err(HumanInTheLoopError::CommandFailed(format!(
            "Terminal exited with status: {}",
            outcome.status
        )));
    }

    if outcome.signals_started {
        wait_for_fifo_signal_with_timeout(
            started_fifo_reader,
            &started_fifo_path,
            TERMINAL_STARTUP_TIMEOUT,
        )?;
    } else {
        drop(started_fifo_reader);
    }

    started_fifo_cleanup.disarm();
    let _ = std::fs::remove_file(&started_fifo_path);
    Ok(())
}
