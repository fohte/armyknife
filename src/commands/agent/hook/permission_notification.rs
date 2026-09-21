use std::fs;
use std::path::{Path, PathBuf};
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

    /// ID identifying the permission request that spawned this worker.
    #[arg(long)]
    pub request_id: String,

    /// Already-formatted permission message, including tool details.
    #[arg(long, allow_hyphen_values = true)]
    pub message: String,
}

/// Starts a detached worker so the synchronous hook is not held open
/// while waiting for a possible follow-up event.
pub(super) fn spawn(session: &Session, agent_key: &str, message: &str) {
    let request_id = session
        .pending_permission_request_ids
        .get(agent_key)
        .cloned()
        .unwrap_or_default();
    let args = [
        "agent",
        "permission-notification",
        "--session",
        session.session_id.as_str(),
        "--agent-key",
        agent_key,
        "--request-id",
        request_id.as_str(),
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
    let Some(session) = load_current_session(&sessions_dir, args, "session_missing", "resolved")?
    else {
        return Ok(());
    };

    let session = if should_defer_for_codex_auto_review(&session) {
        tracing::info!(
            event = "agent.permission_notification.deferred",
            session = %args.session,
            reason = "codex_auto_review",
            delay_secs = CODEX_AUTO_REVIEW_RECHECK_GRACE.as_secs(),
        );
        thread::sleep(CODEX_AUTO_REVIEW_RECHECK_GRACE);

        let Some(rechecked_session) = load_current_session(
            &sessions_dir,
            args,
            "session_missing_after_codex_review_grace",
            "resolved_after_codex_review_grace",
        )?
        else {
            return Ok(());
        };
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

fn load_current_session(
    sessions_dir: &Path,
    args: &DelayedPermissionNotificationArgs,
    missing_reason: &'static str,
    resolved_reason: &'static str,
) -> Result<Option<Session>> {
    let Some(session) = store::load_session_from(sessions_dir, &args.session)? else {
        tracing::info!(
            event = "agent.permission_notification.exit",
            session = %args.session,
            reason = missing_reason,
        );
        return Ok(None);
    };
    if !is_current_request(&session, &args.agent_key, &args.request_id) {
        tracing::info!(
            event = "agent.permission_notification.exit",
            session = %args.session,
            reason = resolved_reason,
        );
        return Ok(None);
    }
    Ok(Some(session))
}

fn is_current_request(session: &Session, agent_key: &str, request_id: &str) -> bool {
    is_current(session, agent_key)
        && session
            .pending_permission_request_ids
            .get(agent_key)
            .is_some_and(|current_id| current_id == request_id)
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
    let content = match fs::read_to_string(&config_path) {
        Ok(content) => content,
        Err(error) => {
            tracing::debug!(
                event = "agent.permission_notification.codex_config_unreadable",
                path = %config_path.display(),
                error = %error,
            );
            return false;
        }
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
    content
        .parse::<toml_edit::DocumentMut>()
        .ok()
        .is_some_and(|document| {
            document
                .get("approvals_reviewer")
                .and_then(|item| item.as_str())
                == Some("auto_review")
        })
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

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
            pending_permission_request_ids: BTreeMap::from([(
                MAIN_THREAD_AGENT_KEY.to_string(),
                "request-id".to_string(),
            )]),
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
    #[case::quoted_key("\"approvals_reviewer\" = \"auto_review\"", true)]
    #[case::nested_auto_review(
        indoc::indoc!("\
            [profiles.default]
            approvals_reviewer = \"auto_review\"
        "),
        false
    )]
    #[case::multiline_array_before_setting(
        indoc::indoc!("\
            values = [
                [\"first\"],
            ]
            approvals_reviewer = \"auto_review\"
        "),
        true
    )]
    #[case::commented_auto_review("# approvals_reviewer = \"auto_review\"", false)]
    #[case::quoted_comment("approvals_reviewer = \"auto_review#user\" # comment", false)]
    #[case::lone_quote("approvals_reviewer = \"", false)]
    fn reads_only_top_level_auto_review_setting(#[case] content: &str, #[case] expected: bool) {
        assert_eq!(approvals_reviewer_is_auto_review(content), expected);
    }

    #[rstest]
    #[case::same_request(true)]
    #[case::stale_request(false)]
    fn only_the_original_permission_request_is_current(
        mut session: Session,
        #[case] same_request: bool,
    ) {
        let request_id = session
            .pending_permission_request_ids
            .get(MAIN_THREAD_AGENT_KEY)
            .cloned()
            .expect("fixture has a request ID");
        if !same_request {
            session.pending_permission_request_ids.insert(
                MAIN_THREAD_AGENT_KEY.to_string(),
                "stale-request-id".to_string(),
            );
        }

        assert_eq!(
            is_current_request(&session, MAIN_THREAD_AGENT_KEY, &request_id),
            same_request,
        );
    }
}
