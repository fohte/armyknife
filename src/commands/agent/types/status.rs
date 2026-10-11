use super::{DisplayStatus, Engine, Session, SessionStatus, StatusColor};

impl Session {
    /// A `Stopped` session is unread when it has never been focused since its
    /// most recent transition into `Stopped`. Drives the `✱` glyph.
    pub fn is_unread_stopped(&self) -> bool {
        self.status == SessionStatus::Stopped && self.read_at.is_none()
    }

    /// True if this session has pending background task or Task-tool subagent
    /// IDs. `pending_bg_task_ids` includes Claude Code task IDs and
    /// `a agent bg run`'s runtime marker.
    pub fn has_pending_bg_tasks(&self) -> bool {
        !self.pending_bg_task_ids.is_empty() || !self.pending_agent_task_ids.is_empty()
    }

    /// True if any agent (main thread or subagent) in this session is
    /// currently blocked on a permission prompt. See
    /// `pending_permission_agent_ids` for what forces this to clear.
    pub fn has_pending_permission_requests(&self) -> bool {
        !self.pending_permission_agent_ids.is_empty()
    }

    /// Presentation status for this session. A stopped main loop with pending
    /// background work is `WaitingInput` when a crit review is linked or a
    /// Human-in-the-Loop review is pending, and `Background` otherwise. Codex
    /// hooks do not report background task IDs, so its review markers also
    /// qualify for this display state.
    /// Notifications, `auto_pause`, `auto_compact`, and `sweep` read persisted
    /// status or `has_pending_bg_tasks` directly and must not switch to this.
    pub fn display_status(&self) -> DisplayStatus {
        let has_pending_bg_tasks = self.has_pending_bg_tasks();
        let has_review_wait_marker =
            !self.crit_urls.is_empty() || !self.pending_human_review_ids.is_empty();
        let codex_review_wait = self.engine == Engine::Codex && has_review_wait_marker;

        if self.status == SessionStatus::Stopped && (has_pending_bg_tasks || codex_review_wait) {
            return if has_review_wait_marker {
                DisplayStatus::WaitingInput
            } else {
                DisplayStatus::Background
            };
        }
        if self.is_unread_stopped() {
            return DisplayStatus::UnreadStopped;
        }
        match self.status {
            SessionStatus::Running => DisplayStatus::Running,
            SessionStatus::WaitingInput => DisplayStatus::WaitingInput,
            SessionStatus::Stopped => DisplayStatus::Stopped,
            SessionStatus::Paused => DisplayStatus::Paused,
            SessionStatus::Ended => DisplayStatus::Ended,
        }
    }

    /// Status symbol that also reflects unread state and in-flight
    /// background tasks. See `display_status`.
    pub fn display_symbol(&self) -> &'static str {
        self.display_status().display_symbol()
    }
}

impl SessionStatus {
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::WaitingInput => "waiting",
            Self::Stopped => "stopped",
            Self::Paused => "paused",
            Self::Ended => "ended",
        }
    }
}

impl DisplayStatus {
    pub fn display_symbol(&self) -> &'static str {
        match self {
            Self::Running => "●",
            Self::WaitingInput => "◐",
            Self::Stopped | Self::Ended => "○",
            Self::UnreadStopped => "✱",
            Self::Paused => "⏸",
            Self::Background => "◎",
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Background => "bg",
            Self::WaitingInput => "waiting",
            Self::Stopped | Self::UnreadStopped => "stopped",
            Self::Paused => "paused",
            Self::Ended => "ended",
        }
    }

    pub fn color(&self) -> StatusColor {
        match self {
            Self::Running => StatusColor::Green,
            Self::WaitingInput => StatusColor::Yellow,
            Self::Background => StatusColor::Cyan,
            Self::Paused => StatusColor::Dim,
            Self::Stopped | Self::UnreadStopped | Self::Ended => StatusColor::Gray,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::Engine;
    use super::*;
    use chrono::{DateTime, Utc};
    use rstest::rstest;
    use std::collections::BTreeSet;
    use std::path::PathBuf;

    fn session(status: SessionStatus, read_at: Option<DateTime<Utc>>) -> Session {
        Session {
            session_id: "s".to_string(),
            work_type: None,
            work_type_pinned: false,
            crit_urls: Vec::new(),
            pending_human_review_ids: Default::default(),
            cwd: PathBuf::from("/tmp/test"),
            transcript_path: None,
            tty: None,
            tmux_info: None,
            status,
            agent_status: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            last_message: None,
            current_tool: None,
            label: None,
            ancestor_session_ids: Vec::new(),
            pending_bg_task_ids: BTreeSet::new(),
            pending_agent_task_ids: BTreeSet::new(),
            pending_permission_agent_ids: BTreeSet::new(),
            pending_permission_request_ids: Default::default(),
            read_at,
            sweep_signaled: false,
            engine: Engine::Claude,
        }
    }

    #[rstest]
    #[case::running_unread(SessionStatus::Running, None, false, false, "\u{25cf}")]
    #[case::running_read(SessionStatus::Running, Some(()), false, false, "\u{25cf}")]
    // A session actually running is `Running` regardless of a bg task -- only
    // an idle main loop with a pending bg task and no crit link renders as
    // `Background`.
    #[case::running_with_bg_task(SessionStatus::Running, None, true, false, "\u{25cf}")]
    #[case::stopped_with_bg_task(SessionStatus::Stopped, None, true, false, "\u{25ce}")]
    #[case::crit_review_waiting(SessionStatus::Stopped, Some(()), true, true, "\u{25d0}")]
    #[case::running_with_crit_review(SessionStatus::Running, None, true, true, "\u{25cf}")]
    #[case::crit_without_pending_task(SessionStatus::Stopped, Some(()), false, true, "\u{25cb}")]
    #[case::waiting_unread(SessionStatus::WaitingInput, None, false, false, "\u{25d0}")]
    #[case::stopped_unread(SessionStatus::Stopped, None, false, false, "\u{2731}")]
    #[case::stopped_read(SessionStatus::Stopped, Some(()), false, false, "\u{25cb}")]
    #[case::paused_unread(SessionStatus::Paused, None, false, false, "\u{23f8}")]
    #[case::paused_read(SessionStatus::Paused, Some(()), false, false, "\u{23f8}")]
    #[case::ended_unread(SessionStatus::Ended, None, false, false, "\u{25cb}")]
    #[case::ended_read(SessionStatus::Ended, Some(()), false, false, "\u{25cb}")]
    fn session_display_symbol_table(
        #[case] status: SessionStatus,
        #[case] read_marker: Option<()>,
        #[case] has_bg_task: bool,
        #[case] has_crit_link: bool,
        #[case] expected: &str,
    ) {
        let read_at = read_marker.map(|()| Utc::now());
        let mut s = session(status, read_at);
        if has_bg_task {
            s.pending_bg_task_ids.insert("bg-1".to_string());
        }
        if has_crit_link {
            s.crit_urls
                .push("https://crit.example/review/1".to_string());
        }
        assert_eq!(s.display_symbol(), expected);
    }

    #[rstest]
    #[case::claude_crit_requires_background_task(
        Engine::Claude,
        SessionStatus::Stopped,
        false,
        true,
        false,
        false,
        DisplayStatus::Stopped
    )]
    #[case::claude_review_requires_background_task(
        Engine::Claude,
        SessionStatus::Stopped,
        false,
        false,
        true,
        false,
        DisplayStatus::Stopped
    )]
    #[case::claude_crit_with_background_task(
        Engine::Claude,
        SessionStatus::Stopped,
        true,
        true,
        false,
        true,
        DisplayStatus::WaitingInput
    )]
    #[case::codex_crit_wait(
        Engine::Codex,
        SessionStatus::Stopped,
        false,
        true,
        false,
        false,
        DisplayStatus::WaitingInput
    )]
    #[case::codex_pr_review_wait(
        Engine::Codex,
        SessionStatus::Stopped,
        false,
        false,
        true,
        false,
        DisplayStatus::WaitingInput
    )]
    #[case::codex_without_review(
        Engine::Codex,
        SessionStatus::Stopped,
        false,
        false,
        false,
        false,
        DisplayStatus::Stopped
    )]
    #[case::codex_other_background_task(
        Engine::Codex,
        SessionStatus::Stopped,
        true,
        false,
        false,
        true,
        DisplayStatus::Background
    )]
    #[case::running_codex_review(
        Engine::Codex,
        SessionStatus::Running,
        false,
        true,
        false,
        false,
        DisplayStatus::Running
    )]
    fn review_markers_affect_display_status_without_changing_pending_background_work(
        #[case] engine: Engine,
        #[case] status: SessionStatus,
        #[case] has_background_task: bool,
        #[case] has_crit_link: bool,
        #[case] has_human_review: bool,
        #[case] expected_pending: bool,
        #[case] expected_status: DisplayStatus,
    ) {
        let mut session = session(status, Some(Utc::now()));
        session.engine = engine;
        if has_background_task {
            session.pending_bg_task_ids.insert("task-1".to_string());
        }
        if has_crit_link {
            session
                .crit_urls
                .push("https://crit.example/review/1".to_string());
        }
        if has_human_review {
            session
                .pending_human_review_ids
                .insert("review-id".to_string());
        }
        assert_eq!(
            (session.has_pending_bg_tasks(), session.display_status()),
            (expected_pending, expected_status)
        );
    }

    #[rstest]
    #[case::neither(false, false, false)]
    #[case::bg_only(true, false, true)]
    #[case::agent_only(false, true, true)]
    #[case::both(true, true, true)]
    fn session_has_pending_bg_tasks_table(
        #[case] bg_pending: bool,
        #[case] agent_pending: bool,
        #[case] expected: bool,
    ) {
        let mut s = session(SessionStatus::Stopped, None);
        if bg_pending {
            s.pending_bg_task_ids.insert("bg-1".to_string());
        }
        if agent_pending {
            s.pending_agent_task_ids.insert("agent-1".to_string());
        }
        assert_eq!(s.has_pending_bg_tasks(), expected);
    }

    #[rstest]
    #[case::empty(&[], false)]
    #[case::non_empty(&["agent-1"], true)]
    fn session_has_pending_permission_requests_table(
        #[case] pending: &[&str],
        #[case] expected: bool,
    ) {
        let mut s = session(SessionStatus::WaitingInput, None);
        s.pending_permission_agent_ids = pending.iter().map(|id| (*id).to_string()).collect();
        assert_eq!(s.has_pending_permission_requests(), expected);
    }
}
