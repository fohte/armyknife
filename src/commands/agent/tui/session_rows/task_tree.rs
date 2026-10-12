use std::collections::{HashMap, HashSet};

use super::{SessionRowEntry, SessionTask, TaskMark};
use crate::commands::agent::types::{AgentStatusKind, DisplayStatus, Session, SessionStatus};
use crate::commands::agent::user_turn::UserTurnKind;

/// Builds TASKS rows: each root (a session with no displayed ancestor)
/// followed by every displayed descendant flattened onto one child level.
/// Delegated sessions are left out because they belong to a single PR rather
/// than a task.
pub(super) fn build_task_rows<'a>(
    sessions: &[&'a Session],
    task_by_session: &HashMap<String, SessionTask>,
    turn_kinds: &HashMap<String, UserTurnKind>,
) -> Vec<SessionRowEntry<'a>> {
    let members: Vec<&'a Session> = sessions
        .iter()
        .copied()
        .filter(|session| !session.is_delegated)
        .collect();
    let member_ids: HashSet<&str> = members.iter().map(|s| s.session_id.as_str()).collect();

    let mut groups: Vec<(&'a Session, Vec<&'a Session>)> = Vec::new();
    let mut children_by_root: HashMap<&str, Vec<&'a Session>> = HashMap::new();
    for &session in &members {
        // The first displayed id in root-to-parent order is the root, since
        // its own ancestors are the ids before it and none of them are shown.
        match session
            .ancestor_session_ids
            .iter()
            .find(|id| member_ids.contains(id.as_str()))
        {
            Some(root_id) => children_by_root
                .entry(root_id.as_str())
                .or_default()
                .push(session),
            None => groups.push((session, Vec::new())),
        }
    }
    for (root, children) in &mut groups {
        if let Some(found) = children_by_root.remove(root.session_id.as_str()) {
            *children = found;
        }
        children.sort_by_key(|child| std::cmp::Reverse(child.updated_at));
    }

    let mark_of = |session: &Session| task_mark(session, turn_kinds.get(&session.session_id));
    groups.sort_by_cached_key(|(root, children)| {
        let has_user_turn = std::iter::once(root)
            .chain(children.iter())
            .any(|s| mark_of(s) == TaskMark::UserTurn);
        let last_moved = std::iter::once(root)
            .chain(children.iter())
            .map(|s| s.updated_at)
            .max();
        (!has_user_turn, std::cmp::Reverse(last_moved))
    });

    let entry = |session: &'a Session, depth: usize, is_last_sibling: bool| SessionRowEntry {
        session,
        depth,
        mark: mark_of(session),
        is_last_sibling,
        is_selectable: !turn_kinds.contains_key(&session.session_id),
        child_count: 0,
        child_user_turn_count: 0,
        task: task_by_session.get(session.session_id.as_str()).cloned(),
    };
    let mut rows = Vec::with_capacity(members.len());
    for (root, children) in groups {
        let mut root_entry = entry(root, 0, false);
        root_entry.child_count = children.len();
        root_entry.child_user_turn_count = children
            .iter()
            .filter(|child| mark_of(child) == TaskMark::UserTurn)
            .count();
        rows.push(root_entry);
        let last = children.len().saturating_sub(1);
        rows.extend(
            children
                .into_iter()
                .enumerate()
                .map(|(i, child)| entry(child, 1, i == last)),
        );
    }
    rows
}

fn task_mark(session: &Session, turn_kind: Option<&UserTurnKind>) -> TaskMark {
    match turn_kind {
        Some(UserTurnKind::Close) => return TaskMark::Close,
        Some(_) => return TaskMark::UserTurn,
        None => {}
    }
    // A status set before the last stop is stale once the session runs again.
    if session.status != SessionStatus::Running {
        match session.agent_status.as_ref().map(|status| status.kind) {
            Some(AgentStatusKind::Done) => return TaskMark::Done,
            Some(AgentStatusKind::Wait) => return TaskMark::Waiting,
            _ => {}
        }
    }
    match session.display_status() {
        DisplayStatus::Running => TaskMark::Working,
        DisplayStatus::Background => TaskMark::Waiting,
        DisplayStatus::Ended => TaskMark::Done,
        DisplayStatus::WaitingInput
        | DisplayStatus::UnreadStopped
        | DisplayStatus::Stopped
        | DisplayStatus::Paused => TaskMark::Unclassified,
    }
}
