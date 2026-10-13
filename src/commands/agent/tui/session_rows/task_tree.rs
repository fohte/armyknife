use std::collections::{HashMap, HashSet};

use super::{SessionRowEntry, SessionTask, TaskMark};
use crate::commands::agent::types::{DisplayStatus, Session, SessionStatus};
use crate::shared::config::{AgentConfig, AgentStatusState};

/// Builds TASKS rows in their displayed parent-child hierarchy. Delegated
/// sessions are left out because they belong to a single PR rather than a task.
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
    let member_ids: HashSet<&'a str> = members
        .iter()
        .map(|session| session.session_id.as_str())
        .collect();
    let mut children_by_parent: HashMap<&'a str, Vec<&'a Session>> = HashMap::new();
    let mut roots = Vec::new();

    for &session in &members {
        let parent_id = session
            .ancestor_session_ids
            .iter()
            .rev()
            .find(|id| id.as_str() != session.session_id && member_ids.contains(id.as_str()));
        if let Some(parent_id) = parent_id {
            children_by_parent
                .entry(parent_id.as_str())
                .or_default()
                .push(session);
        } else {
            roots.push(session);
        }
    }
    for children in children_by_parent.values_mut() {
        children.sort_by_key(|child| std::cmp::Reverse(child.updated_at));
    }

    let mut visited = HashSet::new();
    let mut groups = Vec::new();
    let builder = TaskTreeBuilder {
        children_by_parent: &children_by_parent,
        task_by_session,
        user_turn_session_ids,
        agent_config,
    };

    // Include unvisited sessions as fallback roots. Inconsistent ancestor
    // metadata can form cycles, leaving a component without a natural root.
    for root in roots.into_iter().chain(members.iter().copied()) {
        if visited.contains(root.session_id.as_str()) {
            continue;
        }

        let mut rows = Vec::new();
        builder.append_subtree(root, false, None, &mut visited, &mut rows);
        let has_user_turn = rows.iter().any(|row| row.mark == TaskMark::UserTurn);
        let last_moved = rows
            .iter()
            .map(|row| row.session.updated_at)
            .max()
            .unwrap_or(root.updated_at);
        groups.push(TaskTreeGroup {
            rows,
            has_user_turn,
            last_moved,
        });
    }

    groups.sort_by_cached_key(|group| (!group.has_user_turn, std::cmp::Reverse(group.last_moved)));

    let mut rows = Vec::with_capacity(members.len());
    for group in groups {
        let child_count = group.rows.len().saturating_sub(1);
        let child_user_turn_count = group
            .rows
            .iter()
            .skip(1)
            .filter(|row| row.mark == TaskMark::UserTurn)
            .count();
        let mut group_rows = group.rows;
        if let Some(root) = group_rows.first_mut() {
            root.child_count = child_count;
            root.child_user_turn_count = child_user_turn_count;
        }
        rows.extend(group_rows);
    }
    rows
}

struct TaskTreeGroup<'a> {
    rows: Vec<SessionRowEntry<'a>>,
    has_user_turn: bool,
    last_moved: chrono::DateTime<chrono::Utc>,
}

struct TaskTreeBuilder<'context, 'session> {
    children_by_parent: &'context HashMap<&'session str, Vec<&'session Session>>,
    task_by_session: &'context HashMap<String, SessionTask>,
    user_turn_session_ids: &'context HashSet<String>,
    agent_config: &'context AgentConfig,
}

impl<'context, 'session> TaskTreeBuilder<'context, 'session> {
    fn append_subtree(
        &self,
        session: &'session Session,
        is_last_sibling: bool,
        tree_prefix: Option<Vec<bool>>,
        visited: &mut HashSet<&'session str>,
        rows: &mut Vec<SessionRowEntry<'session>>,
    ) {
        if !visited.insert(session.session_id.as_str()) {
            return;
        }

        let has_user_turn = self.user_turn_session_ids.contains(&session.session_id);
        rows.push(SessionRowEntry {
            session,
            mark: task_mark(session, has_user_turn, self.agent_config),
            is_last_sibling,
            tree_prefix: tree_prefix.clone(),
            child_count: 0,
            child_user_turn_count: 0,
            task: self
                .task_by_session
                .get(session.session_id.as_str())
                .cloned(),
        });

        let Some(children) = self.children_by_parent.get(session.session_id.as_str()) else {
            return;
        };
        let last_index = children.len().saturating_sub(1);
        for (index, &child) in children.iter().enumerate() {
            let child_prefix = Some(match &tree_prefix {
                Some(prefix) => {
                    let mut child_prefix = prefix.clone();
                    child_prefix.push(is_last_sibling);
                    child_prefix
                }
                None => Vec::new(),
            });
            self.append_subtree(child, index == last_index, child_prefix, visited, rows);
        }
    }
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
