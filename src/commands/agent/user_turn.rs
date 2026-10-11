use std::cmp::Ordering;

use super::types::{AgentStatusKind, DisplayStatus, Session, SessionStatus};
use crate::shared::human_in_the_loop::PR_DRAFT_REVIEW_MARKER;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "failed and merge candidates are ordered here for external status sources"
    )
)]
pub(crate) enum UserTurnKind {
    Failed,
    Crit,
    Body,
    Decide,
    Read,
    Do,
    Idle,
    Allow,
    Ask,
    Merge,
    Close,
}

impl UserTurnKind {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Failed => "failed",
            Self::Crit => "crit",
            Self::Body => "body",
            Self::Decide => "decide",
            Self::Read => "read",
            Self::Do => "do",
            Self::Idle => "idle",
            Self::Allow => "allow",
            Self::Ask => "ask",
            Self::Merge => "merge",
            Self::Close => "close",
        }
    }

    pub(crate) fn symbol(self) -> &'static str {
        match self {
            Self::Failed => "✗",
            Self::Close => "◌",
            _ => "◐",
        }
    }

    fn priority(self) -> u8 {
        match self {
            Self::Failed => 0,
            Self::Crit => 1,
            Self::Body => 2,
            Self::Decide => 3,
            Self::Read => 4,
            Self::Do => 5,
            Self::Idle => 6,
            Self::Allow => 7,
            Self::Ask => 8,
            Self::Merge => 9,
            Self::Close => 10,
        }
    }
}

#[derive(Debug)]
pub(crate) struct UserTurn<'a> {
    pub(crate) session: &'a Session,
    pub(crate) kind: UserTurnKind,
    pub(crate) note: Option<&'a str>,
}

pub(crate) fn user_turns(sessions: &[Session]) -> Vec<UserTurn<'_>> {
    let mut turns = sessions.iter().filter_map(user_turn).collect::<Vec<_>>();
    sort_user_turns(&mut turns);
    turns
}

pub(crate) fn sort_user_turns(turns: &mut [UserTurn<'_>]) {
    turns.sort_by(compare_turns);
}

pub(crate) fn next_user_turn(sessions: &[Session]) -> Option<UserTurn<'_>> {
    user_turns(sessions).into_iter().next()
}

fn user_turn(session: &Session) -> Option<UserTurn<'_>> {
    let kind = if session.has_pending_permission_requests() {
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
        session
            .agent_status
            .as_ref()
            .and_then(|status| match status.kind {
                AgentStatusKind::Decide => Some(UserTurnKind::Decide),
                AgentStatusKind::Read => Some(UserTurnKind::Read),
                AgentStatusKind::Do => Some(UserTurnKind::Do),
                AgentStatusKind::Idle => Some(UserTurnKind::Idle),
                AgentStatusKind::Close => Some(UserTurnKind::Close),
                AgentStatusKind::Done | AgentStatusKind::Wait => None,
            })
    } else {
        None
    }?;

    let note = match kind {
        UserTurnKind::Decide
        | UserTurnKind::Read
        | UserTurnKind::Do
        | UserTurnKind::Idle
        | UserTurnKind::Close => session
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
        .priority()
        .cmp(&right.kind.priority())
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

    fn session(session_id: &str, updated_at: DateTime<Utc>) -> Session {
        Session {
            session_id: session_id.to_string(),
            work_type: None,
            work_type_pinned: false,
            crit_urls: Vec::new(),
            pending_human_review_ids: BTreeSet::new(),
            cwd: PathBuf::from("/tmp/example"),
            transcript_path: None,
            tty: None,
            tmux_info: None,
            status: SessionStatus::Stopped,
            agent_status: None,
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
            engine: Engine::Claude,
        }
    }

    fn timestamp(seconds: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(seconds, 0)
            .single()
            .expect("valid timestamp")
    }

    #[test]
    fn user_turns_sort_by_kind_then_oldest_update() {
        let mut newer_decide = session("decide-new", timestamp(30));
        newer_decide.agent_status = Some(AgentStatus {
            kind: AgentStatusKind::Decide,
            note: "choose a path".to_string(),
        });
        let mut older_decide = session("decide-old", timestamp(10));
        older_decide.agent_status = Some(AgentStatus {
            kind: AgentStatusKind::Decide,
            note: "choose a path".to_string(),
        });
        let mut crit = session("crit", timestamp(40));
        crit.crit_urls
            .push("https://example.test/review".to_string());
        crit.pending_bg_task_ids.insert("review-task".to_string());
        let mut close = session("close", timestamp(1));
        close.agent_status = Some(AgentStatus {
            kind: AgentStatusKind::Close,
            note: "close the worktree".to_string(),
        });
        let mut waiting = session("wait", timestamp(2));
        waiting.agent_status = Some(AgentStatus {
            kind: AgentStatusKind::Wait,
            note: "waiting for CI".to_string(),
        });

        assert_eq!(
            user_turns(&[newer_decide, older_decide, crit, close, waiting])
                .into_iter()
                .map(|turn| (turn.session.session_id.as_str(), turn.kind))
                .collect::<Vec<_>>(),
            vec![
                ("crit", UserTurnKind::Crit),
                ("decide-old", UserTurnKind::Decide),
                ("decide-new", UserTurnKind::Decide),
                ("close", UserTurnKind::Close),
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
            next_user_turn(&[session]).map(|turn| turn.kind),
            Some(expected)
        );
    }

    #[rstest]
    #[case::decide(AgentStatusKind::Decide, Some(UserTurnKind::Decide))]
    #[case::read(AgentStatusKind::Read, Some(UserTurnKind::Read))]
    #[case::do_work(AgentStatusKind::Do, Some(UserTurnKind::Do))]
    #[case::idle(AgentStatusKind::Idle, Some(UserTurnKind::Idle))]
    #[case::close(AgentStatusKind::Close, Some(UserTurnKind::Close))]
    #[case::done(AgentStatusKind::Done, None)]
    #[case::wait(AgentStatusKind::Wait, None)]
    fn agent_status_kinds_map_to_user_turns(
        #[case] kind: AgentStatusKind,
        #[case] expected: Option<UserTurnKind>,
    ) {
        let mut session = session("status", timestamp(1));
        session.agent_status = Some(AgentStatus {
            kind,
            note: "describe the next step".to_string(),
        });

        assert_eq!(next_user_turn(&[session]).map(|turn| turn.kind), expected);
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

        assert_eq!(next_user_turn(&[session]).map(|turn| turn.kind), None);
    }

    #[test]
    fn close_is_selected_only_when_no_higher_priority_item_exists() {
        let mut close = session("close", timestamp(1));
        close.agent_status = Some(AgentStatus {
            kind: AgentStatusKind::Close,
            note: String::new(),
        });
        let mut read = session("read", timestamp(2));
        read.agent_status = Some(AgentStatus {
            kind: AgentStatusKind::Read,
            note: "answer the question".to_string(),
        });

        assert_eq!(
            (
                next_user_turn(&[close.clone(), read]).map(|turn| turn.kind),
                next_user_turn(&[close]).map(|turn| turn.kind),
                next_user_turn(&[]).map(|turn| turn.kind),
            ),
            (Some(UserTurnKind::Read), Some(UserTurnKind::Close), None,)
        );
    }

    #[test]
    fn external_failure_and_merge_items_use_the_shared_order() {
        let sessions = [
            session("close", timestamp(1)),
            session("merge", timestamp(2)),
            session("ask", timestamp(3)),
            session("allow", timestamp(4)),
            session("idle", timestamp(5)),
            session("do", timestamp(6)),
            session("read", timestamp(7)),
            session("decide", timestamp(8)),
            session("body", timestamp(9)),
            session("crit", timestamp(10)),
            session("failed", timestamp(11)),
        ];
        let mut turns = [
            UserTurn {
                session: &sessions[0],
                kind: UserTurnKind::Close,
                note: None,
            },
            UserTurn {
                session: &sessions[1],
                kind: UserTurnKind::Merge,
                note: None,
            },
            UserTurn {
                session: &sessions[2],
                kind: UserTurnKind::Ask,
                note: None,
            },
            UserTurn {
                session: &sessions[3],
                kind: UserTurnKind::Allow,
                note: None,
            },
            UserTurn {
                session: &sessions[4],
                kind: UserTurnKind::Idle,
                note: None,
            },
            UserTurn {
                session: &sessions[5],
                kind: UserTurnKind::Do,
                note: None,
            },
            UserTurn {
                session: &sessions[6],
                kind: UserTurnKind::Read,
                note: None,
            },
            UserTurn {
                session: &sessions[7],
                kind: UserTurnKind::Decide,
                note: None,
            },
            UserTurn {
                session: &sessions[8],
                kind: UserTurnKind::Body,
                note: None,
            },
            UserTurn {
                session: &sessions[9],
                kind: UserTurnKind::Crit,
                note: None,
            },
            UserTurn {
                session: &sessions[10],
                kind: UserTurnKind::Failed,
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
                UserTurnKind::Decide,
                UserTurnKind::Read,
                UserTurnKind::Do,
                UserTurnKind::Idle,
                UserTurnKind::Allow,
                UserTurnKind::Ask,
                UserTurnKind::Merge,
                UserTurnKind::Close,
            ]
        );
    }
}
