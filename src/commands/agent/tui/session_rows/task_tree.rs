use std::collections::{HashMap, HashSet};

use super::{SessionRowEntry, SessionTask, TaskMark};
use crate::commands::agent::types::{DisplayStatus, Session, SessionStatus};
use crate::shared::config::{AgentConfig, AgentStatusState};

/// Builds TASKS rows: each root (a session with no displayed ancestor)
/// followed by every displayed descendant flattened onto one child level.
/// Delegated sessions are left out because they belong to a single PR rather
/// than a task.
pub(super) fn build_task_rows<'a>(
    sessions: &[&'a Session],
    task_by_session: &HashMap<String, SessionTask>,
    user_turn_session_ids: &HashSet<String>,
    agent_config: &AgentConfig,
) -> Vec<SessionRowEntry<'a>> {
    let members: Vec<&'a Session> = sessions
        .iter()
        .copied()
        .filter(|session| !session.is_delegated)
        .collect();
    let member_ids: HashSet<&str> = members.iter().map(|s| s.session_id.as_str()).collect();
    let has_displayed_ancestor = |session: &Session| {
        session
            .ancestor_session_ids
            .iter()
            .any(|id| member_ids.contains(id.as_str()))
    };
    let root_ids: HashSet<&str> = members
        .iter()
        .filter(|session| !has_displayed_ancestor(session))
        .map(|session| session.session_id.as_str())
        .collect();

    let mut groups: Vec<(&'a Session, Vec<&'a Session>)> = Vec::new();
    let mut children_by_root: HashMap<&str, Vec<&'a Session>> = HashMap::new();
    for &session in &members {
        // Ancestor lists recorded at different times can disagree (or even
        // form a cycle); a session that reaches no root becomes one itself
        // rather than disappearing.
        match session
            .ancestor_session_ids
            .iter()
            .find(|id| root_ids.contains(id.as_str()))
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

    let mark_of = |session: &Session| {
        task_mark(
            session,
            user_turn_session_ids.contains(&session.session_id),
            agent_config,
        )
    };
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

fn task_mark(session: &Session, has_user_turn: bool, agent_config: &AgentConfig) -> TaskMark {
    if has_user_turn {
        return TaskMark::UserTurn;
    }
    // A status set before the last stop is stale once the session runs again.
    if session.status != SessionStatus::Running {
        match session
            .agent_status
            .as_ref()
            .map(|status| agent_config.status_state(&status.kind))
        {
            Some(AgentStatusState::Done) => return TaskMark::Done,
            Some(AgentStatusState::Waiting) => return TaskMark::Waiting,
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
