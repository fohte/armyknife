use std::collections::HashMap;

use chrono::{DateTime, Duration as ChronoDuration, Utc};

use super::super::progress::ProgressFetchScope;
use super::super::progress::ProgressRow;
use super::super::progress_batches::ImplementationPlan;
use super::App;
use super::progress::CachedProgress;

mod disk_cache;

const MAX_CACHED_TASKS: usize = 128;

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
        self.progress_cache_dirty |= prune_task_entries(&mut self.progress_cache);
    }

    #[cfg(test)]
    pub(in crate::commands::agent::tui) fn progress_refresh_due(
        &self,
        session_id: &str,
        now: DateTime<Utc>,
        max_age: std::time::Duration,
    ) -> bool {
        self.progress_refresh_wait(session_id, now, max_age)
            .is_none_or(|wait| wait.is_zero())
    }

    pub(in crate::commands::agent::tui) fn progress_refresh_wait(
        &self,
        session_id: &str,
        now: DateTime<Utc>,
        max_age: std::time::Duration,
    ) -> Option<std::time::Duration> {
        let last_attempt = self
            .cached_progress(session_id)
            .and_then(|snapshot| snapshot.last_attempt_at.or(snapshot.fetched_at));
        let last_attempt = last_attempt?;
        let max_age_millis = i64::try_from(max_age.as_millis()).unwrap_or(i64::MAX);
        let elapsed = now.signed_duration_since(last_attempt);
        if elapsed >= ChronoDuration::milliseconds(max_age_millis) {
            Some(std::time::Duration::ZERO)
        } else {
            Some(max_age.saturating_sub(elapsed.to_std().unwrap_or_default()))
        }
    }

    pub(in crate::commands::agent::tui) fn mark_progress_refresh_started(
        &mut self,
        session_id: &str,
        attempted_at: DateTime<Utc>,
    ) {
        if let Some(snapshot) = self.cached_progress_mut(session_id) {
            snapshot.last_attempt_at = Some(attempted_at);
        } else {
            self.set_cached_progress(
                session_id,
                CachedProgress {
                    rows: Vec::new(),
                    error: None,
                    implementation: None,
                    request_scope: None,
                    delegated_sessions: Vec::new(),
                    fetched_at: None,
                    last_attempt_at: Some(attempted_at),
                },
            );
        }
    }

    pub(super) fn progress_cache_key(&self, session_id: &str) -> String {
        self.task_by_session
            .get(session_id)
            .map(|task| task_cache_key(&task.task_id))
            .unwrap_or_else(|| format!("session:{session_id}"))
    }

    pub(super) fn cached_progress(&self, session_id: &str) -> Option<&CachedProgress> {
        let key = self.progress_cache_key(session_id);
        self.progress_cache.get(&key)
    }

    pub(super) fn cached_progress_mut(&mut self, session_id: &str) -> Option<&mut CachedProgress> {
        let key = self.progress_cache_key(session_id);
        self.progress_cache.get_mut(&key)
    }

    pub(super) fn set_cached_progress(&mut self, session_id: &str, snapshot: CachedProgress) {
        let key = self.progress_cache_key(session_id);
        let changed = self.progress_cache.get(&key).is_none_or(|cached| {
            cached.rows != snapshot.rows
                || cached.implementation != snapshot.implementation
                || cached.fetched_at != snapshot.fetched_at
                || cached.last_attempt_at != snapshot.last_attempt_at
        });
        if changed && key.starts_with("task:") && snapshot.fetched_at.is_some() {
            self.progress_cache_dirty = true;
        }
        self.progress_cache.insert(key, snapshot);
        self.progress_cache_dirty |= prune_task_entries(&mut self.progress_cache);
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
}

pub(in crate::commands::agent::tui) fn persist_progress_cache(app: &mut App) {
    if !app.progress_cache_dirty {
        return;
    }
    app.progress_cache_dirty = false;
    if let Err(error) = disk_cache::store(&app.progress_cache) {
        tracing::warn!("failed to store progress cache: {error}");
    }
}

fn prune_task_entries(entries: &mut HashMap<String, CachedProgress>) -> bool {
    let evicted = task_keys_to_evict(
        entries
            .iter()
            .filter(|(key, _)| key.starts_with("task:"))
            .map(|(key, snapshot)| {
                (
                    key.clone(),
                    snapshot
                        .last_attempt_at
                        .as_ref()
                        .max(snapshot.fetched_at.as_ref())
                        .cloned(),
                )
            }),
    );
    if evicted.is_empty() {
        return false;
    }
    for key in evicted {
        entries.remove(&key);
    }
    true
}

fn task_keys_to_evict<K: Ord>(
    task_times: impl IntoIterator<Item = (K, Option<DateTime<Utc>>)>,
) -> Vec<K> {
    let mut task_times = task_times.into_iter().collect::<Vec<_>>();
    task_times.sort_by(|(left_key, left_time), (right_key, right_time)| {
        right_time
            .cmp(left_time)
            .then_with(|| left_key.cmp(right_key))
    });
    task_times
        .into_iter()
        .skip(MAX_CACHED_TASKS)
        .map(|(key, _)| key)
        .collect()
}

fn task_cache_key(task_id: &str) -> String {
    format!("task:{task_id}")
}
