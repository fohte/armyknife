//! `a agent peer notify` -- send a message to another session without going
//! through any session's own `SendMessage` tool call.
//!
//! `a agent peer` and `a agent peer wake` both assume a Claude Code session is
//! driving them (the session resolves a name, then calls its own
//! `SendMessage` tool). Some notifications have no session in the loop --
//! e.g. reporting that a delegated PR merged from inside `a agent close`
//! itself. This command is that missing send path: a Claude Code target gets
//! `claude_messaging`'s direct socket write, while a Codex target is steered
//! through its app-server and falls back to `codex queue` when unavailable.

use anyhow::Result;
use clap::Args;

use super::wake;
use crate::commands::agent::claude_messaging;
use crate::commands::agent::claude_registry;
use crate::commands::agent::codex_queue;
use crate::commands::agent::codex_steer;
use crate::commands::agent::error::CcError;
use crate::commands::agent::store;
use crate::commands::agent::types::{Engine, Session, SessionStatus};
use crate::shared::env_var::EnvVars;
use crate::shared::sanitize::strip_angle_brackets;

const MAX_SESSION_ID_SUGGESTIONS: usize = 3;
const MAX_SESSION_ID_EDIT_DISTANCE: usize = 2;
const MIN_SESSION_ID_PREFIX_LENGTH: usize = 8;

#[derive(Args, Clone, PartialEq, Eq)]
pub struct NotifyArgs {
    /// Session ID to notify -- the `session_id` from `a agent peer`
    #[arg(required_unless_present = "parent", conflicts_with = "parent")]
    pub session_id: Option<String>,

    /// Notify the session that delegated to this one
    #[arg(long)]
    pub parent: bool,

    /// Message text to deliver as a user message to the target session
    #[arg(short = 'm', long = "message")]
    pub message: String,
}

/// The delivery path used by [`notify`]. Codex reports `Steered` versus
/// `Started` from armyknife's tracked pre-delivery status because app-server
/// returns the same response for both outcomes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Delivery {
    /// Written to the target's messaging socket.
    Sent,
    /// Delivered through app-server while armyknife tracked an active turn.
    Steered,
    /// Delivered through app-server while armyknife tracked an idle thread.
    Started,
    /// Appended to the target Codex session's queue. The running `codex`
    /// injects it later (polls every ~10s, and only once the session is
    /// idle), so it may not have reached the session yet.
    Queued { reason: String },
}

pub fn run(args: &NotifyArgs) -> Result<()> {
    run_with(args, super::resolve_parent_session, notify)
}

fn run_with(
    args: &NotifyArgs,
    resolve_parent: impl FnOnce() -> Result<Session>,
    deliver: impl FnOnce(&str, &str, Option<&str>, Option<Engine>) -> Result<Delivery>,
) -> Result<()> {
    let session_id =
        resolve_target_session_id(args, || resolve_parent().map(|session| session.session_id))?;
    let (from, from_engine_hint) = match resolve_sender() {
        Some((id, engine_hint)) => (Some(id), engine_hint),
        None => (None, None),
    };
    let delivery = deliver(
        &session_id,
        &args.message,
        from.as_deref(),
        from_engine_hint,
    )?;
    if let Some(output) = delivery_output(&session_id, &delivery) {
        println!("{output}");
    }
    Ok(())
}

fn resolve_target_session_id(
    args: &NotifyArgs,
    resolve_parent: impl FnOnce() -> Result<String>,
) -> Result<String> {
    match (&args.session_id, args.parent) {
        (Some(session_id), false) => Ok(session_id.clone()),
        (None, true) => resolve_parent(),
        _ => anyhow::bail!("Exactly one of <session_id> or --parent must be specified"),
    }
}

fn delivery_output(session_id: &str, delivery: &Delivery) -> Option<String> {
    match delivery {
        Delivery::Sent => None,
        Delivery::Steered => Some(format!(
            "Injected into the running turn of Codex session {session_id}."
        )),
        Delivery::Started => Some(format!(
            "Started a new turn in idle Codex session {session_id}."
        )),
        Delivery::Queued { reason } => Some(format!(
            "Queued for Codex session {session_id}. Direct delivery failed: {reason}. Not delivered yet: the current turn must finish and the session must become idle before `codex queue` can start the queued turn (polled about every 10s)."
        )),
    }
}

/// Resolves this process's own session_id for the `<peer-message>` sender
/// line, trying `ARMYKNIFE_SESSION_ID` (set by the Claude Code `session-start`
/// hook) before each engine's own ambient session env var -- present for a
/// plain `claude`/`codex` CLI invocation, including one whose hooks aren't
/// registered.
/// Returns `None` when nothing resolves (e.g. `a agent close` has no session
/// in the loop), which sends the message unwrapped.
///
/// The paired `Engine` is only a fallback guess, used by [`notify`] when the
/// resolved ID isn't in armyknife's store: `ARMYKNIFE_SESSION_ID` carries no
/// engine hint of its own, so only the engine-specific variables pair with a
/// guess.
///
/// Only called from [`run`], not [`notify`] itself: reading these env vars
/// inside `notify` would make `merge_notify`'s direct calls pick up
/// whichever session's env happens to be ambient in the calling process,
/// misattributing an automated merge notification as a personal message.
fn resolve_sender() -> Option<(String, Option<Engine>)> {
    let env = EnvVars::load();
    if let Some(id) = env.session_id {
        return Some((id, None));
    }
    if let Some(id) = non_empty_env_var("CLAUDE_CODE_SESSION_ID") {
        return Some((id, Some(Engine::Claude)));
    }
    // Codex sets both CODEX_SESSION_ID (shared by the root thread and all of
    // its subagent threads) and CODEX_THREAD_ID (unique per subagent
    // thread). CODEX_SESSION_ID is the one that matches how armyknife scopes
    // a session: `HookInput` (types.rs) carries a single `session_id` field
    // plus a separate optional `agent_id` for subagent-fired events, so a
    // Claude Code subagent's hook events still report the top-level
    // session's session_id, not a per-subagent one. The Codex equivalent of
    // "this session" is the value that's stable across its subagents too.
    if let Some(id) = env.codex_session_id {
        return Some((id, Some(Engine::Codex)));
    }
    None
}

/// Treats an empty-but-set env var as unset, matching `EnvVars::load`'s own
/// `non_empty_var` handling of `ARMYKNIFE_*` variables one branch above.
fn non_empty_env_var(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

fn ensure_not_self_notify(session_id: &str, from: Option<&str>) -> Result<()> {
    if from == Some(session_id) {
        return Err(CcError::SelfNotify(session_id.to_string()).into());
    }
    Ok(())
}

fn session_not_found_error(session_id: &str) -> anyhow::Error {
    let error = CcError::SessionNotFound(session_id.to_string());
    let suggestions = match store::list_sessions() {
        Ok(sessions) => similar_session_ids(session_id, &sessions),
        // Suggestions are best effort; keep the original lookup error if listing fails.
        Err(_) => Vec::new(),
    };

    if suggestions.is_empty() {
        return error.into();
    }

    let suggestions = suggestions
        .iter()
        .map(|suggestion| format!("- {suggestion}"))
        .collect::<Vec<_>>()
        .join("\n");
    anyhow::anyhow!(indoc::formatdoc! {"
        {error}
        Did you mean:
        {suggestions}"
    })
}

#[derive(Debug)]
struct SessionIdSuggestion {
    session_id: String,
    edit_distance: usize,
    common_prefix_length: usize,
}

fn similar_session_ids(session_id: &str, sessions: &[Session]) -> Vec<String> {
    let mut suggestions: Vec<_> = sessions
        .iter()
        .filter(|session| session.status != SessionStatus::Ended)
        .filter(|session| session.session_id != session_id)
        .filter_map(|session| {
            let edit_distance = levenshtein_distance(session_id, &session.session_id);
            let common_prefix_length = common_prefix_length(session_id, &session.session_id);
            (edit_distance <= MAX_SESSION_ID_EDIT_DISTANCE
                || common_prefix_length >= MIN_SESSION_ID_PREFIX_LENGTH)
                .then(|| SessionIdSuggestion {
                    session_id: session.session_id.clone(),
                    edit_distance,
                    common_prefix_length,
                })
        })
        .collect();

    let best_prefix_length = suggestions
        .iter()
        .filter(|suggestion| suggestion.edit_distance > MAX_SESSION_ID_EDIT_DISTANCE)
        .map(|suggestion| suggestion.common_prefix_length)
        .max();
    let unique_best_prefix_id = best_prefix_length.and_then(|best_prefix_length| {
        let mut best_matches = suggestions.iter().filter(|suggestion| {
            suggestion.edit_distance > MAX_SESSION_ID_EDIT_DISTANCE
                && suggestion.common_prefix_length == best_prefix_length
        });
        let best_match = best_matches.next()?;
        best_matches
            .next()
            .is_none()
            .then(|| best_match.session_id.clone())
    });
    suggestions.retain(|suggestion| {
        suggestion.edit_distance <= MAX_SESSION_ID_EDIT_DISTANCE
            || unique_best_prefix_id.as_ref() == Some(&suggestion.session_id)
    });

    suggestions.sort_by(|left, right| {
        let key = |suggestion: &SessionIdSuggestion| {
            let is_edit_match = suggestion.edit_distance <= MAX_SESSION_ID_EDIT_DISTANCE;
            (
                !is_edit_match,
                if is_edit_match {
                    suggestion.edit_distance
                } else {
                    0
                },
                std::cmp::Reverse(suggestion.common_prefix_length),
                suggestion.edit_distance,
            )
        };

        key(left)
            .cmp(&key(right))
            .then_with(|| left.session_id.cmp(&right.session_id))
    });
    suggestions.truncate(MAX_SESSION_ID_SUGGESTIONS);
    suggestions
        .into_iter()
        .map(|suggestion| suggestion.session_id)
        .collect()
}

fn common_prefix_length(left: &str, right: &str) -> usize {
    left.chars()
        .zip(right.chars())
        .take_while(|(left, right)| left == right)
        .count()
}

fn levenshtein_distance(left: &str, right: &str) -> usize {
    let left: Vec<char> = left.chars().collect();
    let right: Vec<char> = right.chars().collect();
    let mut previous: Vec<usize> = (0..=right.len()).collect();
    let mut current = vec![0; right.len() + 1];

    for (left_index, left_char) in left.iter().enumerate() {
        current[0] = left_index + 1;
        for (right_index, right_char) in right.iter().enumerate() {
            let substitution_cost = if left_char == right_char { 0 } else { 1 };
            current[right_index + 1] = (previous[right_index + 1] + 1)
                .min(current[right_index] + 1)
                .min(previous[right_index] + substitution_cost);
        }
        std::mem::swap(&mut previous, &mut current);
    }

    previous[right.len()]
}

pub fn notify(
    session_id: &str,
    message: &str,
    from: Option<&str>,
    from_engine_hint: Option<Engine>,
) -> Result<Delivery> {
    ensure_not_self_notify(session_id, from)?;

    let session =
        store::load_session(session_id)?.ok_or_else(|| session_not_found_error(session_id))?;

    match notify_readiness(session.status) {
        NotifyReadiness::Refused => {
            return Err(CcError::SessionEnded(session_id.to_string()).into());
        }
        NotifyReadiness::NeedsWake => {
            wake::wake(session_id)?;
        }
        NotifyReadiness::Ready => {}
    }

    // Best-effort: `from` not resolving to a tracked session (unknown ID,
    // lookup error) just means the engine falls back to `from_engine_hint`
    // (or is omitted), not a reason to fail the whole notification.
    let from_engine = from
        .and_then(|f| store::load_session(f).ok().flatten().map(|s| s.engine))
        .or(from_engine_hint);
    let content = build_content(message, from, from_engine);

    match session.engine {
        Engine::Claude => {
            send_to_claude(session_id, &content)?;
            Ok(Delivery::Sent)
        }
        Engine::Codex => send_to_codex(
            session_id,
            &content,
            matches!(
                session.status,
                SessionStatus::Running | SessionStatus::WaitingInput
            ),
            codex_steer::send_message,
            codex_queue::queue_message,
        ),
    }
}

fn send_to_codex<S, Q>(
    session_id: &str,
    content: &str,
    was_active: bool,
    steer: S,
    queue: Q,
) -> Result<Delivery>
where
    S: FnOnce(&str, &str) -> codex_steer::Result<()>,
    Q: FnOnce(&str, &str) -> Result<()>,
{
    match steer(session_id, content) {
        Ok(()) if was_active => Ok(Delivery::Steered),
        Ok(()) => Ok(Delivery::Started),
        Err(codex_steer::DeliveryError::NotDelivered(direct_error)) => {
            queue(session_id, content).map_err(|queue_error| {
                anyhow::anyhow!(
                    "direct Codex delivery failed: {direct_error}; queue fallback failed: {queue_error}"
                )
            })?;
            Ok(Delivery::Queued {
                reason: direct_error.to_string(),
            })
        }
        Err(error @ codex_steer::DeliveryError::Unconfirmed(_)) => Err(error.into()),
    }
}

fn send_to_claude(session_id: &str, content: &str) -> Result<()> {
    let connection = claude_registry::load_peer_connection(session_id).ok_or_else(|| {
        anyhow::anyhow!("no active Claude Code session registry entry for session {session_id}")
    })?;
    let socket_path = connection
        .messaging_socket_path
        .ok_or_else(|| CcError::NoMessagingSocket(session_id.to_string()))?;
    claude_messaging::send_message(&socket_path, connection.pid, content)
}

/// Wraps `message` in a `<peer-message>` envelope naming `from` (and its
/// engine, when resolved) when given, otherwise passes `message` through
/// unchanged (existing callers, e.g. `merge_notify`, already wrap their own
/// message in their own envelope). `from` goes through `strip_angle_brackets`
/// since it's caller-supplied and could otherwise close the envelope early.
fn build_content(message: &str, from: Option<&str>, from_engine: Option<Engine>) -> String {
    match from {
        None => message.to_string(),
        Some(from) => {
            let from = strip_angle_brackets(from);
            let engine_line = from_engine
                .map(|e| format!("\n- Sender engine: {}", e.process_name()))
                .unwrap_or_default();
            indoc::formatdoc! {"
                <peer-message>
                - From session_id: {from}{engine_line}

                {message}
                </peer-message>"}
        }
    }
}

/// Whether `notify` can send immediately, must resume the session first, or
/// must refuse outright, based on armyknife's own tracked session status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NotifyReadiness {
    Ready,
    NeedsWake,
    /// `Ended` means the user intentionally terminated the session --
    /// armyknife must not resurrect it just to deliver a notification.
    Refused,
}

fn notify_readiness(status: SessionStatus) -> NotifyReadiness {
    match status {
        SessionStatus::Ended => NotifyReadiness::Refused,
        SessionStatus::Paused => NotifyReadiness::NeedsWake,
        SessionStatus::Running
        | SessionStatus::WaitingInput
        | SessionStatus::Stopped
        | SessionStatus::Failed => NotifyReadiness::Ready,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::agent::types::TmuxInfo;
    use chrono::Utc;
    use rstest::{fixture, rstest};
    use std::path::PathBuf;

    struct TestSessionStore {
        _temp_dir: tempfile::TempDir,
        cache_home: String,
        sessions_dir: PathBuf,
    }

    #[fixture]
    fn session_store() -> TestSessionStore {
        let temp_dir = tempfile::tempdir().expect("temporary session store should be created");
        let cache_home = temp_dir.path().to_string_lossy().into_owned();
        let sessions_dir = temp_dir.path().join("armyknife/cc/sessions");
        TestSessionStore {
            _temp_dir: temp_dir,
            cache_home,
            sessions_dir,
        }
    }

    fn suggestion_session(session_id: &str, status: SessionStatus, engine: Engine) -> Session {
        Session {
            pr_context: None,
            session_id: session_id.to_string(),
            work_type: None,
            work_type_pinned: false,
            vibe_enabled: false,
            launched_with_vibe: false,
            is_delegated: false,
            crit_urls: Vec::new(),
            pending_human_review_ids: Default::default(),
            cwd: PathBuf::from("/tmp/test-session"),
            transcript_path: None,
            tty: None,
            tmux_info: None::<TmuxInfo>,
            status,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            last_message: None,
            current_tool: None,
            label: None,
            ancestor_session_ids: Vec::new(),
            pending_bg_task_ids: Default::default(),
            pending_agent_task_ids: Default::default(),
            pending_permission_agent_ids: Default::default(),
            pending_permission_request_ids: Default::default(),
            read_at: None,
            sweep_signaled: false,
            stop_blocked_by_armyknife: false,
            agent_status: None,
            agent_status_clear_pending: false,
            archived_at: None,
            engine,
        }
    }

    fn missing_session_error(
        session_store: &TestSessionStore,
        session_id: &str,
        sessions: &[(&str, SessionStatus, Engine)],
    ) -> String {
        for (id, status, engine) in sessions {
            store::save_session_to(
                &session_store.sessions_dir,
                &suggestion_session(id, *status, *engine),
            )
            .expect("test session should be stored");
        }

        temp_env::with_vars(
            [("XDG_CACHE_HOME", Some(session_store.cache_home.as_str()))],
            || {
                notify(session_id, "hello", None, None)
                    .expect_err("untracked session should be rejected")
                    .to_string()
            },
        )
    }

    fn session(session_id: &str, cwd: &str, ancestor_session_ids: Vec<String>) -> Session {
        Session {
            archived_at: None,
            pr_context: None,
            session_id: session_id.to_string(),
            work_type: None,
            work_type_pinned: false,
            vibe_enabled: false,
            launched_with_vibe: false,
            is_delegated: false,
            crit_urls: Vec::new(),
            pending_human_review_ids: Default::default(),
            cwd: PathBuf::from(cwd),
            transcript_path: None,
            tty: None,
            tmux_info: None::<TmuxInfo>,
            status: SessionStatus::Running,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            last_message: None,
            current_tool: None,
            label: None,
            ancestor_session_ids,
            pending_bg_task_ids: Default::default(),
            pending_agent_task_ids: Default::default(),
            pending_permission_agent_ids: Default::default(),
            pending_permission_request_ids: Default::default(),
            read_at: None,
            sweep_signaled: false,
            stop_blocked_by_armyknife: false,
            agent_status: None,
            agent_status_clear_pending: false,
            engine: Engine::Claude,
        }
    }

    #[rstest]
    #[case::same_session(
        "session-a",
        Some("session-a"),
        Err("Cannot notify your own session 'session-a'".to_string())
    )]
    #[case::different_session("session-a", Some("session-b"), Ok(()))]
    #[case::sender_unknown("session-a", None, Ok(()))]
    fn self_notification_guard_cases(
        #[case] session_id: &str,
        #[case] from: Option<&str>,
        #[case] expected: std::result::Result<(), String>,
    ) {
        assert_eq!(
            ensure_not_self_notify(session_id, from).map_err(|error| error.to_string()),
            expected,
        );
    }

    #[test]
    fn run_rejects_notification_to_resolved_sender() {
        let result = temp_env::with_vars(
            [
                ("ARMYKNIFE_SESSION_ID", Some("session-a")),
                ("CLAUDE_CODE_SESSION_ID", None::<&str>),
                ("CODEX_SESSION_ID", None::<&str>),
            ],
            || {
                run(&NotifyArgs {
                    session_id: Some("session-a".to_string()),
                    parent: false,
                    message: "hello".to_string(),
                })
            },
        );

        assert_eq!(
            result.map_err(|error| error.to_string()),
            Err("Cannot notify your own session 'session-a'".to_string()),
        );
    }

    #[test]
    fn parent_target_is_delivered_to_the_notifier() {
        use clap::Parser;

        let cache_dir = tempfile::tempdir().expect("cache directory should be created");
        let cache_home = cache_dir.path().to_string_lossy().to_string();
        let sessions_dir = cache_dir.path().join("armyknife/cc/sessions");
        let child = session(
            "child-session",
            cache_dir
                .path()
                .to_str()
                .expect("temporary path should be UTF-8"),
            vec!["root-session".to_string(), "parent-session".to_string()],
        );
        let parent = session(
            "parent-session",
            cache_dir
                .path()
                .to_str()
                .expect("temporary path should be UTF-8"),
            vec![],
        );
        store::save_session_to(&sessions_dir, &child).expect("child session should be saved");
        store::save_session_to(&sessions_dir, &parent).expect("parent session should be saved");

        let cli = crate::cli::Cli::try_parse_from([
            "a", "agent", "peer", "notify", "--parent", "-m", "hello",
        ])
        .expect("parent notification arguments should parse");
        let crate::cli::Commands::Agent(crate::commands::agent::AgentCommands::Peer(
            super::super::PeerCommands::Notify(args),
        )) = cli.command
        else {
            panic!("CLI should parse a peer notify command");
        };
        let mut deliveries = Vec::new();

        let result = temp_env::with_vars(
            [
                ("XDG_CACHE_HOME", Some(cache_home.as_str())),
                ("ARMYKNIFE_SESSION_ID", Some("child-session")),
                ("CODEX_SESSION_ID", None::<&str>),
                ("CLAUDE_CODE_SESSION_ID", None::<&str>),
            ],
            || {
                run_with(
                    &args,
                    super::super::resolve_parent_session,
                    |target, message, from, engine| {
                        deliveries.push((
                            target.to_string(),
                            message.to_string(),
                            from.map(str::to_string),
                            engine,
                        ));
                        Ok(Delivery::Sent)
                    },
                )
            },
        );

        assert_eq!(
            (result.map_err(|error| error.to_string()), deliveries),
            (
                Ok(()),
                vec![(
                    "parent-session".to_string(),
                    "hello".to_string(),
                    Some("child-session".to_string()),
                    None,
                )],
            ),
        );
    }

    #[rstest]
    #[case::explicit_session_id(
        vec!["a", "agent", "peer", "notify", "parent-session", "-m", "hello"],
        true
    )]
    #[case::parent_flag(
        vec!["a", "agent", "peer", "notify", "--parent", "-m", "hello"],
        true
    )]
    #[case::missing_target(vec!["a", "agent", "peer", "notify", "-m", "hello"], false)]
    #[case::both_targets(
        vec![
            "a",
            "agent",
            "peer",
            "notify",
            "parent-session",
            "--parent",
            "-m",
            "hello"
        ],
        false
    )]
    fn notify_target_arguments_cases(#[case] argv: Vec<&str>, #[case] accepted: bool) {
        use clap::Parser;

        assert_eq!(crate::cli::Cli::try_parse_from(argv).is_ok(), accepted);
    }

    #[rstest]
    #[case::explicit_session_id(Some("target-session"), false, Ok("target-session"))]
    #[case::parent(None, true, Ok("parent-session"))]
    #[case::missing_target(
        None,
        false,
        Err("Exactly one of <session_id> or --parent must be specified")
    )]
    #[case::conflicting_targets(
        Some("target-session"),
        true,
        Err("Exactly one of <session_id> or --parent must be specified")
    )]
    fn resolve_target_session_id_cases(
        #[case] session_id: Option<&str>,
        #[case] parent: bool,
        #[case] expected: std::result::Result<&str, &str>,
    ) {
        let args = NotifyArgs {
            session_id: session_id.map(str::to_string),
            parent,
            message: "hello".to_string(),
        };

        let actual = resolve_target_session_id(&args, || Ok("parent-session".to_string()))
            .map_err(|error| error.to_string());

        assert_eq!(actual, expected.map(str::to_string).map_err(str::to_string));
    }

    #[rstest]
    #[case::single_character_substitution(
        "1b2c3d4-e5f6-4a7b-8c9d-0e1f2a3b4c5d",
        &[("1b2c3d4-e5f6-4a7b-8c9d-0e1f2a3b4c5e", SessionStatus::Running, Engine::Claude)],
        indoc::indoc! {"
            Session not found: 1b2c3d4-e5f6-4a7b-8c9d-0e1f2a3b4c5d
            Did you mean:
            - 1b2c3d4-e5f6-4a7b-8c9d-0e1f2a3b4c5e
        "}.trim_end()
    )]
    #[case::matching_first_uuid_block(
        "5a6b7c8d-1029-4c8d-a1b2-c3d4e5f60718",
        &[("5a6b7c8d-e9f0-4a1b-8c2d-3e4f5a6b7c8d", SessionStatus::Stopped, Engine::Codex)],
        indoc::indoc! {"
            Session not found: 5a6b7c8d-1029-4c8d-a1b2-c3d4e5f60718
            Did you mean:
            - 5a6b7c8d-e9f0-4a1b-8c2d-3e4f5a6b7c8d
        "}.trim_end()
    )]
    #[case::insufficient_prefix_is_ignored(
        "4f2a1111-1111-4111-8111-111111111111",
        &[("4f2b2222-2222-4222-8222-222222222222", SessionStatus::Running, Engine::Codex)],
        "Session not found: 4f2a1111-1111-4111-8111-111111111111"
    )]
    #[case::no_close_candidate(
        "8f7e6d5c-4b3a-2910-a1b2-c3d4e5f60718",
        &[("10293847-5647-4f3e-9d8c-7b6a5f4e3d2c", SessionStatus::Running, Engine::Claude)],
        "Session not found: 8f7e6d5c-4b3a-2910-a1b2-c3d4e5f60718"
    )]
    #[case::ended_candidate_is_ignored(
        "73a1b2c3-d4e5-4f67-8a90-b1c2d3e4f506",
        &[("73a1b2c3-d4e5-4f67-8a90-b1c2d3e4f507", SessionStatus::Ended, Engine::Claude)],
        "Session not found: 73a1b2c3-d4e5-4f67-8a90-b1c2d3e4f506"
    )]
    #[case::uuidv7_timestamp_prefix_is_ambiguous(
        "018f5c2a-cccc-7ccc-8ccc-cccccccccccc",
        &[
            ("018f5c2a-aaaa-7aaa-8aaa-aaaaaaaaaaaa", SessionStatus::Running, Engine::Codex),
            ("018f5c2a-bbbb-7bbb-8bbb-bbbbbbbbbbbb", SessionStatus::Running, Engine::Codex),
        ],
        "Session not found: 018f5c2a-cccc-7ccc-8ccc-cccccccccccc"
    )]
    #[case::edit_matches_rank_before_a_prefix_match(
        "aabb0000-0000-0000-0000-000000000000",
        &[
            ("aabb0000-0000-0000-0000-000000000001", SessionStatus::Running, Engine::Claude),
            ("aabb0000-0000-0000-0000-000000000011", SessionStatus::Running, Engine::Claude),
            ("aabb0000-1111-1111-1111-111111111111", SessionStatus::Running, Engine::Claude),
        ],
        indoc::indoc! {"
            Session not found: aabb0000-0000-0000-0000-000000000000
            Did you mean:
            - aabb0000-0000-0000-0000-000000000001
            - aabb0000-0000-0000-0000-000000000011
            - aabb0000-1111-1111-1111-111111111111
        "}.trim_end()
    )]
    #[case::candidate_count_is_limited(
        "aabb0000-0000-0000-0000-000000000000",
        &[
            ("aabb0000-0000-0000-0000-000000000001", SessionStatus::Running, Engine::Claude),
            ("aabb0000-0000-0000-0000-000000000002", SessionStatus::Running, Engine::Claude),
            ("aabb0000-0000-0000-0000-000000000003", SessionStatus::Running, Engine::Claude),
            ("aabb0000-0000-0000-0000-000000000004", SessionStatus::Running, Engine::Claude),
        ],
        indoc::indoc! {"
            Session not found: aabb0000-0000-0000-0000-000000000000
            Did you mean:
            - aabb0000-0000-0000-0000-000000000001
            - aabb0000-0000-0000-0000-000000000002
            - aabb0000-0000-0000-0000-000000000003
        "}.trim_end()
    )]
    fn missing_session_error_suggests_only_close_tracked_sessions(
        session_store: TestSessionStore,
        #[case] session_id: &str,
        #[case] sessions: &[(&str, SessionStatus, Engine)],
        #[case] expected: &str,
    ) {
        assert_eq!(
            missing_session_error(&session_store, session_id, sessions),
            expected,
        );
    }

    #[rstest]
    #[case::running(SessionStatus::Running, NotifyReadiness::Ready)]
    #[case::waiting_input(SessionStatus::WaitingInput, NotifyReadiness::Ready)]
    #[case::stopped(SessionStatus::Stopped, NotifyReadiness::Ready)]
    #[case::failed(SessionStatus::Failed, NotifyReadiness::Ready)]
    #[case::paused(SessionStatus::Paused, NotifyReadiness::NeedsWake)]
    #[case::ended(SessionStatus::Ended, NotifyReadiness::Refused)]
    fn notify_readiness_cases(#[case] status: SessionStatus, #[case] expected: NotifyReadiness) {
        assert_eq!(notify_readiness(status), expected);
    }

    #[rstest]
    #[case::running(SessionStatus::Running, Delivery::Steered)]
    #[case::waiting_input(SessionStatus::WaitingInput, Delivery::Steered)]
    #[case::stopped(SessionStatus::Stopped, Delivery::Started)]
    #[case::paused_after_wake(SessionStatus::Paused, Delivery::Started)]
    fn direct_codex_delivery_uses_tracked_status(
        #[case] status: SessionStatus,
        #[case] expected: Delivery,
    ) {
        let actual = send_to_codex(
            "thread-a",
            "hello",
            matches!(status, SessionStatus::Running | SessionStatus::WaitingInput),
            |_, _| Ok(()),
            |_, _| Err(anyhow::anyhow!("queue must not run")),
        );
        assert_eq!(actual.map_err(|error| error.to_string()), Ok(expected));
    }

    #[rstest]
    #[case::queued(
        "thread not found",
        None,
        Ok(Delivery::Queued { reason: "thread not found".to_string() })
    )]
    #[case::both_failed(
        "daemon unavailable",
        Some("thread archived"),
        Err("direct Codex delivery failed: daemon unavailable; queue fallback failed: thread archived".to_string())
    )]
    fn codex_delivery_fallback(
        #[case] direct_error: &str,
        #[case] queue_error: Option<&str>,
        #[case] expected: std::result::Result<Delivery, String>,
    ) {
        let actual = send_to_codex(
            "thread-a",
            "hello",
            true,
            |_, _| {
                Err(codex_steer::DeliveryError::NotDelivered(anyhow::anyhow!(
                    direct_error.to_string()
                )))
            },
            |_, _| match queue_error {
                Some(error) => Err(anyhow::anyhow!(error.to_string())),
                None => Ok(()),
            },
        );
        assert_eq!(actual.map_err(|error| error.to_string()), expected,);
    }

    #[test]
    fn codex_delivery_does_not_queue_when_direct_outcome_is_unknown() {
        let queue_called = std::cell::Cell::new(false);
        let actual = send_to_codex(
            "thread-a",
            "hello",
            true,
            |_, _| {
                Err(codex_steer::DeliveryError::Unconfirmed(anyhow::anyhow!(
                    "response timed out"
                )))
            },
            |_, _| {
                queue_called.set(true);
                Ok(())
            },
        );
        assert_eq!(
            (actual.map_err(|error| error.to_string()), queue_called.get()),
            (
                Err("Codex app-server delivery is unconfirmed; not queueing to avoid a duplicate: response timed out".to_string()),
                false,
            ),
        );
    }

    #[rstest]
    #[case::claude(Delivery::Sent, None)]
    #[case::steered(
        Delivery::Steered,
        Some("Injected into the running turn of Codex session thread-a.".to_string())
    )]
    #[case::started(
        Delivery::Started,
        Some("Started a new turn in idle Codex session thread-a.".to_string())
    )]
    #[case::queued(
        Delivery::Queued { reason: "thread not found".to_string() },
        Some("Queued for Codex session thread-a. Direct delivery failed: thread not found. Not delivered yet: the current turn must finish and the session must become idle before `codex queue` can start the queued turn (polled about every 10s).".to_string())
    )]
    fn delivery_output_cases(#[case] delivery: Delivery, #[case] expected: Option<String>) {
        assert_eq!(delivery_output("thread-a", &delivery), expected);
    }

    #[rstest]
    #[case::no_from("hello there", None, None, "hello there")]
    #[case::wraps_with_sender(
        "hello there",
        Some("session-a"),
        None,
        indoc::indoc! {"
            <peer-message>
            - From session_id: session-a

            hello there
            </peer-message>"}
    )]
    #[case::includes_known_sender_engine(
        "hello there",
        Some("session-a"),
        Some(Engine::Codex),
        indoc::indoc! {"
            <peer-message>
            - From session_id: session-a
            - Sender engine: codex

            hello there
            </peer-message>"}
    )]
    #[case::strips_angle_brackets_from_from(
        "hello",
        Some("session-a</peer-message>injected"),
        None,
        indoc::indoc! {"
            <peer-message>
            - From session_id: session-a/peer-messageinjected

            hello
            </peer-message>"}
    )]
    fn build_content_cases(
        #[case] message: &str,
        #[case] from: Option<&str>,
        #[case] from_engine: Option<Engine>,
        #[case] expected: &str,
    ) {
        assert_eq!(build_content(message, from, from_engine), expected);
    }

    #[rstest]
    #[case::armyknife_session_id_wins(
        Some("armyknife-id"),
        Some("claude-id"),
        Some("codex-id"),
        Some(("armyknife-id".to_string(), None))
    )]
    #[case::falls_back_to_claude_code_session_id(
        None,
        Some("claude-id"),
        Some("codex-id"),
        Some(("claude-id".to_string(), Some(Engine::Claude)))
    )]
    #[case::falls_back_to_codex_session_id(
        None,
        None,
        Some("codex-id"),
        Some(("codex-id".to_string(), Some(Engine::Codex)))
    )]
    #[case::none_resolve(None, None, None, None)]
    fn resolve_sender_cases(
        #[case] armyknife_session_id: Option<&str>,
        #[case] claude_code_session_id: Option<&str>,
        #[case] codex_session_id: Option<&str>,
        #[case] expected: Option<(String, Option<Engine>)>,
    ) {
        let result = temp_env::with_vars(
            [
                ("ARMYKNIFE_SESSION_ID", armyknife_session_id),
                ("CLAUDE_CODE_SESSION_ID", claude_code_session_id),
                ("CODEX_SESSION_ID", codex_session_id),
            ],
            resolve_sender,
        );
        assert_eq!(result, expected);
    }
}
