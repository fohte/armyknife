use std::fs;
use std::path::PathBuf;
use std::thread;
use std::time::Duration;

use anyhow::Result;
use clap::Args;

use super::{build_notification_with_message, config, is_notification_enabled, store};
use crate::commands::agent::types::{Engine, Session, SessionStatus};
use crate::infra::notification;
use crate::infra::process;
use crate::shared::dirs;

#[cfg(test)]
use crate::commands::agent::types::MAIN_THREAD_AGENT_KEY;

/// The agent CLIs do not expose the aggregate PermissionRequest decision to
/// command hooks. The one-second debounce covers the normal handoff to a
/// later PostToolUse or Stop hook without holding the synchronous hook open.
const PERMISSION_NOTIFICATION_GRACE: Duration = Duration::from_secs(1);

/// Codex v0.155.1's Guardian review timeout is 90 seconds
/// (`ext/guardian-reviewer/src/lib.rs`). Wait one extra second so an optional
/// review that reaches its timeout can hand the request back to the user
/// before the notification is sent.
const CODEX_AUTO_REVIEW_RECHECK_GRACE: Duration = Duration::from_secs(91);

#[derive(Args, Clone, PartialEq, Eq)]
pub struct DelayedPermissionNotificationArgs {
    /// Agent session ID whose permission request is being checked.
    #[arg(long)]
    pub session: String,

    /// Agent key stored in the session's pending permission set.
    #[arg(long)]
    pub agent_key: String,

    /// Already-formatted permission message, including tool details.
    #[arg(long, allow_hyphen_values = true)]
    pub message: String,
}

/// Starts a detached worker so the synchronous hook is not held open
/// while waiting for a possible follow-up event.
pub(super) fn spawn(session: &Session, agent_key: &str, message: &str) {
    let args = [
        "agent",
        "permission-notification",
        "--session",
        session.session_id.as_str(),
        "--agent-key",
        agent_key,
        "--message",
        message,
    ];
    process::spawn_self_detached(
        "agent.permission_notification.spawn",
        "agent.permission_notification.spawn_failed",
        &session.session_id,
        &args,
    );
}

pub(crate) fn run(args: &DelayedPermissionNotificationArgs) -> Result<()> {
    thread::sleep(PERMISSION_NOTIFICATION_GRACE);

    let sessions_dir = store::sessions_dir()?;
    let Some(session) = store::load_session_from(&sessions_dir, &args.session)? else {
        tracing::info!(
            event = "agent.permission_notification.exit",
            session = %args.session,
            reason = "session_missing",
        );
        return Ok(());
    };
    if !is_current(&session, &args.agent_key) {
        tracing::info!(
            event = "agent.permission_notification.exit",
            session = %args.session,
            reason = "resolved",
        );
        return Ok(());
    }

    let session = if should_defer_for_codex_auto_review(&session) {
        thread::sleep(CODEX_AUTO_REVIEW_RECHECK_GRACE);

        let Some(rechecked_session) = store::load_session_from(&sessions_dir, &args.session)?
        else {
            tracing::info!(
                event = "agent.permission_notification.exit",
                session = %args.session,
                reason = "session_missing_after_codex_review_grace",
            );
            return Ok(());
        };
        if !is_current(&rechecked_session, &args.agent_key) {
            tracing::info!(
                event = "agent.permission_notification.exit",
                session = %args.session,
                reason = "resolved_after_codex_review_grace",
            );
            return Ok(());
        }
        rechecked_session
    } else {
        session
    };

    let config = config::load_config().unwrap_or_default();
    if !is_notification_enabled(&config) {
        tracing::info!(
            event = "agent.permission_notification.exit",
            session = %args.session,
            reason = "disabled",
        );
        return Ok(());
    }

    let notification = build_notification_with_message(args.message.clone(), &session, &config);
    match notification::send(&notification) {
        Ok(()) => tracing::info!(
            event = "agent.permission_notification.sent",
            session = %args.session,
        ),
        Err(error) => tracing::warn!(
            event = "agent.permission_notification.send_failed",
            session = %args.session,
            error = %error,
        ),
    }
    Ok(())
}

pub(super) fn is_current(session: &Session, agent_key: &str) -> bool {
    session.status == SessionStatus::WaitingInput
        && session.pending_permission_agent_ids.contains(agent_key)
}

fn should_defer_for_codex_auto_review(session: &Session) -> bool {
    // The current app-server thread/resume APIs join a running thread. If a
    // side-effect-free subscription exposes item/.../requestApproval later,
    // use that protocol signal instead of this configuration-based timing.
    session.engine == Engine::Codex && codex_uses_auto_review()
}

fn codex_uses_auto_review() -> bool {
    // Launch-only `-c` overrides and Apps connector reviewer overrides are
    // resolved inside Codex, so they are not visible here. An unreadable
    // setting stays on the existing one-second path.
    let Some(config_path) = codex_config_path() else {
        return false;
    };
    let Ok(content) = fs::read_to_string(config_path) else {
        return false;
    };
    approvals_reviewer_is_auto_review(&content)
}

fn codex_config_path() -> Option<PathBuf> {
    std::env::var("CODEX_HOME")
        .ok()
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|home| home.join(".codex")))
        .map(|codex_home| codex_home.join("config.toml"))
}

fn approvals_reviewer_is_auto_review(content: &str) -> bool {
    let mut in_root_table = true;
    for line in content.lines() {
        let line = strip_toml_comment(line).trim();
        if line.starts_with('[') {
            in_root_table = false;
            continue;
        }
        if !in_root_table {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key.trim() == "approvals_reviewer" {
            return toml_string_value(value.trim()) == Some("auto_review");
        }
    }
    false
}

fn strip_toml_comment(line: &str) -> &str {
    let mut quote = None;
    let mut escaped = false;
    for (index, character) in line.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match (quote, character) {
            (Some('"'), '\\') => escaped = true,
            (Some(current), character) if current == character => quote = None,
            (None, '"' | '\'') => quote = Some(character),
            (None, '#') => return &line[..index],
            _ => {}
        }
    }
    line
}

fn toml_string_value(value: &str) -> Option<&str> {
    let quote = value.as_bytes().first().copied()?;
    if !matches!(quote, b'"' | b'\'') || value.as_bytes().last().copied()? != quote {
        return None;
    }
    Some(&value[1..value.len() - 1])
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use chrono::Utc;
    use rstest::{fixture, rstest};

    use super::*;

    #[fixture]
    fn session() -> Session {
        let now = Utc::now();
        Session {
            session_id: "session-placeholder".to_string(),
            cwd: "/tmp/session-placeholder".into(),
            transcript_path: None,
            tty: None,
            tmux_info: None,
            status: SessionStatus::WaitingInput,
            created_at: now,
            updated_at: now,
            last_message: None,
            current_tool: None,
            label: None,
            ancestor_session_ids: Vec::new(),
            pending_bg_task_ids: BTreeSet::new(),
            pending_agent_task_ids: BTreeSet::new(),
            pending_permission_agent_ids: BTreeSet::from([MAIN_THREAD_AGENT_KEY.to_string()]),
            read_at: None,
            sweep_signaled: false,
            engine: Engine::Codex,
        }
    }

    #[rstest]
    #[case::codex_current_request(
        Engine::Codex,
        SessionStatus::WaitingInput,
        Some(MAIN_THREAD_AGENT_KEY),
        true
    )]
    #[case::claude_current_request(
        Engine::Claude,
        SessionStatus::WaitingInput,
        Some(MAIN_THREAD_AGENT_KEY),
        true
    )]
    #[case::post_tool_use_cleared(
        Engine::Codex,
        SessionStatus::Running,
        Some(MAIN_THREAD_AGENT_KEY),
        false
    )]
    #[case::stop_cleared(
        Engine::Codex,
        SessionStatus::Stopped,
        Some(MAIN_THREAD_AGENT_KEY),
        false
    )]
    #[case::no_pending_permission(Engine::Codex, SessionStatus::WaitingInput, None, false)]
    #[case::sibling_permission_only(
        Engine::Codex,
        SessionStatus::WaitingInput,
        Some("sibling-agent"),
        false
    )]
    fn only_current_permission_wait_is_notified(
        mut session: Session,
        #[case] engine: Engine,
        #[case] status: SessionStatus,
        #[case] pending_agent: Option<&str>,
        #[case] expected: bool,
    ) {
        session.engine = engine;
        session.status = status;
        session.pending_permission_agent_ids = pending_agent
            .map(|agent| BTreeSet::from([agent.to_string()]))
            .unwrap_or_default();

        assert_eq!(is_current(&session, MAIN_THREAD_AGENT_KEY), expected);
    }

    #[rstest]
    #[case::top_level_auto_review("approvals_reviewer = \"auto_review\"", true)]
    #[case::top_level_user("approvals_reviewer = \"user\"", false)]
    #[case::nested_auto_review(
        indoc::indoc!("\
            [profiles.default]
            approvals_reviewer = \"auto_review\"
        "),
        false
    )]
    #[case::commented_auto_review("# approvals_reviewer = \"auto_review\"", false)]
    #[case::quoted_comment("approvals_reviewer = \"auto_review#user\" # comment", false)]
    fn reads_only_top_level_auto_review_setting(#[case] content: &str, #[case] expected: bool) {
        assert_eq!(approvals_reviewer_is_auto_review(content), expected);
    }
}
