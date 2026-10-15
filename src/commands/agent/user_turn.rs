use std::cmp::Ordering;

use super::types::{DisplayStatus, Session, SessionStatus};
use crate::shared::config::{AgentConfig, AgentStatusState};
use crate::shared::human_in_the_loop::PR_DRAFT_REVIEW_MARKER;

/// Variant order is the shared next-action priority used by tmux status and watch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "merge candidates are ordered here for external status sources"
    )
)]
pub(crate) enum UserTurnKind {
    Failed,
    Crit,
    Body,
    AgentStatus,
    Allow,
    Ask,
    Merge,
}

impl UserTurnKind {
    pub(crate) fn label(self) -> Option<&'static str> {
        match self {
            Self::Failed => Some("failed"),
            Self::Crit => Some("crit"),
            Self::Body => Some("body"),
            Self::AgentStatus => None,
            Self::Allow => Some("allow"),
            Self::Ask => Some("ask"),
            Self::Merge => Some("merge"),
        }
    }

    pub(crate) fn symbol(self) -> &'static str {
        match self {
            Self::Failed => "✗",
            _ => "◐",
        }
    }
}

#[derive(Debug)]
pub(crate) struct UserTurn<'a> {
    pub(crate) session: &'a Session,
    pub(crate) kind: UserTurnKind,
    pub(crate) label: Option<&'a str>,
    pub(crate) note: Option<&'a str>,
}

pub(crate) fn user_turns<'a>(
    sessions: impl IntoIterator<Item = &'a Session>,
    agent_config: &AgentConfig,
) -> Vec<UserTurn<'a>> {
    let mut turns = sessions
        .into_iter()
        .filter_map(|session| user_turn(session, agent_config))
        .collect::<Vec<_>>();
    sort_user_turns(&mut turns);
    turns
}

pub(crate) fn sort_user_turns(turns: &mut [UserTurn<'_>]) {
    turns.sort_by(compare_turns);
}

pub(crate) fn next_user_turn<'a>(
    sessions: &'a [Session],
    agent_config: &AgentConfig,
) -> Option<UserTurn<'a>> {
    user_turns(sessions, agent_config).into_iter().next()
}

fn user_turn<'a>(session: &'a Session, agent_config: &AgentConfig) -> Option<UserTurn<'a>> {
    let kind = if session.status == SessionStatus::Failed {
        Some(UserTurnKind::Failed)
    } else if session.has_pending_permission_requests() {
        Some(UserTurnKind::Allow)
    } else if is_ask_user_question(session) {
        Some(UserTurnKind::Ask)
    } else if session.display_status() == DisplayStatus::WaitingInput {
        if !session.crit_urls.is_empty() {
            Some(UserTurnKind::Crit)
        } else if session
            .pending_human_review_ids
            .iter()
            .any(|id| id.starts_with(PR_DRAFT_REVIEW_MARKER))
        {
            Some(UserTurnKind::Body)
        } else {
            None
        }
    } else if matches!(
        session.status,
        SessionStatus::Stopped | SessionStatus::Paused | SessionStatus::Ended
    ) {
        session.agent_status.as_ref().and_then(|status| {
            match agent_config.status_state(&status.kind) {
                AgentStatusState::User => Some(UserTurnKind::AgentStatus),
                AgentStatusState::Waiting | AgentStatusState::Done => None,
            }
        })
    } else {
        None
    }?;

    let label = match kind {
        UserTurnKind::AgentStatus => session
            .agent_status
            .as_ref()
            .map(|status| status.kind.as_str()),
        _ => kind.label(),
    };
    let note = match kind {
        UserTurnKind::AgentStatus => session
            .agent_status
            .as_ref()
            .map(|status| status.note.as_str()),
        UserTurnKind::Allow => session.current_tool.as_deref(),
        _ => None,
    }
    .filter(|note| !note.is_empty());

    Some(UserTurn {
        session,
        kind,
        label,
        note,
    })
}

fn is_ask_user_question(session: &Session) -> bool {
    session
        .current_tool
        .as_deref()
        .is_some_and(|tool| tool == "AskUserQuestion" || tool.starts_with("AskUserQuestion("))
}

fn compare_turns(left: &UserTurn<'_>, right: &UserTurn<'_>) -> Ordering {
    left.kind
        .cmp(&right.kind)
        .then_with(|| left.session.updated_at.cmp(&right.session.updated_at))
        .then_with(|| left.session.session_id.cmp(&right.session.session_id))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::path::PathBuf;

    use chrono::{DateTime, TimeZone, Utc};
    use rstest::rstest;

    use super::*;
    use crate::commands::agent::types::{AgentStatus, Engine};
    use crate::shared::config::AgentStatusConfig;
    use crate::shared::config::test_support::agent_config;

    fn session(session_id: &str, updated_at: DateTime<Utc>) -> Session {
        Session {
            pr_context: None,
            session_id: session_id.to_string(),
            work_type: None,
            work_type_pinned: false,
            vibe_enabled: false,
            launched_with_vibe: false,
            is_delegated: false,
            crit_urls: Vec::new(),
            pending_human_review_ids: BTreeSet::new(),
            cwd: PathBuf::from("/tmp/example"),
            transcript_path: None,
            tty: None,
            tmux_info: None,
            status: SessionStatus::Stopped,
            agent_status: None,
            agent_status_clear_pending: false,
            created_at: updated_at,
            updated_at,
            last_message: None,
            current_tool: None,
            label: None,
            ancestor_session_ids: Vec::new(),
            pending_bg_task_ids: BTreeSet::new(),
            pending_agent_task_ids: BTreeSet::new(),
            pending_permission_agent_ids: BTreeSet::new(),
            pending_permission_request_ids: Default::default(),
            read_at: None,
            sweep_signaled: false,
            stop_blocked_by_armyknife: false,
            engine: Engine::Claude,
        }
    }

    fn timestamp(seconds: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(seconds, 0)
            .single()
            .expect("valid timestamp")
    }

    #[rstest]
    fn user_turns_sort_by_kind_then_oldest_update(agent_config: AgentConfig) {
        let mut newer_user_turn = session("user-new", timestamp(30));
        newer_user_turn.agent_status = Some(AgentStatus {
            kind: "needs_attention".to_string(),
            note: "review the proposal".to_string(),
        });
        let mut older_user_turn = session("user-old", timestamp(10));
        older_user_turn.agent_status = Some(AgentStatus {
            kind: "needs_attention".to_string(),
            note: "review the proposal".to_string(),
        });
        let mut crit = session("crit", timestamp(40));
        crit.crit_urls
            .push("https://example.test/review".to_string());
        crit.pending_bg_task_ids.insert("review-task".to_string());
        let mut done = session("finished", timestamp(1));
        done.agent_status = Some(AgentStatus {
            kind: "finished_work".to_string(),
            note: "work is complete".to_string(),
        });
        let mut waiting = session("external", timestamp(2));
        waiting.agent_status = Some(AgentStatus {
            kind: "awaiting_build".to_string(),
            note: "waiting for a build".to_string(),
        });

        assert_eq!(
            user_turns(
                &[newer_user_turn, older_user_turn, crit, done, waiting],
                &agent_config,
            )
            .into_iter()
            .map(|turn| (turn.session.session_id.as_str(), turn.kind))
            .collect::<Vec<_>>(),
            vec![
                ("crit", UserTurnKind::Crit),
                ("user-old", UserTurnKind::AgentStatus),
                ("user-new", UserTurnKind::AgentStatus),
            ]
        );
    }

    #[rstest]
    #[case::crit(UserTurnKind::Crit)]
    #[case::body(UserTurnKind::Body)]
    #[case::permission(UserTurnKind::Allow)]
    #[case::ask(UserTurnKind::Ask)]
    fn automatic_user_turn_markers_are_detected(#[case] expected: UserTurnKind) {
        let mut session = session("waiting", timestamp(1));
        match expected {
            UserTurnKind::Crit => {
                session
                    .crit_urls
                    .push("https://example.test/review".to_string());
                session
                    .pending_bg_task_ids
                    .insert("review-task".to_string());
            }
            UserTurnKind::Body => {
                session
                    .pending_human_review_ids
                    .insert(format!("{PR_DRAFT_REVIEW_MARKER}review-marker"));
                session
                    .pending_bg_task_ids
                    .insert("review-task".to_string());
            }
            UserTurnKind::Allow => {
                session
                    .pending_permission_agent_ids
                    .insert("agent".to_string());
            }
            UserTurnKind::Ask => {
                session.current_tool = Some("AskUserQuestion".to_string());
                session.status = SessionStatus::Running;
            }
            _ => unreachable!("test case only covers automatic markers"),
        }

        assert_eq!(
            next_user_turn(&[session], &AgentConfig::default()).map(|turn| turn.kind),
            Some(expected)
        );
    }

    #[test]
    fn failed_session_is_reported_as_a_user_turn() {
        let mut session = session("failed", timestamp(1));
        session.status = SessionStatus::Failed;

        assert_eq!(
            next_user_turn(&[session], &AgentConfig::default())
                .map(|turn| (turn.kind, turn.label, turn.note)),
            Some((UserTurnKind::Failed, Some("failed"), None))
        );
    }

    #[rstest]
    #[case::configured_user(
        "needs_attention",
        Some(AgentStatusState::User),
        Some(UserTurnKind::AgentStatus),
        Some("needs_attention")
    )]
    #[case::configured_waiting("external_wait", Some(AgentStatusState::Waiting), None, None)]
    #[case::configured_done("work_finished", Some(AgentStatusState::Done), None, None)]
    #[case::unknown_is_user(
        "removed_name",
        None,
        Some(UserTurnKind::AgentStatus),
        Some("removed_name")
    )]
    fn agent_status_names_map_to_user_turns(
        #[case] kind: &str,
        #[case] state: Option<AgentStatusState>,
        #[case] expected: Option<UserTurnKind>,
        #[case] expected_label: Option<&str>,
    ) {
        let mut agent_config = AgentConfig::default();
        if let Some(state) = state {
            agent_config
                .statuses
                .insert(kind.to_string(), AgentStatusConfig { state });
        }
        let mut session = session("status", timestamp(1));
        session.agent_status = Some(AgentStatus {
            kind: kind.to_string(),
            note: "describe the next step".to_string(),
        });

        assert_eq!(
            next_user_turn(&[session], &agent_config)
                .map(|turn| (turn.kind, turn.label, turn.note)),
            expected.map(|kind| (kind, expected_label, Some("describe the next step")))
        );
    }

    #[test]
    fn generic_human_review_is_not_labeled_as_pr_body_review() {
        let mut session = session("review", timestamp(1));
        session
            .pending_human_review_ids
            .insert("review-marker".to_string());
        session
            .pending_bg_task_ids
            .insert("review-task".to_string());

        assert_eq!(
            next_user_turn(&[session], &AgentConfig::default()).map(|turn| turn.kind),
            None
        );
    }

    #[test]
    fn external_failure_and_merge_items_use_the_shared_order() {
        let sessions = [
            session("status", timestamp(1)),
            session("merge", timestamp(2)),
            session("ask", timestamp(3)),
            session("allow", timestamp(4)),
            session("body", timestamp(5)),
            session("crit", timestamp(6)),
            session("failed", timestamp(7)),
        ];
        let mut turns = [
            UserTurn {
                session: &sessions[0],
                kind: UserTurnKind::AgentStatus,
                label: None,
                note: None,
            },
            UserTurn {
                session: &sessions[1],
                kind: UserTurnKind::Merge,
                label: None,
                note: None,
            },
            UserTurn {
                session: &sessions[2],
                kind: UserTurnKind::Ask,
                label: None,
                note: None,
            },
            UserTurn {
                session: &sessions[3],
                kind: UserTurnKind::Allow,
                label: None,
                note: None,
            },
            UserTurn {
                session: &sessions[4],
                kind: UserTurnKind::Body,
                label: None,
                note: None,
            },
            UserTurn {
                session: &sessions[5],
                kind: UserTurnKind::Crit,
                label: None,
                note: None,
            },
            UserTurn {
                session: &sessions[6],
                kind: UserTurnKind::Failed,
                label: None,
                note: None,
            },
        ];

        sort_user_turns(&mut turns);

        assert_eq!(
            turns.into_iter().map(|turn| turn.kind).collect::<Vec<_>>(),
            vec![
                UserTurnKind::Failed,
                UserTurnKind::Crit,
                UserTurnKind::Body,
                UserTurnKind::AgentStatus,
                UserTurnKind::Allow,
                UserTurnKind::Ask,
                UserTurnKind::Merge,
            ]
        );
    }
}
