use std::path::Path;

use crate::shared::config::EditorConfig;

use super::{
    FifoCleanupGuard, HumanInTheLoopError, LaunchOptions, Result, ReviewHandler,
    TERMINAL_STARTUP_TIMEOUT, create_fifo_with_suffix, open_fifo_reader,
    wait_for_fifo_signal_with_timeout,
};

pub(super) fn launch_review<S, H>(
    tmux_pane_id: Option<&str>,
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
    let mut review_args = handler.build_complete_args(document_path, tmux_pane_id, window_title);
    review_args.push("--done-fifo".into());
    review_args.push(done_fifo_path.as_os_str().to_os_string());

    if let Some(parent_pane_id) = tmux_pane_id {
        let session_id = crate::shared::env_var::EnvVars::load().own_session_id();
        launch_tmux_review(
            || {
                super::tmux::open_review_pane(
                    parent_pane_id,
                    window_title,
                    exe_path.as_os_str(),
                    &review_args,
                    done_fifo_path,
                )
            },
            session_id.as_deref(),
            |session_id| {
                super::notification::send_review_requested(
                    session_id,
                    document_path,
                    window_title,
                    editor_config,
                )
            },
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

fn launch_tmux_review(
    open_pane: impl FnOnce() -> Result<()>,
    session_id: Option<&str>,
    notify: impl FnOnce(&str),
) -> Result<()> {
    open_pane()?;
    if let Some(session_id) = session_id {
        notify(session_id);
    }
    Ok(())
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

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use rstest::rstest;

    use super::*;

    #[rstest]
    #[case::agent_review_notifies_after_open(
        true,
        Some("session-id"),
        vec!["pane opened".to_string(), "notified: session-id".to_string()],
        Ok(()),
    )]
    #[case::without_agent_session_only_opens_pane(
        true,
        None,
        vec!["pane opened".to_string()],
        Ok(()),
    )]
    #[case::failed_open_does_not_notify(
        false,
        Some("session-id"),
        vec!["pane opened".to_string()],
        Err("Command failed: pane unavailable".to_string()),
    )]
    fn notifies_only_after_successful_pane_open(
        #[case] open_succeeds: bool,
        #[case] session_id: Option<&str>,
        #[case] expected_events: Vec<String>,
        #[case] expected_result: std::result::Result<(), String>,
    ) {
        let events = RefCell::new(Vec::new());
        let result = launch_tmux_review(
            || {
                events.borrow_mut().push("pane opened".to_string());
                if open_succeeds {
                    Ok(())
                } else {
                    Err(HumanInTheLoopError::CommandFailed(
                        "pane unavailable".to_string(),
                    ))
                }
            },
            session_id,
            |session_id| {
                events.borrow_mut().push(format!("notified: {session_id}"));
            },
        )
        .map_err(|error| error.to_string());

        assert_eq!(
            (events.into_inner(), result),
            (expected_events, expected_result),
        );
    }
}
