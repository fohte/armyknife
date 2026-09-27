use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};

use crate::commands::agent::types::{DisplayStatus, Session};
use crate::infra::tq::{TqProject, TqSidebarTask};

use super::tq_snapshot::TqSnapshot;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub(super) enum SidebarSelection {
    #[default]
    All,
    Untasked,
    Project(String),
    Task(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SidebarRowKind {
    All,
    Untasked,
    Separator,
    Project,
    Task,
}

#[derive(Debug, Clone)]
pub(super) struct SidebarRow {
    pub kind: SidebarRowKind,
    pub selection: Option<SidebarSelection>,
    pub depth: usize,
    pub title: String,
    pub task: Option<TqSidebarTask>,
    pub project: Option<TqProject>,
    pub has_children: bool,
    pub expanded: bool,
    pub session_counts: Option<Vec<(DisplayStatus, usize)>>,
}

#[derive(Debug, Clone)]
pub(super) enum SidebarScopeFilter {
    All,
    Untasked,
    AssignedToAny(HashSet<String>),
}

enum RootNode {
    Project(String),
    Task(String),
}

struct TaskRowsContext<'a> {
    children_by_parent: &'a HashMap<String, Vec<String>>,
    visited: &'a mut HashSet<String>,
    rows: &'a mut Vec<SidebarRow>,
    sessions: &'a [Session],
    snapshot: &'a TqSnapshot,
    collapsed: &'a HashSet<SidebarSelection>,
}

pub(super) fn build_sidebar_rows(
    snapshot: Option<&TqSnapshot>,
    sessions: &[Session],
    collapsed: &HashSet<SidebarSelection>,
) -> Vec<SidebarRow> {
    let mut rows = vec![
        SidebarRow {
            kind: SidebarRowKind::All,
            selection: Some(SidebarSelection::All),
            depth: 0,
            title: "すべて".to_string(),
            task: None,
            project: None,
            has_children: false,
            expanded: false,
            session_counts: Some(count_sessions(sessions, |_| true)),
        },
        SidebarRow {
            kind: SidebarRowKind::Untasked,
            selection: Some(SidebarSelection::Untasked),
            depth: 0,
            title: "タスクなし".to_string(),
            task: None,
            project: None,
            has_children: false,
            expanded: false,
            session_counts: snapshot.map(|snapshot| {
                count_sessions(sessions, |session| {
                    snapshot
                        .session_tasks
                        .get(&session.session_id)
                        .is_none_or(Vec::is_empty)
                })
            }),
        },
        SidebarRow {
            kind: SidebarRowKind::Separator,
            selection: None,
            depth: 0,
            title: String::new(),
            task: None,
            project: None,
            has_children: false,
            expanded: false,
            session_counts: None,
        },
    ];

    let Some(snapshot) = snapshot else {
        return rows;
    };

    let mut children_by_parent: HashMap<String, Vec<String>> = HashMap::new();
    let mut project_roots: HashMap<String, Vec<String>> = HashMap::new();
    let mut ungrouped_roots = Vec::new();

    for task in snapshot.tasks.values() {
        let project_id = effective_project_id(task, snapshot);
        let parent = task.parent_id.as_ref().and_then(|parent_id| {
            snapshot
                .tasks
                .get(parent_id)
                .filter(|parent| effective_project_id(parent, snapshot) == project_id)
        });
        if let Some(parent) = parent {
            children_by_parent
                .entry(parent.id.clone())
                .or_default()
                .push(task.id.clone());
        } else if let Some(project_id) =
            project_id.filter(|project_id| snapshot.projects.contains_key(*project_id))
        {
            project_roots
                .entry(project_id.to_string())
                .or_default()
                .push(task.id.clone());
        } else {
            ungrouped_roots.push(task.id.clone());
        }
    }

    for children in children_by_parent.values_mut() {
        sort_task_ids(children, snapshot);
    }
    for roots in project_roots.values_mut() {
        sort_task_ids(roots, snapshot);
    }
    sort_task_ids(&mut ungrouped_roots, snapshot);

    let mut roots = project_roots
        .keys()
        .map(|project_id| RootNode::Project(project_id.clone()))
        .chain(ungrouped_roots.iter().cloned().map(RootNode::Task))
        .collect::<Vec<_>>();
    roots.sort_by(|left, right| compare_root_recency(left, right, snapshot));

    let mut visited = HashSet::new();
    {
        let mut context = TaskRowsContext {
            children_by_parent: &children_by_parent,
            visited: &mut visited,
            rows: &mut rows,
            sessions,
            snapshot,
            collapsed,
        };
        for root in roots {
            match root {
                RootNode::Project(project_id) => {
                    let Some(project) = snapshot.projects.get(&project_id) else {
                        continue;
                    };
                    let selection = SidebarSelection::Project(project_id.clone());
                    let has_children = project_roots
                        .get(&project_id)
                        .is_some_and(|children| !children.is_empty());
                    let expanded = has_children && !collapsed.contains(&selection);
                    context.rows.push(SidebarRow {
                        kind: SidebarRowKind::Project,
                        selection: Some(selection),
                        depth: 0,
                        title: project.title.clone(),
                        task: None,
                        project: Some(project.clone()),
                        has_children,
                        expanded,
                        session_counts: Some(count_for_project(&project_id, sessions, snapshot)),
                    });
                    if expanded && let Some(task_ids) = project_roots.get(&project_id) {
                        for task_id in task_ids {
                            context.append(task_id, 1);
                        }
                    }
                }
                RootNode::Task(task_id) => context.append(&task_id, 0),
            }
        }
    }

    let mut unvisited = snapshot
        .tasks
        .keys()
        .filter(|id| !visited.contains(*id))
        .cloned()
        .collect::<Vec<_>>();
    sort_task_ids(&mut unvisited, snapshot);
    let mut context = TaskRowsContext {
        children_by_parent: &children_by_parent,
        visited: &mut visited,
        rows: &mut rows,
        sessions,
        snapshot,
        collapsed,
    };
    for task_id in unvisited {
        context.append(&task_id, 0);
    }

    rows
}

pub(super) fn scope_filter(
    snapshot: Option<&TqSnapshot>,
    selection: &SidebarSelection,
) -> SidebarScopeFilter {
    match selection {
        SidebarSelection::All => SidebarScopeFilter::All,
        SidebarSelection::Untasked => {
            if snapshot.is_some() {
                SidebarScopeFilter::Untasked
            } else {
                SidebarScopeFilter::All
            }
        }
        SidebarSelection::Task(task_id) => snapshot.map_or(SidebarScopeFilter::All, |snapshot| {
            SidebarScopeFilter::AssignedToAny(task_descendants(task_id, snapshot))
        }),
        SidebarSelection::Project(project_id) => {
            snapshot.map_or(SidebarScopeFilter::All, |snapshot| {
                SidebarScopeFilter::AssignedToAny(
                    snapshot
                        .tasks
                        .values()
                        .filter(|task| {
                            effective_project_id(task, snapshot) == Some(project_id.as_str())
                        })
                        .map(|task| task.id.clone())
                        .collect(),
                )
            })
        }
    }
}

pub(super) fn matches_scope(
    filter: &SidebarScopeFilter,
    snapshot: Option<&TqSnapshot>,
    session_id: &str,
) -> bool {
    match filter {
        SidebarScopeFilter::All => true,
        SidebarScopeFilter::Untasked => snapshot.is_some_and(|snapshot| {
            snapshot
                .session_tasks
                .get(session_id)
                .is_none_or(Vec::is_empty)
        }),
        SidebarScopeFilter::AssignedToAny(task_ids) => snapshot.is_some_and(|snapshot| {
            snapshot
                .session_tasks
                .get(session_id)
                .is_some_and(|tasks| tasks.iter().any(|task| task_ids.contains(&task.task_id)))
        }),
    }
}

pub(super) fn selection_exists(
    snapshot: Option<&TqSnapshot>,
    selection: &SidebarSelection,
) -> bool {
    match selection {
        SidebarSelection::All | SidebarSelection::Untasked => true,
        SidebarSelection::Task(task_id) => {
            snapshot.is_some_and(|snapshot| snapshot.tasks.contains_key(task_id))
        }
        SidebarSelection::Project(project_id) => snapshot.is_some_and(|snapshot| {
            snapshot.projects.contains_key(project_id)
                && snapshot
                    .tasks
                    .values()
                    .any(|task| effective_project_id(task, snapshot) == Some(project_id.as_str()))
        }),
    }
}

impl TaskRowsContext<'_> {
    fn append(&mut self, task_id: &str, depth: usize) {
        if !self.visited.insert(task_id.to_string()) {
            return;
        }
        let Some(task) = self.snapshot.tasks.get(task_id) else {
            return;
        };
        let selection = SidebarSelection::Task(task_id.to_string());
        let children = self
            .children_by_parent
            .get(task_id)
            .cloned()
            .unwrap_or_default();
        let has_children = !children.is_empty();
        let expanded = has_children && !self.collapsed.contains(&selection);
        self.rows.push(SidebarRow {
            kind: SidebarRowKind::Task,
            selection: Some(selection),
            depth,
            title: task.title.clone(),
            task: Some(task.clone()),
            project: None,
            has_children,
            expanded,
            session_counts: Some(count_for_task(&task.id, self.sessions, self.snapshot)),
        });
        if expanded {
            for child_id in children {
                self.append(&child_id, depth + 1);
            }
        }
    }
}

fn count_for_task(
    task_id: &str,
    sessions: &[Session],
    snapshot: &TqSnapshot,
) -> Vec<(DisplayStatus, usize)> {
    let task_ids = task_descendants(task_id, snapshot);
    count_linked_sessions(&task_ids, sessions, snapshot)
}

fn count_linked_sessions(
    task_ids: &HashSet<String>,
    sessions: &[Session],
    snapshot: &TqSnapshot,
) -> Vec<(DisplayStatus, usize)> {
    count_sessions(sessions, |session| {
        snapshot
            .session_tasks
            .get(&session.session_id)
            .is_some_and(|tasks| tasks.iter().any(|task| task_ids.contains(&task.task_id)))
    })
}

fn count_for_project(
    project_id: &str,
    sessions: &[Session],
    snapshot: &TqSnapshot,
) -> Vec<(DisplayStatus, usize)> {
    let task_ids = snapshot
        .tasks
        .values()
        .filter(|task| effective_project_id(task, snapshot) == Some(project_id))
        .map(|task| task.id.clone())
        .collect::<HashSet<_>>();
    count_linked_sessions(&task_ids, sessions, snapshot)
}

fn count_sessions(
    sessions: &[Session],
    matches: impl Fn(&Session) -> bool,
) -> Vec<(DisplayStatus, usize)> {
    let mut counts = Vec::new();
    for session in sessions.iter().filter(|session| matches(session)) {
        let status = session.display_status();
        if let Some((_, count)) = counts.iter_mut().find(|(existing, _)| *existing == status) {
            *count += 1;
        } else {
            counts.push((status, 1));
        }
    }
    counts.sort_by_key(|(status, _)| status_order(*status));
    counts
}

fn status_order(status: DisplayStatus) -> u8 {
    match status {
        DisplayStatus::WaitingInput => 0,
        DisplayStatus::Running => 1,
        DisplayStatus::Background => 2,
        DisplayStatus::UnreadStopped => 3,
        DisplayStatus::Stopped => 4,
        DisplayStatus::Paused => 5,
        DisplayStatus::Ended => 6,
    }
}

fn task_descendants(task_id: &str, snapshot: &TqSnapshot) -> HashSet<String> {
    snapshot
        .tasks
        .values()
        .filter(|task| task_is_descendant_of(task, task_id, snapshot))
        .map(|task| task.id.clone())
        .collect()
}

fn effective_project_id<'a>(task: &'a TqSidebarTask, snapshot: &'a TqSnapshot) -> Option<&'a str> {
    let mut current = Some(task);
    let mut visited = HashSet::new();
    while let Some(task) = current {
        if let Some(project_id) = task.project_id.as_deref() {
            return Some(project_id);
        }
        if !visited.insert(task.id.as_str()) {
            return None;
        }
        current = task
            .parent_id
            .as_ref()
            .and_then(|parent_id| snapshot.tasks.get(parent_id));
    }
    None
}

fn task_is_descendant_of(task: &TqSidebarTask, ancestor_id: &str, snapshot: &TqSnapshot) -> bool {
    let mut current_id = task.parent_id.as_deref();
    let mut visited = HashSet::new();
    while let Some(parent_id) = current_id {
        if parent_id == ancestor_id {
            return true;
        }
        if !visited.insert(parent_id) {
            return false;
        }
        current_id = snapshot
            .tasks
            .get(parent_id)
            .and_then(|parent| parent.parent_id.as_deref());
    }
    task.id == ancestor_id
}

fn sort_task_ids(task_ids: &mut [String], snapshot: &TqSnapshot) {
    task_ids.sort_by(|left, right| {
        compare_task_recency(snapshot.tasks.get(right), snapshot.tasks.get(left))
    });
}

fn compare_task_recency(left: Option<&TqSidebarTask>, right: Option<&TqSidebarTask>) -> Ordering {
    match (left, right) {
        (Some(left), Some(right)) => match (&left.created_at, &right.created_at) {
            (Some(left_date), Some(right_date)) => left_date
                .cmp(right_date)
                .then_with(|| left.number.cmp(&right.number)),
            _ => left.number.cmp(&right.number),
        },
        (Some(_), None) => Ordering::Greater,
        (None, Some(_)) => Ordering::Less,
        (None, None) => Ordering::Equal,
    }
}

fn compare_root_recency(left: &RootNode, right: &RootNode, snapshot: &TqSnapshot) -> Ordering {
    compare_task_recency(
        latest_root_task(right, snapshot),
        latest_root_task(left, snapshot),
    )
}

fn latest_root_task<'a>(node: &RootNode, snapshot: &'a TqSnapshot) -> Option<&'a TqSidebarTask> {
    match node {
        RootNode::Project(project_id) => snapshot
            .tasks
            .values()
            .filter(|task| effective_project_id(task, snapshot) == Some(project_id))
            .max_by(|a, b| compare_task_recency(Some(a), Some(b))),
        RootNode::Task(task_id) => snapshot.tasks.get(task_id),
    }
}
