use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::commands::agent::{display_label, repo_name, store};
use crate::infra::notification::{Notification, NotificationAction};
use crate::shared::config::EditorConfig;

const REVIEW_NOTIFICATION_TITLE: &str = "■ HITL review requested";
const NEOVIM_LOGO_URL: &str = "https://neovim.io/logos/neovim-mark-flat.png";

pub(super) fn send_review_requested(
    session_id: &str,
    document_path: &Path,
    window_title: &str,
    editor_config: &EditorConfig,
) {
    let session = match store::load_session(session_id) {
        Ok(session) => session,
        Err(error) => {
            tracing::warn!(
                event = "hitl.review_notification.session_load_failed",
                session = %session_id,
                error = %error,
            );
            None
        }
    };
    let cwd = session
        .as_ref()
        .map(|session| session.cwd.clone())
        .or_else(|| std::env::current_dir().ok())
        .or_else(|| document_path.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."));
    let repo_name = repo_name(&cwd);
    let display_label = display_label(session.as_ref(), session_id);
    let subtitle = session
        .as_ref()
        .and_then(|session| session.tmux_info.as_ref())
        .map(|info| {
            format!(
                "{}:{} | {display_label}",
                info.session_name, info.window_name
            )
        })
        .unwrap_or_else(|| format!("Agent session {display_label}"));

    let notification = match build_review_notification(
        window_title,
        session_id,
        &repo_name,
        &subtitle,
        editor_config,
    ) {
        Ok(notification) => notification,
        Err(error) => {
            tracing::warn!(
                event = "hitl.review_notification.build_failed",
                session = %session_id,
                error = %error,
            );
            eprintln!("[armyknife] warning: failed to build HITL review notification: {error:#}");
            return;
        }
    };

    send_best_effort(&notification, crate::infra::notification::send);
}

fn build_review_notification(
    window_title: &str,
    session_id: &str,
    repo_name: &str,
    subtitle: &str,
    editor_config: &EditorConfig,
) -> Result<Notification> {
    let focus_command = shlex::try_join(["a", "agent", "focus", session_id])
        .context("failed to quote the HITL review notification command")?;
    let action = format!("{focus_command}; {}", editor_config.focus_app_command());

    Ok(Notification::new(
        REVIEW_NOTIFICATION_TITLE,
        format!("{window_title} ({repo_name})"),
    )
    .with_subtitle(subtitle)
    .with_action(NotificationAction::new(action))
    .with_content_image_url(NEOVIM_LOGO_URL))
}

fn send_best_effort(notification: &Notification, send: impl FnOnce(&Notification) -> Result<()>) {
    if let Err(error) = send(notification) {
        tracing::warn!(
            event = "hitl.review_notification.failed",
            error = %error,
        );
        eprintln!("[armyknife] warning: failed to send HITL review notification: {error:#}");
    }
}

#[cfg(test)]
mod tests {
    use anyhow::anyhow;

    use super::*;

    #[test]
    fn builds_notification_with_review_repo_session_and_focus_action() {
        let notification = build_review_notification(
            "PR: sample/repo @ sample-branch",
            "session-id",
            "sample-repo",
            "work:agent | review-session",
            &EditorConfig::default(),
        )
        .expect("notification should build");

        assert_eq!(
            (
                notification.title(),
                notification.message(),
                notification.subtitle(),
                notification.action().map(NotificationAction::command),
                notification.content_image_url(),
                notification.app_icon(),
            ),
            (
                REVIEW_NOTIFICATION_TITLE,
                "PR: sample/repo @ sample-branch (sample-repo)",
                Some("work:agent | review-session"),
                Some("a agent focus session-id; open -a WezTerm"),
                Some(NEOVIM_LOGO_URL),
                None,
            ),
        );
    }

    #[test]
    fn notification_send_failure_is_ignored() {
        let notification = Notification::new("Review", "sample repo");
        let mut send_called = false;
        assert_eq!(
            (
                send_best_effort(&notification, |_| {
                    send_called = true;
                    Err(anyhow!("notification service unavailable"))
                }),
                send_called,
            ),
            ((), true),
        );
    }
}
