use std::collections::HashMap;

use chrono::{DateTime, Duration as ChronoDuration, Utc};

use super::super::progress::ProgressFetchScope;
use super::super::progress::ProgressRow;
use super::super::progress_batches::ImplementationPlan;
use super::App;
use super::progress::CachedProgress;

mod disk_cache;

pub(super) fn same_implementation_scope(
    cached: Option<&ProgressFetchScope>,
    current: Option<&ProgressFetchScope>,
) -> bool {
    match (cached, current) {
        (Some(cached), Some(current)) if cached.task_id.is_some() && current.task_id.is_some() => {
            cached.task_id == current.task_id
        }
        _ => cached == current,
    }
}

pub(super) fn expand_initial_target_batch(
    plan: &ImplementationPlan,
    target: Option<&str>,
    expanded_batches: &mut HashMap<String, bool>,
) {
    if let Some(batch_id) = target.and_then(|delegate_id| plan.batch_id_for_delegate(delegate_id)) {
        expanded_batches.insert(batch_id.to_string(), true);
    }
}

impl App {
    pub(in crate::commands::agent::tui) fn load_progress_cache_from_disk(
        &mut self,
    ) -> anyhow::Result<()> {
        if let Some(entries) = disk_cache::load()? {
            self.restore_progress_cache(entries);
        }
        Ok(())
    }

    pub(super) fn restore_progress_cache(&mut self, entries: HashMap<String, CachedProgress>) {
        self.progress_cache.extend(
            entries
                .into_iter()
                .map(|(task_id, snapshot)| (task_cache_key(&task_id), snapshot)),
        );
    }

    pub(in crate::commands::agent::tui) fn progress_refresh_due(
        &self,
        session_id: &str,
        now: DateTime<Utc>,
        max_age: std::time::Duration,
    ) -> bool {
        let cache_key = self.progress_cache_key(session_id);
        let last_attempt = self
            .progress_cache
            .get(&cache_key)
            .and_then(|snapshot| snapshot.last_attempt_at.or(snapshot.fetched_at));
        let Some(last_attempt) = last_attempt else {
            return true;
        };
        let max_age_millis = i64::try_from(max_age.as_millis()).unwrap_or(i64::MAX);
        now.signed_duration_since(last_attempt) >= ChronoDuration::milliseconds(max_age_millis)
    }

    pub(in crate::commands::agent::tui) fn mark_progress_refresh_started(
        &mut self,
        session_id: &str,
        attempted_at: DateTime<Utc>,
    ) {
        let cache_key = self.progress_cache_key(session_id);
        let has_cached_result = self
            .progress_cache
            .get_mut(&cache_key)
            .map(|snapshot| {
                snapshot.last_attempt_at = Some(attempted_at);
                snapshot.fetched_at.is_some()
            })
            .unwrap_or(false);
        if has_cached_result {
            self.persist_progress_cache();
        }
    }

    pub(super) fn progress_cache_key(&self, session_id: &str) -> String {
        self.task_by_session
            .get(session_id)
            .map(|task| task_cache_key(&task.task_id))
            .unwrap_or_else(|| format!("session:{session_id}"))
    }

    pub(super) fn rows_with_current_panes(&self, rows: &[ProgressRow]) -> Vec<ProgressRow> {
        rows.iter()
            .cloned()
            .map(|mut row| {
                let progress_row = match &mut row {
                    ProgressRow::Work(row) => Some((&row.session_id, &mut row.pane_id)),
                    ProgressRow::PullRequest(row)
                    | ProgressRow::ChecklistPullRequest { row, .. } => {
                        Some((&row.session_id, &mut row.pane_id))
                    }
                    ProgressRow::ImplementationHeader { .. }
                    | ProgressRow::UnmatchedDelegatesHeader { .. }
                    | ProgressRow::BatchHeader { .. }
                    | ProgressRow::PlannedPullRequest { .. } => None,
                };
                if let Some((session_id, pane_id)) = progress_row {
                    *pane_id = self
                        .sessions
                        .iter()
                        .find(|session| session.session_id == *session_id)
                        .and_then(|session| session.tmux_info.as_ref())
                        .map(|info| info.pane_id.clone());
                }
                row
            })
            .collect()
    }

    pub(super) fn persist_progress_cache(&self) {
        if let Err(error) = disk_cache::store(&self.progress_cache) {
            tracing::warn!("failed to store progress cache: {error}");
        }
    }
}

fn task_cache_key(task_id: &str) -> String {
    format!("task:{task_id}")
}
