use std::collections::{HashMap, HashSet};

use crate::commands::agent::tui::progress::ProgressRow;

use super::{Section, SectionHeaderRow, SessionRow, SessionRowEntry, SessionRowKey};

#[derive(Debug)]
pub(in crate::commands::agent::tui) struct InlineProgressEntry<'a> {
    pub progress: Option<ProgressRow>,
    pub label: Option<String>,
    pub session: Option<SessionRowEntry<'a>>,
    pub tree_prefix: Vec<bool>,
    pub is_last_sibling: bool,
    pub selection_key: Option<SessionRowKey>,
}

pub(in crate::commands::agent::tui) fn project_inline_progress_rows<'a>(
    original_rows: Vec<SessionRow<'a>>,
    remaining_rows: Vec<SessionRow<'a>>,
    source_session_id: &str,
    progress_rows: &[ProgressRow],
    matched_session_ids: &HashSet<String>,
    status: Option<String>,
) -> Vec<SessionRow<'a>> {
    let entries = original_rows
        .iter()
        .filter_map(|row| match row {
            SessionRow::Session(entry) => Some((entry.session.session_id.clone(), entry.clone())),
            _ => None,
        })
        .collect::<HashMap<_, _>>();
    let descendants = collect_matched_descendants(&original_rows, matched_session_ids);

    let mut priority_rows = original_rows
        .into_iter()
        .filter(|row| {
            matches!(
                row,
                SessionRow::UserTurn(_)
                    | SessionRow::SectionHeader(SectionHeaderRow {
                        kind: Section::UserTurn,
                        ..
                    })
            )
        })
        .collect::<Vec<_>>();
    let mut task_rows = remaining_rows
        .into_iter()
        .filter(|row| {
            matches!(
                row,
                SessionRow::Session(_)
                    | SessionRow::SectionHeader(SectionHeaderRow {
                        kind: Section::Tasks,
                        ..
                    })
            )
        })
        .collect::<Vec<_>>();

    let Some(source_placement) = find_source_placement(&task_rows, source_session_id) else {
        priority_rows.extend(task_rows);
        return priority_rows;
    };
    let inline_rows = build_inline_progress_rows(
        source_session_id,
        progress_rows,
        status,
        source_placement.has_remaining_children,
        &entries,
        &descendants,
    );

    if inline_rows.is_empty() {
        priority_rows.extend(task_rows);
        return priority_rows;
    }
    task_rows.splice(
        source_placement.row_index + 1..source_placement.row_index + 1,
        inline_rows,
    );
    priority_rows.extend(task_rows);
    priority_rows
}

struct SourcePlacement {
    row_index: usize,
    has_remaining_children: bool,
}

fn find_source_placement(
    task_rows: &[SessionRow<'_>],
    source_session_id: &str,
) -> Option<SourcePlacement> {
    let row_index = task_rows.iter().position(
        |row| matches!(row, SessionRow::Session(entry) if entry.session.session_id == source_session_id),
    )?;
    let source_depth = match &task_rows[row_index] {
        SessionRow::Session(entry) => entry
            .tree_prefix
            .as_ref()
            .map_or(0, |prefix| prefix.len() + 1),
        _ => 0,
    };
    let has_remaining_children = task_rows
        .iter()
        .skip(row_index + 1)
        .take_while(|row| !matches!(row, SessionRow::Session(entry) if entry.tree_prefix.is_none()))
        .any(|row| {
            matches!(row, SessionRow::Session(entry)
                if entry.session.ancestor_session_ids.iter().any(|id| id == source_session_id)
                    && entry.tree_prefix.as_ref().is_some_and(|prefix| prefix.len() == source_depth))
        });

    Some(SourcePlacement {
        row_index,
        has_remaining_children,
    })
}

fn build_inline_progress_rows<'a>(
    source_session_id: &str,
    progress_rows: &[ProgressRow],
    status: Option<String>,
    has_remaining_children: bool,
    entries: &HashMap<String, SessionRowEntry<'a>>,
    descendants: &HashMap<String, Vec<SessionRowEntry<'a>>>,
) -> Vec<SessionRow<'a>> {
    let mut inline_rows = Vec::new();
    let has_visible_progress_rows = progress_rows.iter().any(|row| {
        matches!(
            row,
            ProgressRow::ImplementationHeader { .. }
                | ProgressRow::BatchHeader { .. }
                | ProgressRow::ChecklistPullRequest { .. }
                | ProgressRow::PlannedPullRequest { .. }
        )
    });
    if let Some(label) = status {
        inline_rows.push(SessionRow::InlineProgress(Box::new(InlineProgressEntry {
            progress: None,
            label: Some(label),
            session: None,
            tree_prefix: Vec::new(),
            is_last_sibling: !has_remaining_children && !has_visible_progress_rows,
            selection_key: None,
        })));
    }

    let visible_batches = progress_rows
        .iter()
        .filter(|row| matches!(row, ProgressRow::BatchHeader { .. }))
        .count();
    let last_batch_id = progress_rows.iter().rev().find_map(|row| match row {
        ProgressRow::BatchHeader { batch_id, .. } => Some(batch_id.as_str()),
        _ => None,
    });
    let mut batch_index = 0;
    let mut batch_item_counts = HashMap::<String, usize>::new();
    for row in progress_rows {
        if let Some(batch_id) = row_batch_id(row) {
            *batch_item_counts.entry(batch_id.to_string()).or_default() += 1;
        }
    }
    let mut batch_item_indices = HashMap::<String, usize>::new();

    for progress in progress_rows {
        let Some(selection_key) = progress.selection_key() else {
            continue;
        };
        let (tree_prefix, is_last_sibling, session) = match progress {
            ProgressRow::ImplementationHeader { .. } => (Vec::new(), !has_remaining_children, None),
            ProgressRow::BatchHeader { .. } => {
                batch_index += 1;
                (
                    vec![!has_remaining_children],
                    batch_index == visible_batches,
                    None,
                )
            }
            ProgressRow::ChecklistPullRequest { batch_id, row, .. } => {
                let item_index = batch_item_indices.entry(batch_id.clone()).or_default();
                *item_index += 1;
                let item_count = batch_item_counts.get(batch_id).copied().unwrap_or(1);
                (
                    vec![
                        !has_remaining_children,
                        Some(batch_id.as_str()) == last_batch_id,
                    ],
                    *item_index == item_count,
                    entries.get(&row.session_id).cloned(),
                )
            }
            ProgressRow::PlannedPullRequest { batch_id, .. } => {
                let item_index = batch_item_indices.entry(batch_id.clone()).or_default();
                *item_index += 1;
                let item_count = batch_item_counts.get(batch_id).copied().unwrap_or(1);
                (
                    vec![
                        !has_remaining_children,
                        Some(batch_id.as_str()) == last_batch_id,
                    ],
                    *item_index == item_count,
                    None,
                )
            }
            ProgressRow::Work(_)
            | ProgressRow::PullRequest(_)
            | ProgressRow::UnmatchedDelegatesHeader { .. } => continue,
        };

        let session_id = session
            .as_ref()
            .map(|entry| entry.session.session_id.clone());
        inline_rows.push(SessionRow::InlineProgress(Box::new(InlineProgressEntry {
            progress: Some(progress.clone()),
            label: None,
            session: session.clone().map(|mut entry| {
                entry.tree_prefix = Some(tree_prefix.clone());
                entry.is_last_sibling = is_last_sibling;
                entry
            }),
            tree_prefix: tree_prefix.clone(),
            is_last_sibling,
            selection_key: Some(SessionRowKey::Progress {
                source_session_id: source_session_id.to_string(),
                selection_key,
                session_id,
            }),
        })));

        if let Some(parent) = session
            && let Some(children) = descendants.get(&parent.session.session_id)
        {
            for child in children {
                let mut child = child.clone();
                let old_prefix = child.tree_prefix.clone().unwrap_or_default();
                let mut child_prefix = tree_prefix.clone();
                child_prefix.push(is_last_sibling);
                child_prefix.extend(old_prefix.into_iter().skip(1));
                child.tree_prefix = Some(child_prefix);
                inline_rows.push(SessionRow::Session(child));
            }
        }
    }

    inline_rows
}

fn collect_matched_descendants<'a>(
    rows: &[SessionRow<'a>],
    matched_session_ids: &HashSet<String>,
) -> HashMap<String, Vec<SessionRowEntry<'a>>> {
    let mut descendants = HashMap::<String, Vec<SessionRowEntry<'a>>>::new();
    for entry in rows.iter().filter_map(|row| match row {
        SessionRow::Session(entry) => Some(entry),
        _ => None,
    }) {
        if matched_session_ids.contains(&entry.session.session_id) {
            continue;
        }
        if let Some(parent_id) = entry
            .session
            .ancestor_session_ids
            .iter()
            .rev()
            .find(|id| matched_session_ids.contains(id.as_str()))
        {
            descendants
                .entry(parent_id.clone())
                .or_default()
                .push(entry.clone());
        }
    }
    descendants
}

fn row_batch_id(row: &ProgressRow) -> Option<&str> {
    match row {
        ProgressRow::ChecklistPullRequest { batch_id, .. }
        | ProgressRow::PlannedPullRequest { batch_id, .. } => Some(batch_id),
        _ => None,
    }
}
