use std::path::Path;

use crate::shared::config::EditorConfig;
use crate::shared::env_var::EnvVars;
use crate::shared::notification_policy;

use super::{
    FifoCleanupGuard, HumanInTheLoopError, LaunchOptions, Result, ReviewHandler,
    TERMINAL_STARTUP_TIMEOUT, create_fifo_with_suffix, open_fifo_reader,
    wait_for_fifo_signal_with_timeout,
};

pub(super) struct ReviewContext<'a> {
    pub(super) lock_id: &'a str,
    pub(super) done_fifo_path: &'a Path,
    pub(super) finish_fifo_path: &'a Path,
    pub(super) window_title: &'a str,
    pub(super) command_name: &'a str,
}

/// Returns the shpool session running the editor when launched in tmux. The
/// caller watches it to notice the editor ending without a completion signal,
/// and holds the queue entry until the review ends.
pub(super) fn launch_review<S, H>(
    tmux_pane_id: Option<&str>,
    document_path: &Path,
    context: &ReviewContext<'_>,
    handler: &H,
    editor_config: &EditorConfig,
    notifications_enabled: bool,
) -> Result<(Option<String>, Option<super::queue_entry::QueueEntry>)>
where
    S: super::DocumentSchema,
    H: ReviewHandler<S>,
{
    let exe_path = std::env::current_exe()?;
    let review_args = build_review_args(
        handler.build_complete_args(document_path, tmux_pane_id, context.window_title),
        context,
    );

    let mut queue_entry = None;
    if let Some(parent_pane_id) = tmux_pane_id {
        let env_vars = EnvVars::load();
        let session_id = env_vars.own_session_id();
        let shpool_session = crate::infra::tmux::new_editor_session_name(parent_pane_id);
        launch_tmux_review(
            || {
                super::tmux::open_review_pane(
                    parent_pane_id,
                    context.window_title,
                    &shpool_session,
                    exe_path.as_os_str(),
                    &review_args,
                    context.done_fifo_path,
                    &mut queue_entry,
                )
            },
            session_id.as_deref(),
            notifications_enabled,
            env_vars.cc_notify.as_deref(),
            |session_id, review_pane_id| {
                super::notification::send_review_requested(
                    session_id,
                    review_pane_id,
                    document_path,
                    context.window_title,
                    context.command_name,
                    editor_config,
                )
            },
        )?;
        return Ok((Some(shpool_session), queue_entry));
    }

    launch_in_terminal(
        &exe_path,
        &review_args,
        document_path,
        context.window_title,
        editor_config,
    )?;
    Ok((None, None))
}

fn build_review_args(
    mut args: Vec<std::ffi::OsString>,
    context: &ReviewContext<'_>,
) -> Vec<std::ffi::OsString> {
    args.extend([
        "--done-fifo".into(),
        context.done_fifo_path.as_os_str().to_os_string(),
        "--finish-fifo".into(),
        context.finish_fifo_path.as_os_str().to_os_string(),
        "--review-lock-id".into(),
        context.lock_id.into(),
    ]);
    args
}

fn launch_tmux_review(
    open_pane: impl FnOnce() -> Result<String>,
    session_id: Option<&str>,
    notifications_enabled: bool,
    cc_notify: Option<&str>,
    notify: impl FnOnce(&str, &str),
) -> Result<String> {
    let review_pane_id = open_pane()?;
    if notification_policy::is_enabled(notifications_enabled, cc_notify)
        && let Some(session_id) = session_id
    {
        notify(session_id, &review_pane_id);
    }
    Ok(review_pane_id)
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

    #[test]
    fn complete_arguments_include_both_review_fifo_paths() {
        let done_fifo_path = Path::new("/tmp/example-review.md.done");
        let finish_fifo_path = Path::new("/tmp/example-review.md.finish");
        let context = ReviewContext {
            lock_id: "example-lock-id",
            done_fifo_path,
            finish_fifo_path,
            window_title: "Example review",
            command_name: "example-review",
        };

        assert_eq!(
            build_review_args(vec!["--example-option".into()], &context)
                .iter()
                .map(|argument| argument.to_string_lossy().into_owned())
                .collect::<Vec<_>>(),
            vec![
                "--example-option",
                "--done-fifo",
                "/tmp/example-review.md.done",
                "--finish-fifo",
                "/tmp/example-review.md.finish",
                "--review-lock-id",
                "example-lock-id",
            ],
        );
    }

    #[rstest]
    #[case::agent_review_notifies_after_open(
        true,
        Some("session-id"),
        true,
        None,
        vec![
            "pane opened".to_string(),
            "notified: session-id: %review".to_string(),
        ],
        Ok("%review".to_string()),
    )]
    #[case::notifications_disabled_keeps_agent_session(
        true,
        Some("session-id"),
        false,
        None,
        vec!["pane opened".to_string()],
        Ok("%review".to_string()),
    )]
    #[case::environment_override_disables_notification(
        true,
        Some("session-id"),
        true,
        Some("0"),
        vec!["pane opened".to_string()],
        Ok("%review".to_string()),
    )]
    #[case::without_agent_session_only_opens_pane(
        true,
        None,
        true,
        None,
        vec!["pane opened".to_string()],
        Ok("%review".to_string()),
    )]
    #[case::failed_open_does_not_notify(
        false,
        Some("session-id"),
        true,
        None,
        vec!["pane opened".to_string()],
        Err("Command failed: pane unavailable".to_string()),
    )]
    fn notifies_only_after_successful_pane_open(
        #[case] open_succeeds: bool,
        #[case] session_id: Option<&str>,
        #[case] notifications_enabled: bool,
        #[case] cc_notify: Option<&str>,
        #[case] expected_events: Vec<String>,
        #[case] expected_result: std::result::Result<String, String>,
    ) {
        let events = RefCell::new(Vec::new());
        let result = launch_tmux_review(
            || {
                events.borrow_mut().push("pane opened".to_string());
                if open_succeeds {
                    Ok("%review".to_string())
                } else {
                    Err(HumanInTheLoopError::CommandFailed(
                        "pane unavailable".to_string(),
                    ))
                }
            },
            session_id,
            notifications_enabled,
            cc_notify,
            |session_id, review_pane_id| {
                events
                    .borrow_mut()
                    .push(format!("notified: {session_id}: {review_pane_id}"));
            },
        )
        .map_err(|error| error.to_string());

        assert_eq!(
            (events.into_inner(), result),
            (expected_events, expected_result),
        );
    }
}
