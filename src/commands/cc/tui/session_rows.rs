use std::collections::{HashMap, HashSet};

use crate::commands::cc::types::{Session, SessionStatus};

/// One row in the session list's selection/render order.
///
/// Section headers are not individually selectable; only `Session` rows are
/// (see [`SessionRow::session_id`]).
#[derive(Debug)]
pub(super) enum SessionRow<'a> {
    SectionHeader(SectionHeaderRow),
    Session(SessionRowEntry<'a>),
}

#[derive(Debug)]
pub(super) struct SectionHeaderRow {
    pub label: String,
    /// Which section this header represents, so the renderer can color it
    /// consistently with that section's status (e.g. amber for NEEDS YOU).
    pub kind: Section,
}

#[derive(Debug)]
pub(super) struct SessionRowEntry<'a> {
    pub session: &'a Session,
    /// Nearest living ancestor among the currently displayed sessions, for
    /// the `parent title › ` breadcrumb prefix. `None` for root sessions or
    /// when every ancestor has been filtered out of the current view.
    pub breadcrumb_ancestor: Option<&'a Session>,
}

impl<'a> SessionRow<'a> {
    /// The session_id occupying this row, if it is individually selectable.
    pub(super) fn session_id(&self) -> Option<&str> {
        match self {
            SessionRow::Session(entry) => Some(entry.session.session_id.as_str()),
            SessionRow::SectionHeader(_) => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Section {
    NeedsYou,
    Running,
    Unread,
    Idle,
}

fn section_of(session: &Session) -> Section {
    if session.status == SessionStatus::WaitingInput {
        Section::NeedsYou
    } else if session.status == SessionStatus::Running {
        Section::Running
    } else if session.is_unread_stopped() {
        Section::Unread
    } else {
        Section::Idle
    }
}

/// Whether `session` sits in the last section (Paused, read Stopped, Ended).
/// Exposed for the renderer's idle-styling decision, so the "which statuses
/// count as idle" rule lives in one place.
pub(super) fn is_idle_session(session: &Session) -> bool {
    matches!(section_of(session), Section::Idle)
}

/// Label for the last section, based on which idle statuses are actually
/// present. Ended sessions are rare in practice (retained briefly for
/// `claude -c` resume) and fall under "STOPPED" alongside read Stopped
/// sessions.
fn idle_section_label(idle: &[&Session]) -> &'static str {
    let has_paused = idle.iter().any(|s| s.status == SessionStatus::Paused);
    let has_other = idle.iter().any(|s| s.status != SessionStatus::Paused);
    match (has_paused, has_other) {
        (true, false) => "PAUSED",
        (false, true) => "STOPPED",
        _ => "PAUSED / STOPPED",
    }
}

/// Builds the session list's section-grouped row order.
///
/// Sections appear in a fixed order and are omitted entirely (no header) when
/// empty: NEEDS YOU (waiting) -> RUNNING -> UNREAD (unread stopped) ->
/// PAUSED/STOPPED. Within a section, sessions keep the relative order of
/// `sessions` (the caller is expected to pass them already time-sorted).
pub(super) fn build_session_rows<'a>(sessions: &[&'a Session]) -> Vec<SessionRow<'a>> {
    let by_id: HashMap<&str, &'a Session> = sessions
        .iter()
        .map(|s| (s.session_id.as_str(), *s))
        .collect();
    let displayed_ids: HashSet<&str> = by_id.keys().copied().collect();

    let mut needs_you = Vec::new();
    let mut running = Vec::new();
    let mut unread = Vec::new();
    let mut idle = Vec::new();
    for &session in sessions {
        match section_of(session) {
            Section::NeedsYou => needs_you.push(session),
            Section::Running => running.push(session),
            Section::Unread => unread.push(session),
            Section::Idle => idle.push(session),
        }
    }

    let mut rows = Vec::with_capacity(sessions.len() + 4);
    push_group(
        &mut rows,
        "NEEDS YOU".to_string(),
        Section::NeedsYou,
        &needs_you,
        &by_id,
        &displayed_ids,
    );
    push_group(
        &mut rows,
        format!("RUNNING ({})", running.len()),
        Section::Running,
        &running,
        &by_id,
        &displayed_ids,
    );
    push_group(
        &mut rows,
        format!("UNREAD ({})", unread.len()),
        Section::Unread,
        &unread,
        &by_id,
        &displayed_ids,
    );

    let idle_label = format!("{} ({})", idle_section_label(&idle), idle.len());
    push_group(
        &mut rows,
        idle_label,
        Section::Idle,
        &idle,
        &by_id,
        &displayed_ids,
    );

    rows
}

fn push_group<'a>(
    rows: &mut Vec<SessionRow<'a>>,
    label: String,
    kind: Section,
    group: &[&'a Session],
    by_id: &HashMap<&str, &'a Session>,
    displayed_ids: &HashSet<&str>,
) {
    if group.is_empty() {
        return;
    }
    rows.push(SessionRow::SectionHeader(SectionHeaderRow { label, kind }));
    for &session in group {
        rows.push(SessionRow::Session(SessionRowEntry {
            session,
            breadcrumb_ancestor: nearest_living_ancestor(session, by_id, displayed_ids),
        }));
    }
}

/// Finds the nearest living ancestor of a session among the displayed
/// sessions. Walks `ancestor_session_ids` from the end (nearest ancestor) to
/// the start (root).
///
/// Shared with `App::select_parent`, so the "jump to parent" navigation
/// always lands on the same session the breadcrumb prefix names.
pub(super) fn nearest_living_ancestor<'a>(
    session: &Session,
    by_id: &HashMap<&str, &'a Session>,
    displayed_ids: &HashSet<&str>,
) -> Option<&'a Session> {
    for ancestor_id in session.ancestor_session_ids.iter().rev() {
        if displayed_ids.contains(ancestor_id.as_str()) {
            return by_id.get(ancestor_id.as_str()).copied();
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use rstest::rstest;
    use std::path::PathBuf;

    fn create_test_session(id: &str, status: SessionStatus) -> Session {
        Session {
            session_id: id.to_string(),
            cwd: PathBuf::from("/home/user/project"),
            transcript_path: None,
            tty: None,
            tmux_info: None,
            status,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            last_message: None,
            current_tool: None,
            label: None,
            ancestor_session_ids: Vec::new(),
            pending_bg_task_ids: std::collections::BTreeSet::new(),
            pending_agent_task_ids: std::collections::BTreeSet::new(),
            read_at: None,
            sweep_signaled: false,
        }
    }

    /// A read (non-unread) Stopped session: `read_at` set so it does not
    /// land in UNREAD.
    fn create_read_stopped_session(id: &str) -> Session {
        let mut session = create_test_session(id, SessionStatus::Stopped);
        session.read_at = Some(Utc::now());
        session
    }

    /// Maps rows to the subset of fields relevant to these tests, so each
    /// test can assert on one derived value instead of matching on raw
    /// `SessionRow` variants (which are not `PartialEq`-friendly).
    #[derive(Debug, PartialEq)]
    enum RowDescription {
        Header {
            label: String,
        },
        Session {
            id: String,
            breadcrumb: Option<String>,
        },
    }

    fn describe(rows: &[SessionRow]) -> Vec<RowDescription> {
        rows.iter()
            .map(|row| match row {
                SessionRow::SectionHeader(header) => RowDescription::Header {
                    label: header.label.clone(),
                },
                SessionRow::Session(entry) => RowDescription::Session {
                    id: entry.session.session_id.clone(),
                    breadcrumb: entry.breadcrumb_ancestor.map(|s| s.session_id.clone()),
                },
            })
            .collect()
    }

    #[test]
    fn test_build_session_rows_empty_input() {
        let rows = build_session_rows(&[]);
        assert_eq!(describe(&rows), Vec::<RowDescription>::new());
    }

    #[test]
    fn test_build_session_rows_one_session_per_section() {
        let waiting = create_test_session("waiting", SessionStatus::WaitingInput);
        let running = create_test_session("running", SessionStatus::Running);
        let mut unread = create_test_session("unread", SessionStatus::Stopped);
        unread.read_at = None;
        let paused = create_test_session("paused", SessionStatus::Paused);

        let sessions: Vec<&Session> = vec![&waiting, &running, &unread, &paused];
        let rows = build_session_rows(&sessions);

        assert_eq!(
            describe(&rows),
            vec![
                RowDescription::Header {
                    label: "NEEDS YOU".to_string(),
                },
                RowDescription::Session {
                    id: "waiting".to_string(),
                    breadcrumb: None,
                },
                RowDescription::Header {
                    label: "RUNNING (1)".to_string(),
                },
                RowDescription::Session {
                    id: "running".to_string(),
                    breadcrumb: None,
                },
                RowDescription::Header {
                    label: "UNREAD (1)".to_string(),
                },
                RowDescription::Session {
                    id: "unread".to_string(),
                    breadcrumb: None,
                },
                RowDescription::Header {
                    label: "PAUSED (1)".to_string(),
                },
                RowDescription::Session {
                    id: "paused".to_string(),
                    breadcrumb: None,
                },
            ]
        );
    }

    #[test]
    fn test_build_session_rows_empty_sections_produce_no_header() {
        let running = create_test_session("running", SessionStatus::Running);
        let sessions: Vec<&Session> = vec![&running];
        let rows = build_session_rows(&sessions);

        assert_eq!(
            describe(&rows),
            vec![
                RowDescription::Header {
                    label: "RUNNING (1)".to_string(),
                },
                RowDescription::Session {
                    id: "running".to_string(),
                    breadcrumb: None,
                },
            ]
        );
    }

    #[rstest]
    #[case::paused_only(
        vec![SessionStatus::Paused, SessionStatus::Paused],
        "PAUSED (2)"
    )]
    #[case::stopped_only(
        vec![SessionStatus::Ended],
        "STOPPED (1)"
    )]
    #[case::mixed(
        vec![SessionStatus::Paused, SessionStatus::Ended],
        "PAUSED / STOPPED (2)"
    )]
    fn test_idle_section_label(#[case] statuses: Vec<SessionStatus>, #[case] expected: &str) {
        let sessions: Vec<Session> = statuses
            .into_iter()
            .enumerate()
            .map(|(i, status)| {
                if status == SessionStatus::Stopped {
                    create_read_stopped_session(&i.to_string())
                } else {
                    create_test_session(&i.to_string(), status)
                }
            })
            .collect();
        let refs: Vec<&Session> = sessions.iter().collect();
        let rows = build_session_rows(&refs);

        let header_label = match rows.first() {
            Some(SessionRow::SectionHeader(header)) => header.label.clone(),
            _ => panic!("expected a section header row"),
        };
        assert_eq!(header_label, expected);
    }

    #[test]
    fn test_build_session_rows_idle_section_shows_individual_rows() {
        let paused1 = create_test_session("paused1", SessionStatus::Paused);
        let paused2 = create_test_session("paused2", SessionStatus::Paused);
        let sessions: Vec<&Session> = vec![&paused1, &paused2];
        let rows = build_session_rows(&sessions);

        assert_eq!(
            describe(&rows),
            vec![
                RowDescription::Header {
                    label: "PAUSED (2)".to_string(),
                },
                RowDescription::Session {
                    id: "paused1".to_string(),
                    breadcrumb: None,
                },
                RowDescription::Session {
                    id: "paused2".to_string(),
                    breadcrumb: None,
                },
            ]
        );
    }

    #[test]
    fn test_build_session_rows_breadcrumb_skips_deleted_ancestor() {
        let root = create_test_session("root", SessionStatus::Running);
        let mut child = create_test_session("child", SessionStatus::Running);
        child.ancestor_session_ids = vec!["root".to_string(), "deleted_middle".to_string()];

        let sessions: Vec<&Session> = vec![&root, &child];
        let rows = build_session_rows(&sessions);

        assert_eq!(
            describe(&rows),
            vec![
                RowDescription::Header {
                    label: "RUNNING (2)".to_string(),
                },
                RowDescription::Session {
                    id: "root".to_string(),
                    breadcrumb: None,
                },
                RowDescription::Session {
                    id: "child".to_string(),
                    breadcrumb: Some("root".to_string()),
                },
            ]
        );
    }

    #[test]
    fn test_build_session_rows_section_order_ignores_input_order() {
        let paused = create_test_session("paused", SessionStatus::Paused);
        let unread = create_test_session("unread", SessionStatus::Stopped);
        let running = create_test_session("running", SessionStatus::Running);
        let waiting = create_test_session("waiting", SessionStatus::WaitingInput);

        // Deliberately out of the fixed section order.
        let sessions: Vec<&Session> = vec![&paused, &unread, &running, &waiting];
        let rows = build_session_rows(&sessions);

        let session_ids: Vec<&str> = rows.iter().filter_map(|r| r.session_id()).collect();
        assert_eq!(session_ids, vec!["waiting", "running", "unread", "paused"]);
    }

    #[test]
    fn test_build_session_rows_within_section_order_matches_input() {
        let running_b = create_test_session("running_b", SessionStatus::Running);
        let running_a = create_test_session("running_a", SessionStatus::Running);

        // Input order is "b then a"; the function must not re-sort.
        let sessions: Vec<&Session> = vec![&running_b, &running_a];
        let rows = build_session_rows(&sessions);

        let session_ids: Vec<&str> = rows.iter().filter_map(|r| r.session_id()).collect();
        assert_eq!(session_ids, vec!["running_b", "running_a"]);
    }
}
