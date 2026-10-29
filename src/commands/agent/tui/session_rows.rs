use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::commands::agent::types::Session;
use crate::commands::agent::user_turn::{UserTurnKind, user_turns};
use crate::shared::config::AgentConfig;

pub(super) mod inline_progress;
mod task_tree;

/// One row in the session list's selection/render order.
///
/// Section headers are not individually selectable.
#[derive(Debug)]
pub(super) enum SessionRow<'a> {
    SectionHeader(SectionHeaderRow),
    Session(SessionRowEntry<'a>),
    InlineProgress(Box<inline_progress::InlineProgressEntry<'a>>),
}

#[derive(Debug)]
pub(super) struct SectionHeaderRow {
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) enum SessionRowKey {
    Task(String),
    Progress {
        source_session_id: String,
        selection_key: crate::commands::agent::tui::progress::ProgressSelectionKey,
        session_id: Option<String>,
    },
}

impl SessionRowKey {
    pub(super) fn session_id(&self) -> Option<&str> {
        match self {
            Self::Task(session_id) => Some(session_id),
            Self::Progress { session_id, .. } => session_id.as_deref(),
        }
    }
}

/// Captures session order while list navigation is active.
#[derive(Debug, Default)]
pub(super) struct SessionRowOrder {
    task_positions: HashMap<String, usize>,
    next_task_position: usize,
}

impl SessionRowOrder {
    pub(super) fn from_rows(rows: &[SessionRow<'_>]) -> Self {
        let mut order = Self::default();
        order.remember_rows(rows);
        order
    }

    pub(super) fn remember_rows(&mut self, rows: &[SessionRow<'_>]) {
        let session_ids = rows
            .iter()
            .filter_map(SessionRow::session_id)
            .collect::<Vec<_>>();
        self.remember_session_ids(&session_ids);
    }

    pub(super) fn remember_session_ids(&mut self, session_ids: &[&str]) {
        for session_id in session_ids {
            if !self.task_positions.contains_key(*session_id) {
                self.task_positions
                    .insert((*session_id).to_string(), self.next_task_position);
                self.next_task_position += 1;
            }
        }
    }

    pub(super) fn retain_sessions(&mut self, session_ids: &HashSet<String>) {
        self.task_positions
            .retain(|session_id, _| session_ids.contains(session_id));
    }

    fn task_position(&self, session_id: &str) -> Option<usize> {
        self.task_positions.get(session_id).copied()
    }
}

/// One TASKS row. `tree_prefix` is `None` for a root; descendants carry one
/// sibling marker for each non-root ancestor between the root and parent.
#[derive(Debug, Clone)]
pub(super) struct SessionRowEntry<'a> {
    pub session: &'a Session,
    pub mark: TaskMark,
    pub vibe_mode: Option<VibeMode>,
    pub is_last_sibling: bool,
    /// Whether each non-root ancestor was the last child in its own group.
    /// `None` identifies a root; an empty prefix identifies a direct child.
    pub tree_prefix: Option<Vec<bool>>,
    /// Descendants shown under a root (always 0 on descendant rows).
    pub child_count: usize,
    pub child_user_turn_count: usize,
    /// The user-action reason attached to this session, without its note.
    pub user_turn: Option<UserTurnInfo<'a>>,
    /// The tq task this session is linked to, if any. Drives the task-number
    /// column, the row name, and cursor-relative task-kinship styling.
    pub task: Option<SessionTask>,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct UserTurnInfo<'a> {
    pub kind: UserTurnKind,
    pub label: Option<&'a str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum VibeMode {
    Explicit,
    Inherited,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TaskMark {
    Done,
    UserTurn,
    Working,
    /// Stopped while waiting on something other than the user (another
    /// session, CI, a background command).
    Waiting,
    /// Stopped without a reason this module recognizes; rendered with the
    /// session's plain status glyph instead of a task mark.
    Unclassified,
}

impl<'a> SessionRow<'a> {
    pub(super) fn row_key(&self) -> Option<SessionRowKey> {
        match self {
            SessionRow::SectionHeader(_) => None,
            SessionRow::Session(entry) => {
                Some(SessionRowKey::Task(entry.session.session_id.clone()))
            }
            SessionRow::InlineProgress(entry) => entry.selection_key.clone(),
        }
    }

    pub(super) fn session_id(&self) -> Option<&str> {
        match self {
            SessionRow::Session(entry) => Some(entry.session.session_id.as_str()),
            SessionRow::InlineProgress(entry) => entry
                .selection_key
                .as_ref()
                .and_then(SessionRowKey::session_id),
            SessionRow::SectionHeader(_) => None,
        }
    }
}

/// A tq task linked to a session, as needed for rendering the task-number
/// column and for the cursor-relative task-kinship check
/// (see [`is_related_task`]). `parent_task_id` is `None` both when the task
/// genuinely has no parent and when the running `tq` binary is too old to
/// report `parentId` at all -- either way, kinship degrades to "same task
/// only" rather than being treated as an error.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub(super) struct SessionTask {
    pub task_id: String,
    pub task_number: u32,
    pub task_title: String,
    pub parent_task_id: Option<String>,
    /// Whether the linked tq task is closed. `false` both when the task is
    /// genuinely open and when the running `tq` binary is too old to report
    /// task status at all -- either way, the task number renders as open.
    pub is_closed: bool,
}

/// Builds the TASKS session tree.
///
/// `task_by_session` attaches each row's linked tq task (see [`SessionTask`]);
/// callers can pass an empty map when tq integration is unavailable.
#[cfg(test)]
pub(super) fn build_session_rows<'a>(
    sessions: &[&'a Session],
    task_by_session: &HashMap<String, SessionTask>,
    agent_config: &AgentConfig,
) -> Vec<SessionRow<'a>> {
    build_session_rows_with_vibe(sessions, task_by_session, agent_config, &HashSet::new())
}

/// Builds session rows using explicit vibe settings from stored ancestors,
/// including ancestors that are no longer active.
pub(super) fn build_session_rows_with_vibe<'a>(
    sessions: &[&'a Session],
    task_by_session: &HashMap<String, SessionTask>,
    agent_config: &AgentConfig,
    vibe_enabled_session_ids: &HashSet<String>,
) -> Vec<SessionRow<'a>> {
    build_session_rows_with_order(
        sessions,
        task_by_session,
        agent_config,
        vibe_enabled_session_ids,
        None,
    )
}

pub(super) fn build_session_rows_with_order<'a>(
    sessions: &[&'a Session],
    task_by_session: &HashMap<String, SessionTask>,
    agent_config: &AgentConfig,
    vibe_enabled_session_ids: &HashSet<String>,
    order: Option<&SessionRowOrder>,
) -> Vec<SessionRow<'a>> {
    let turns = user_turns(sessions.iter().copied(), agent_config);
    let user_turns_by_session = turns
        .iter()
        .map(|turn| {
            (
                turn.session.session_id.as_str(),
                UserTurnInfo {
                    kind: turn.kind,
                    label: turn.label,
                },
            )
        })
        .collect();
    let mut rows = Vec::with_capacity(sessions.len() + 1);
    let task_rows = task_tree::build_task_rows(
        sessions,
        task_by_session,
        &user_turns_by_session,
        agent_config,
        vibe_enabled_session_ids,
        order,
    );
    if !task_rows.is_empty() {
        rows.push(SessionRow::SectionHeader(SectionHeaderRow {
            label: "TASKS".to_string(),
        }));
        rows.extend(task_rows.into_iter().map(SessionRow::Session));
    }

    rows
}

/// Whether `session` descends from `ancestor_id` at any depth (i.e.
/// `ancestor_id` appears anywhere in `ancestor_session_ids`). Used by the
/// drill-down scope filter (`App::apply_filter`).
pub(super) fn is_descendant_of(session: &Session, ancestor_id: &str) -> bool {
    session
        .ancestor_session_ids
        .iter()
        .any(|id| id == ancestor_id)
}

/// Finds the nearest living ancestor of a session among the sessions present
/// in `by_id` (the currently displayed ones). Walks `ancestor_session_ids`
/// from the end (nearest ancestor) to the start (root).
///
/// Used by `App::select_parent` for the "jump to parent" navigation.
pub(super) fn nearest_living_ancestor<'a>(
    session: &Session,
    by_id: &HashMap<&str, &'a Session>,
) -> Option<&'a Session> {
    session
        .ancestor_session_ids
        .iter()
        .rev()
        .find_map(|ancestor_id| by_id.get(ancestor_id.as_str()).copied())
}

/// Direction of a kin relationship relative to the cursor session: whether
/// the other session sits above it (toward the root), below it (toward the
/// leaves), or off to the side -- sharing a common ancestor with it without
/// being either -- in the family tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum KinDirection {
    Ancestor,
    Descendant,
    Collateral,
}

/// How many generations `session` sits from `selected`, and in which
/// direction: `Ancestor`/`Descendant` for a direct lineage (1 = direct
/// parent/child), or `Collateral` for a shared-ancestor relationship that is
/// neither (siblings, cousins, ...), measured by generations up to their
/// nearest common ancestor (1 = sibling, 2 = cousin). `None` only when the
/// two share no common ancestor at all, or are the same session.
///
/// Distance is derived purely from `ancestor_session_ids`, so it does not
/// depend on which intermediate ancestors happen to be displayed -- unlike
/// [`nearest_living_ancestor`], which intentionally scopes to the current
/// view.
pub(super) fn kin_relation(selected: &Session, session: &Session) -> Option<(KinDirection, usize)> {
    if selected.session_id == session.session_id {
        return None;
    }
    if let Some(pos) = session
        .ancestor_session_ids
        .iter()
        .position(|id| id == &selected.session_id)
    {
        return Some((
            KinDirection::Descendant,
            session.ancestor_session_ids.len() - pos,
        ));
    }
    if let Some(pos) = selected
        .ancestor_session_ids
        .iter()
        .position(|id| id == &session.session_id)
    {
        return Some((
            KinDirection::Ancestor,
            selected.ancestor_session_ids.len() - pos,
        ));
    }
    collateral_kin_relation(selected, session)
}

/// Collateral case of [`kin_relation`]: `selected` and `session` are
/// neither's ancestor, so their kinship is measured by how many generations
/// each sits below their nearest common ancestor. Siblings share a parent
/// (1 generation below it on both sides); cousins share a grandparent (2
/// generations below). When the two sit at different depths below the
/// common ancestor (e.g. aunt/uncle vs. niece/nephew), the farther side sets
/// the distance. `None` if the two share no common ancestor.
fn collateral_kin_relation(selected: &Session, session: &Session) -> Option<(KinDirection, usize)> {
    let common_ancestor_depth = selected
        .ancestor_session_ids
        .iter()
        .zip(session.ancestor_session_ids.iter())
        .take_while(|(a, b)| a == b)
        .count();
    if common_ancestor_depth == 0 {
        return None;
    }
    let selected_distance = selected.ancestor_session_ids.len() - common_ancestor_depth + 1;
    let session_distance = session.ancestor_session_ids.len() - common_ancestor_depth + 1;
    Some((
        KinDirection::Collateral,
        selected_distance.max(session_distance),
    ))
}

/// Whether `other`'s task should read as "related" to `cursor`'s task, for
/// the task-number brightness. Unlike [`kin_relation`]'s graded distance,
/// task kinship is deliberately binary: the same task, or a direct
/// parent/child task -- not any ancestor/descendant generation beyond that.
/// `false` whenever either side has no task (nothing to relate), including
/// when `parent_task_id` is unknown because the local `tq` binary predates
/// that field -- kinship then degrades to "same task only".
pub(super) fn is_related_task(cursor: Option<&SessionTask>, other: Option<&SessionTask>) -> bool {
    let (Some(cursor), Some(other)) = (cursor, other) else {
        return false;
    };
    cursor.task_id == other.task_id
        || cursor.parent_task_id.as_deref() == Some(other.task_id.as_str())
        || other.parent_task_id.as_deref() == Some(cursor.task_id.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::agent::types::{AgentStatus, Engine, SessionStatus};
    use crate::commands::agent::user_turn::UserTurnKind;
    use crate::shared::config::AgentConfig;
    use crate::shared::config::test_support::agent_config;
    use chrono::{DateTime, TimeZone, Utc};
    use rstest::rstest;
    use std::{collections::HashMap, path::PathBuf};

    fn at_minute(minute: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 1, 1, 0, minute, 0)
            .single()
            .unwrap()
    }

    fn create_test_session(id: &str, status: SessionStatus) -> Session {
        Session {
            archived_at: None,
            pr_context: None,
            session_id: id.to_string(),
            work_type: None,
            work_type_pinned: false,
            vibe_enabled: false,
            launched_with_vibe: false,
            child_launch: Default::default(),
            crit_urls: Vec::new(),
            pending_human_review_ids: Default::default(),
            cwd: PathBuf::from("/home/user/project"),
            transcript_path: None,
            tty: None,
            tmux_info: None,
            status,
            created_at: at_minute(0),
            updated_at: at_minute(0),
            last_message: None,
            current_tool: None,
            label: None,
            ancestor_session_ids: Vec::new(),
            pending_bg_task_ids: std::collections::BTreeSet::new(),
            pending_agent_task_ids: std::collections::BTreeSet::new(),
            pending_permission_agent_ids: std::collections::BTreeSet::new(),
            pending_permission_request_ids: Default::default(),
            read_at: None,
            sweep_signaled: false,
            stop_blocked_by_armyknife: false,
            agent_status: None,
            agent_status_clear_pending: false,
            engine: Engine::Claude,
            model: None,
            effort: None,
        }
    }

    fn running_session(id: &str, ancestors: &[&str], minute: u32) -> Session {
        let mut session = create_test_session(id, SessionStatus::Running);
        session.ancestor_session_ids = ancestors.iter().map(|s| s.to_string()).collect();
        session.updated_at = at_minute(minute);
        session
    }

    fn with_agent_status(mut session: Session, kind: &str) -> Session {
        session.status = SessionStatus::Stopped;
        session.agent_status = Some(AgentStatus {
            kind: kind.to_string(),
            note: "sample note".to_string(),
        });
        session
    }

    fn task_ref(id: &str, parent_id: Option<&str>) -> SessionTask {
        SessionTask {
            task_id: id.to_string(),
            task_number: 1,
            task_title: "Task".to_string(),
            parent_task_id: parent_id.map(String::from),
            is_closed: false,
        }
    }

    /// Maps rows to the subset of fields relevant to these tests, so each
    /// test can assert on one derived value instead of matching on raw
    /// `SessionRow` variants (which are not `PartialEq`-friendly).
    #[derive(Debug, PartialEq)]
    enum RowDescription {
        Header {
            label: String,
        },
        Task {
            id: String,
            mark: TaskMark,
            is_last_sibling: bool,
            tree_prefix: Option<Vec<bool>>,
            child_count: usize,
            child_user_turn_count: usize,
            user_turn: Option<(UserTurnKind, Option<String>)>,
        },
        InlineProgress {
            label: Option<String>,
            selection_key: Option<SessionRowKey>,
            session_id: Option<String>,
        },
    }

    fn describe(rows: &[SessionRow]) -> Vec<RowDescription> {
        rows.iter()
            .map(|row| match row {
                SessionRow::SectionHeader(header) => RowDescription::Header {
                    label: header.label.clone(),
                },
                SessionRow::Session(entry) => RowDescription::Task {
                    id: entry.session.session_id.clone(),
                    mark: entry.mark,
                    is_last_sibling: entry.is_last_sibling,
                    tree_prefix: entry.tree_prefix.clone(),
                    child_count: entry.child_count,
                    child_user_turn_count: entry.child_user_turn_count,
                    user_turn: entry
                        .user_turn
                        .map(|turn| (turn.kind, turn.label.map(str::to_owned))),
                },
                SessionRow::InlineProgress(entry) => RowDescription::InlineProgress {
                    label: entry.label.clone(),
                    selection_key: entry.selection_key.clone(),
                    session_id: entry
                        .session
                        .as_ref()
                        .map(|entry| entry.session.session_id.clone()),
                },
            })
            .collect()
    }

    fn header(label: &str) -> RowDescription {
        RowDescription::Header {
            label: label.to_string(),
        }
    }

    fn root_row(id: &str, mark: TaskMark, children: usize, user_turns: usize) -> RowDescription {
        RowDescription::Task {
            id: id.to_string(),
            mark,
            is_last_sibling: false,
            tree_prefix: None,
            child_count: children,
            child_user_turn_count: user_turns,
            user_turn: None,
        }
    }

    fn user_turn_root_row(id: &str, kind: UserTurnKind, label: Option<&str>) -> RowDescription {
        RowDescription::Task {
            id: id.to_string(),
            mark: TaskMark::UserTurn,
            is_last_sibling: false,
            tree_prefix: None,
            child_count: 0,
            child_user_turn_count: 0,
            user_turn: Some((kind, label.map(str::to_owned))),
        }
    }

    fn child_row(id: &str, mark: TaskMark, is_last_sibling: bool) -> RowDescription {
        RowDescription::Task {
            id: id.to_string(),
            mark,
            is_last_sibling,
            tree_prefix: Some(Vec::new()),
            child_count: 0,
            child_user_turn_count: 0,
            user_turn: None,
        }
    }

    fn nested_row(
        id: &str,
        mark: TaskMark,
        is_last_sibling: bool,
        tree_prefix: &[bool],
    ) -> RowDescription {
        RowDescription::Task {
            id: id.to_string(),
            mark,
            is_last_sibling,
            tree_prefix: Some(tree_prefix.to_vec()),
            child_count: 0,
            child_user_turn_count: 0,
            user_turn: None,
        }
    }

    fn user_turn_nested_row(
        id: &str,
        is_last_sibling: bool,
        tree_prefix: &[bool],
        kind: UserTurnKind,
        label: Option<&str>,
    ) -> RowDescription {
        RowDescription::Task {
            id: id.to_string(),
            mark: TaskMark::UserTurn,
            is_last_sibling,
            tree_prefix: Some(tree_prefix.to_vec()),
            child_count: 0,
            child_user_turn_count: 0,
            user_turn: Some((kind, label.map(str::to_owned))),
        }
    }

    #[rstest]
    fn test_build_session_rows_empty_input(agent_config: AgentConfig) {
        let rows = build_session_rows(&[], &HashMap::new(), &agent_config);
        assert_eq!(describe(&rows), Vec::<RowDescription>::new());
    }

    #[rstest]
    fn test_build_session_rows_places_user_turn_reason_on_tasks_row(agent_config: AgentConfig) {
        let mut crit = create_test_session("crit", SessionStatus::WaitingInput);
        crit.crit_urls
            .push("https://example.com/review".to_string());
        let running = running_session("running", &[], 0);

        let sessions: Vec<&Session> = vec![&running, &crit];
        let rows = build_session_rows(&sessions, &HashMap::new(), &agent_config);

        assert_eq!(
            describe(&rows),
            vec![
                header("TASKS"),
                user_turn_root_row("crit", UserTurnKind::Crit, Some("crit")),
                root_row("running", TaskMark::Working, 0, 0),
            ]
        );
    }

    #[rstest]
    fn test_build_session_rows_preserves_full_depth_for_all_child_sessions(
        agent_config: AgentConfig,
    ) {
        let pm = running_session("pm", &[], 0);
        let design = running_session("design", &["pm"], 3);
        // The intermediate ancestor is gone, so this still nests under "pm".
        let orphan = running_session("orphan", &["pm", "deleted_middle"], 1);
        let grandchild = running_session("grandchild", &["pm", "design"], 2);
        let great_grandchild =
            running_session("great_grandchild", &["pm", "design", "grandchild"], 6);
        let qa = running_session("qa", &["pm", "design"], 0);
        let mut delegated = running_session("delegated", &["pm", "design"], 4);
        delegated.child_launch = crate::commands::agent::types::ChildLaunchPermission::Deny;
        let delegated_child = with_agent_status(
            running_session("delegated_child", &["pm", "design", "delegated"], 5),
            "needs_attention",
        );

        let sessions: Vec<&Session> = vec![
            &pm,
            &design,
            &orphan,
            &grandchild,
            &great_grandchild,
            &qa,
            &delegated,
            &delegated_child,
        ];
        let rows = build_session_rows(&sessions, &HashMap::new(), &agent_config);

        assert_eq!(
            describe(&rows),
            vec![
                header("TASKS"),
                root_row("pm", TaskMark::Working, 7, 1),
                child_row("design", TaskMark::Working, false),
                nested_row("delegated", TaskMark::Working, false, &[false]),
                user_turn_nested_row(
                    "delegated_child",
                    true,
                    &[false, false],
                    UserTurnKind::AgentStatus,
                    Some("needs_attention"),
                ),
                nested_row("grandchild", TaskMark::Working, false, &[false]),
                nested_row("great_grandchild", TaskMark::Working, true, &[false, false],),
                nested_row("qa", TaskMark::Working, true, &[false]),
                child_row("orphan", TaskMark::Working, true),
            ]
        );
    }

    #[rstest]
    fn test_build_session_rows_keeps_sessions_with_inconsistent_ancestor_chains(
        agent_config: AgentConfig,
    ) {
        let root = running_session("root", &[], 3);
        // "parent" was re-parented under "root" after "child" recorded it
        // as its own root.
        let parent = running_session("parent", &["root"], 2);
        let child = running_session("child", &["parent"], 1);
        let cycle_a = running_session("cycle_a", &["cycle_b"], 0);
        let cycle_b = running_session("cycle_b", &["cycle_a"], 0);

        let sessions: Vec<&Session> = vec![&root, &parent, &child, &cycle_a, &cycle_b];
        let rows = build_session_rows(&sessions, &HashMap::new(), &agent_config);

        assert_eq!(
            describe(&rows),
            vec![
                header("TASKS"),
                root_row("root", TaskMark::Working, 2, 0),
                child_row("parent", TaskMark::Working, true),
                nested_row("child", TaskMark::Working, true, &[true]),
                root_row("cycle_a", TaskMark::Working, 1, 0),
                child_row("cycle_b", TaskMark::Working, true),
            ]
        );
    }

    #[rstest]
    fn test_build_session_rows_orders_roots_by_user_turn_then_last_moved(
        agent_config: AgentConfig,
    ) {
        let old_turn = with_agent_status(running_session("old_turn", &[], 1), "needs_attention");
        let recent = running_session("recent", &[], 9);
        let older = running_session("older", &[], 2);
        // A child's activity counts as its root moving.
        let older_child = running_session("older_child", &["older"], 10);
        let parent_of_turn = running_session("parent_of_turn", &[], 0);
        let child_turn = with_agent_status(
            running_session("child_turn", &["parent_of_turn"], 3),
            "needs_attention",
        );

        let sessions: Vec<&Session> = vec![
            &old_turn,
            &recent,
            &older,
            &older_child,
            &parent_of_turn,
            &child_turn,
        ];
        let rows = build_session_rows(&sessions, &HashMap::new(), &agent_config);
        let task_ids: Vec<&str> = rows
            .iter()
            .filter_map(|row| match row {
                SessionRow::Session(entry) => Some(entry.session.session_id.as_str()),
                _ => None,
            })
            .collect();

        assert_eq!(
            task_ids,
            vec![
                "parent_of_turn",
                "child_turn",
                "old_turn",
                "older",
                "older_child",
                "recent",
            ]
        );
    }

    #[rstest]
    #[case::running(SessionStatus::Running, None, false, TaskMark::Working)]
    #[case::background(SessionStatus::Stopped, None, true, TaskMark::Waiting)]
    #[case::agent_wait(
        SessionStatus::Stopped,
        Some("awaiting_build"),
        false,
        TaskMark::Waiting
    )]
    #[case::agent_done(SessionStatus::Paused, Some("finished_work"), false, TaskMark::Done)]
    #[case::stale_status_while_running(
        SessionStatus::Running,
        Some("awaiting_build"),
        false,
        TaskMark::Working
    )]
    #[case::ended(SessionStatus::Ended, None, false, TaskMark::Done)]
    #[case::you(
        SessionStatus::Stopped,
        Some("needs_attention"),
        false,
        TaskMark::UserTurn
    )]
    #[case::failed(SessionStatus::Failed, None, false, TaskMark::UserTurn)]
    #[case::unknown_status_is_user(
        SessionStatus::Stopped,
        Some("removed_name"),
        false,
        TaskMark::UserTurn
    )]
    #[case::stopped_without_reason(SessionStatus::Stopped, None, false, TaskMark::Unclassified)]
    fn test_build_session_rows_task_mark(
        agent_config: AgentConfig,
        #[case] status: SessionStatus,
        #[case] agent_status: Option<&str>,
        #[case] has_bg_task: bool,
        #[case] expected: TaskMark,
    ) {
        let mut session = create_test_session("s", status);
        session.agent_status = agent_status.map(|kind| AgentStatus {
            kind: kind.to_string(),
            note: "sample note".to_string(),
        });
        if has_bg_task {
            session.pending_bg_task_ids.insert("bg-1".to_string());
        }

        let rows = build_session_rows(&[&session], &HashMap::new(), &agent_config);
        let marks: Vec<TaskMark> = rows
            .iter()
            .filter_map(|row| match row {
                SessionRow::Session(entry) => Some(entry.mark),
                _ => None,
            })
            .collect();

        assert_eq!(marks, vec![expected]);
    }

    #[rstest]
    #[case::linked_session_gets_its_task(
        HashMap::from([("a".to_string(), task_ref("t1", None))]),
        Some(task_ref("t1", None))
    )]
    #[case::unlinked_session_has_no_task(HashMap::new(), None)]
    fn test_build_session_rows_attaches_task_from_lookup(
        agent_config: AgentConfig,
        #[case] task_by_session: HashMap<String, SessionTask>,
        #[case] expected: Option<SessionTask>,
    ) {
        let session = create_test_session("a", SessionStatus::Running);
        let sessions: Vec<&Session> = vec![&session];
        let rows = build_session_rows(&sessions, &task_by_session, &agent_config);

        let task = rows.iter().find_map(|r| match r {
            SessionRow::Session(entry) => Some(entry.task.clone()),
            SessionRow::SectionHeader(_) | SessionRow::InlineProgress(_) => None,
        });
        assert_eq!(task, Some(expected));
    }

    #[rstest]
    #[case::same_task(Some(task_ref("t1", None)), Some(task_ref("t1", None)), true)]
    #[case::cursor_is_parent_of_other(
        Some(task_ref("t1", None)),
        Some(task_ref("t2", Some("t1"))),
        true
    )]
    #[case::cursor_is_child_of_other(
        Some(task_ref("t2", Some("t1"))),
        Some(task_ref("t1", None)),
        true
    )]
    #[case::unrelated_tasks(Some(task_ref("t1", None)), Some(task_ref("t2", None)), false)]
    #[case::cursor_has_no_task(None, Some(task_ref("t1", None)), false)]
    #[case::other_has_no_task(Some(task_ref("t1", None)), None, false)]
    fn test_is_related_task(
        #[case] cursor: Option<SessionTask>,
        #[case] other: Option<SessionTask>,
        #[case] expected: bool,
    ) {
        assert_eq!(is_related_task(cursor.as_ref(), other.as_ref()), expected);
    }

    #[test]
    fn test_kin_relation_same_session_is_none() {
        let session = create_test_session("s", SessionStatus::Running);
        assert_eq!(kin_relation(&session, &session), None);
    }

    #[test]
    fn test_kin_relation_completely_unrelated_sessions_is_none() {
        let mut a = create_test_session("a", SessionStatus::Running);
        a.ancestor_session_ids = vec!["root_a".to_string()];
        let mut b = create_test_session("b", SessionStatus::Running);
        b.ancestor_session_ids = vec!["root_b".to_string()];

        assert_eq!(kin_relation(&a, &b), None);
    }

    fn ancestor_path(branch: &str, generations: usize) -> Vec<String> {
        let mut ids = vec!["shared_root".to_string()];
        for i in 1..generations {
            ids.push(format!("{branch}{i}"));
        }
        ids
    }

    #[rstest]
    #[case::siblings(1)]
    #[case::cousins(2)]
    #[case::second_cousins(3)]
    fn test_kin_relation_collateral_distance(#[case] generations: usize) {
        // `selected` and `session` diverge onto separate branches right
        // after their shared root ancestor, so their nearest common
        // ancestor sits `generations` levels above each of them.
        let mut selected = create_test_session("selected", SessionStatus::Running);
        selected.ancestor_session_ids = ancestor_path("a", generations);
        let mut session = create_test_session("session", SessionStatus::Running);
        session.ancestor_session_ids = ancestor_path("b", generations);

        assert_eq!(
            kin_relation(&selected, &session),
            Some((KinDirection::Collateral, generations))
        );
    }

    #[test]
    fn test_kin_relation_collateral_distance_uses_the_farther_side() {
        // `aunt` is a direct child of the shared grandparent (1 generation
        // below it); `niece` is her sibling's child (2 generations below
        // it). The relationship is bounded by the farther side, so it
        // reads as distance 2, not 1.
        let mut aunt = create_test_session("aunt", SessionStatus::Running);
        aunt.ancestor_session_ids = vec!["grandparent".to_string()];
        let mut niece = create_test_session("niece", SessionStatus::Running);
        niece.ancestor_session_ids = vec!["grandparent".to_string(), "parent".to_string()];

        assert_eq!(
            kin_relation(&niece, &aunt),
            Some((KinDirection::Collateral, 2))
        );
    }

    #[rstest]
    #[case::direct_parent(1)]
    #[case::grandparent(2)]
    #[case::great_grandparent(3)]
    fn test_kin_relation_ancestor_distance(#[case] generations: usize) {
        // `selected` is `generations` hops below `ancestor` in the tree;
        // from `selected`'s cursor, `ancestor` should read as an Ancestor at
        // that same distance.
        let ancestor_ids: Vec<String> = (0..generations).map(|i| format!("gen{i}")).collect();
        let ancestor = create_test_session(&ancestor_ids[0], SessionStatus::Running);
        let mut selected = create_test_session("selected", SessionStatus::Running);
        selected.ancestor_session_ids = ancestor_ids;

        assert_eq!(
            kin_relation(&selected, &ancestor),
            Some((KinDirection::Ancestor, generations))
        );
    }

    #[rstest]
    #[case::direct_child(1)]
    #[case::grandchild(2)]
    #[case::great_grandchild(3)]
    fn test_kin_relation_descendant_distance(#[case] generations: usize) {
        // `descendant` is `generations` hops below `selected`; from
        // `selected`'s cursor, `descendant` should read as a Descendant at
        // that same distance.
        let mut ancestor_ids = vec!["selected".to_string()];
        for i in 1..generations {
            ancestor_ids.push(format!("gen{i}"));
        }
        let selected = create_test_session("selected", SessionStatus::Running);
        let mut descendant = create_test_session("descendant", SessionStatus::Running);
        descendant.ancestor_session_ids = ancestor_ids;

        assert_eq!(
            kin_relation(&selected, &descendant),
            Some((KinDirection::Descendant, generations))
        );
    }

    #[test]
    fn test_kin_relation_survives_deleted_intermediate_ancestor() {
        // Distance is computed from the raw `ancestor_session_ids` array, so
        // it must not depend on whether intermediate ancestors are actually
        // displayed (that's `nearest_living_ancestor`'s job, not this one's).
        let root = create_test_session("root", SessionStatus::Running);
        let mut leaf = create_test_session("leaf", SessionStatus::Running);
        leaf.ancestor_session_ids = vec!["root".to_string(), "deleted_middle".to_string()];

        assert_eq!(
            kin_relation(&root, &leaf),
            Some((KinDirection::Descendant, 2))
        );
        assert_eq!(
            kin_relation(&leaf, &root),
            Some((KinDirection::Ancestor, 2))
        );
    }
}
