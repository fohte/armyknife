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
    /// Number of currently displayed sessions that descend from this one
    /// (any depth, not just direct children -- see [`descendant_counts`]).
    /// Drives the `▸{n}` badge; `0` means no badge.
    pub descendant_count: usize,
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
    let descendant_counts = descendant_counts(sessions, &displayed_ids);

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
        &descendant_counts,
    );
    push_group(
        &mut rows,
        format!("RUNNING ({})", running.len()),
        Section::Running,
        &running,
        &by_id,
        &descendant_counts,
    );
    push_group(
        &mut rows,
        format!("UNREAD ({})", unread.len()),
        Section::Unread,
        &unread,
        &by_id,
        &descendant_counts,
    );

    let idle_label = format!("{} ({})", idle_section_label(&idle), idle.len());
    push_group(
        &mut rows,
        idle_label,
        Section::Idle,
        &idle,
        &by_id,
        &descendant_counts,
    );

    rows
}

fn push_group<'a>(
    rows: &mut Vec<SessionRow<'a>>,
    label: String,
    kind: Section,
    group: &[&'a Session],
    by_id: &HashMap<&str, &'a Session>,
    descendant_counts: &HashMap<String, usize>,
) {
    if group.is_empty() {
        return;
    }
    rows.push(SessionRow::SectionHeader(SectionHeaderRow { label, kind }));
    for &session in group {
        rows.push(SessionRow::Session(SessionRowEntry {
            session,
            breadcrumb_ancestor: nearest_living_ancestor(session, by_id),
            descendant_count: descendant_counts
                .get(session.session_id.as_str())
                .copied()
                .unwrap_or(0),
        }));
    }
}

/// Counts, for every displayed session, how many other displayed sessions
/// carry its ID anywhere in `ancestor_session_ids` -- i.e. every displayed
/// descendant at any depth, not just direct children. This lets a badge
/// count survive an intermediate session being Ended or deleted, and only
/// ever counts sessions actually visible in the current (filtered) view, so
/// the number always matches what's on screen.
fn descendant_counts(
    sessions: &[&Session],
    displayed_ids: &HashSet<&str>,
) -> HashMap<String, usize> {
    let mut counts: HashMap<String, usize> = HashMap::new();
    for &ancestor_id in displayed_ids {
        let count = sessions
            .iter()
            .filter(|s| is_descendant_of(s, ancestor_id))
            .count();
        if count > 0 {
            counts.insert(ancestor_id.to_string(), count);
        }
    }
    counts
}

/// Whether `session` descends from `ancestor_id` at any depth (i.e.
/// `ancestor_id` appears anywhere in `ancestor_session_ids`). Shared by the
/// descendant-count badge above and the drill-down scope filter
/// (`App::apply_filter`), so the two never disagree on what counts as a
/// descendant.
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
/// Shared with `App::select_parent`, so the "jump to parent" navigation
/// always lands on the same session the breadcrumb prefix names.
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
/// [`nearest_living_ancestor`] and [`descendant_counts`], which intentionally
/// scope to the current view.
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
            pending_permission_agent_ids: std::collections::BTreeSet::new(),
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
            descendant_count: usize,
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
                    descendant_count: entry.descendant_count,
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
                    descendant_count: 0,
                },
                RowDescription::Header {
                    label: "RUNNING (1)".to_string(),
                },
                RowDescription::Session {
                    id: "running".to_string(),
                    breadcrumb: None,
                    descendant_count: 0,
                },
                RowDescription::Header {
                    label: "UNREAD (1)".to_string(),
                },
                RowDescription::Session {
                    id: "unread".to_string(),
                    breadcrumb: None,
                    descendant_count: 0,
                },
                RowDescription::Header {
                    label: "PAUSED (1)".to_string(),
                },
                RowDescription::Session {
                    id: "paused".to_string(),
                    breadcrumb: None,
                    descendant_count: 0,
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
                    descendant_count: 0,
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
                    descendant_count: 0,
                },
                RowDescription::Session {
                    id: "paused2".to_string(),
                    breadcrumb: None,
                    descendant_count: 0,
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
                    descendant_count: 1,
                },
                RowDescription::Session {
                    id: "child".to_string(),
                    breadcrumb: Some("root".to_string()),
                    descendant_count: 0,
                },
            ]
        );
    }

    #[test]
    fn test_build_session_rows_descendant_count_covers_every_depth() {
        let root = create_test_session("root", SessionStatus::Running);
        let mut mid = create_test_session("mid", SessionStatus::Running);
        mid.ancestor_session_ids = vec!["root".to_string()];
        let mut leaf = create_test_session("leaf", SessionStatus::Running);
        leaf.ancestor_session_ids = vec!["root".to_string(), "mid".to_string()];

        let sessions: Vec<&Session> = vec![&root, &mid, &leaf];
        let rows = build_session_rows(&sessions);

        assert_eq!(
            describe(&rows),
            vec![
                RowDescription::Header {
                    label: "RUNNING (3)".to_string(),
                },
                RowDescription::Session {
                    id: "root".to_string(),
                    breadcrumb: None,
                    descendant_count: 2,
                },
                RowDescription::Session {
                    id: "mid".to_string(),
                    breadcrumb: Some("root".to_string()),
                    descendant_count: 1,
                },
                RowDescription::Session {
                    id: "leaf".to_string(),
                    breadcrumb: Some("mid".to_string()),
                    descendant_count: 0,
                },
            ]
        );
    }

    #[test]
    fn test_build_session_rows_descendant_count_survives_deleted_intermediate_ancestor() {
        let root = create_test_session("root", SessionStatus::Running);
        let mut leaf = create_test_session("leaf", SessionStatus::Running);
        leaf.ancestor_session_ids = vec!["root".to_string(), "deleted_middle".to_string()];

        let sessions: Vec<&Session> = vec![&root, &leaf];
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
                    descendant_count: 1,
                },
                RowDescription::Session {
                    id: "leaf".to_string(),
                    breadcrumb: Some("root".to_string()),
                    descendant_count: 0,
                },
            ]
        );
    }

    #[test]
    fn test_build_session_rows_descendant_count_ignores_sessions_outside_current_view() {
        // "child" is a real descendant of "root" but was filtered out of the
        // current view before `build_session_rows` was called (e.g. by
        // search/status filtering), so it must not be reflected in the count.
        let root = create_test_session("root", SessionStatus::Running);

        let sessions: Vec<&Session> = vec![&root];
        let rows = build_session_rows(&sessions);

        assert_eq!(
            describe(&rows),
            vec![
                RowDescription::Header {
                    label: "RUNNING (1)".to_string(),
                },
                RowDescription::Session {
                    id: "root".to_string(),
                    breadcrumb: None,
                    descendant_count: 0,
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
