use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::super::progress::CachedProgress;
use super::MAX_CACHED_TASKS;
use crate::commands::agent::tui::progress::{ProgressFetchScope, ProgressRow};
use crate::commands::agent::tui::progress_batches::ImplementationPlan;
use crate::shared::cache;

const FORMAT_VERSION: u8 = 1;

fn path() -> Option<PathBuf> {
    cache::base_dir().map(|dir| dir.join("agent").join("watch-progress.json"))
}

pub(super) fn load() -> Result<Option<HashMap<String, CachedProgress>>> {
    let Some(path) = path() else {
        return Ok(None);
    };
    load_from(&path)
}

pub(super) fn store(entries: &HashMap<String, CachedProgress>) -> Result<()> {
    let path = path().context("cache directory is unavailable")?;
    store_to(&path, entries)
}

fn load_from(path: &Path) -> Result<Option<HashMap<String, CachedProgress>>> {
    match fs::read(path) {
        Ok(content) => {
            let cache: ProgressCacheFile = serde_json::from_slice(&content)
                .with_context(|| format!("failed to parse cache file: {}", path.display()))?;
            if cache.format_version != FORMAT_VERSION {
                bail!(
                    "unsupported progress cache format version: {}",
                    cache.format_version
                );
            }
            Ok(Some(
                cache
                    .tasks
                    .into_iter()
                    .map(|(task_id, progress)| {
                        let snapshot = CachedProgress {
                            rows: progress.rows,
                            error: None,
                            implementation: progress.implementation,
                            request_scope: Some(ProgressFetchScope {
                                task_id: Some(task_id.clone()),
                                session_ids: None,
                            }),
                            delegated_sessions: Vec::new(),
                            fetched_at: Some(progress.fetched_at),
                            last_attempt_at: progress.last_attempt_at,
                        };
                        (task_id, snapshot)
                    })
                    .collect(),
            ))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => {
            Err(error).with_context(|| format!("failed to read cache file: {}", path.display()))
        }
    }
}

fn store_to(path: &Path, entries: &HashMap<String, CachedProgress>) -> Result<()> {
    let mut tasks: HashMap<String, PersistedProgress> = entries
        .iter()
        .filter_map(|(key, snapshot)| {
            let task_id = key.strip_prefix("task:")?;
            let fetched_at = snapshot.fetched_at?;
            Some((
                task_id.to_string(),
                PersistedProgress {
                    fetched_at,
                    last_attempt_at: snapshot.last_attempt_at,
                    rows: snapshot.rows.clone(),
                    implementation: snapshot.implementation.clone(),
                },
            ))
        })
        .collect();
    if tasks.is_empty() {
        return Ok(());
    }
    let mut task_times = tasks
        .iter()
        .map(|(task_id, progress)| {
            (
                task_id.clone(),
                progress
                    .last_attempt_at
                    .as_ref()
                    .unwrap_or(&progress.fetched_at)
                    .to_owned(),
            )
        })
        .collect::<Vec<_>>();
    task_times.sort_by(|(left_id, left_time), (right_id, right_time)| {
        right_time
            .cmp(left_time)
            .then_with(|| left_id.cmp(right_id))
    });
    for (task_id, _) in task_times.into_iter().skip(MAX_CACHED_TASKS) {
        tasks.remove(&task_id);
    }

    let parent = path
        .parent()
        .context("cache file path has no parent directory")?;
    fs::create_dir_all(parent)
        .with_context(|| format!("failed to create cache directory: {}", parent.display()))?;
    let cache = ProgressCacheFile {
        format_version: FORMAT_VERSION,
        tasks,
    };

    let mut temp_file = tempfile::NamedTempFile::new_in(parent).with_context(|| {
        format!(
            "failed to create temporary cache file in {}",
            parent.display()
        )
    })?;
    serde_json::to_writer(&mut temp_file, &cache).context("failed to serialize progress cache")?;
    temp_file.write_all(b"\n")?;
    temp_file.as_file().sync_all()?;
    temp_file
        .persist(path)
        .with_context(|| format!("failed to replace cache file: {}", path.display()))?;
    Ok(())
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProgressCacheFile {
    format_version: u8,
    tasks: HashMap<String, PersistedProgress>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct PersistedProgress {
    fetched_at: DateTime<Utc>,
    last_attempt_at: Option<DateTime<Utc>>,
    rows: Vec<ProgressRow>,
    implementation: Option<ImplementationPlan>,
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use chrono::{Duration, TimeZone};
    use rstest::{fixture, rstest};
    use tempfile::TempDir;

    use super::*;
    use crate::commands::agent::tui::progress::{
        PrProgressRow, ProgressMark, ProgressRow, StageMarks, WorkProgressRow,
    };
    use crate::commands::agent::tui::progress_batches::build_implementation_plan;
    use crate::infra::github::{PrCiState, PrInfo, PrProgressInfo, PrState};
    use crate::infra::tq::{TqChecklist, TqChecklistItem, TqGithubLink};

    #[fixture]
    fn cache_directory() -> TempDir {
        tempfile::tempdir().unwrap()
    }

    #[rstest]
    fn missing_progress_cache_returns_none(cache_directory: TempDir) {
        assert_eq!(
            load_from(&cache_directory.path().join("missing.json"))
                .unwrap()
                .map(|entries| entries.len()),
            None
        );
    }

    #[rstest]
    fn progress_cache_round_trip_keeps_checklist_and_pr_rows(cache_directory: TempDir) {
        let path = cache_directory.path().join("watch-progress.json");
        let fetched_at = Utc.timestamp_opt(1_800_000_000, 0).single().unwrap();
        let last_attempt_at = Utc.timestamp_opt(1_800_000_010, 0).single().unwrap();
        let progress = cached_progress(fetched_at, last_attempt_at);

        store_to(
            &path,
            &HashMap::from([("task:task-placeholder".to_string(), progress)]),
        )
        .unwrap();
        let loaded = load_from(&path).unwrap().unwrap();
        let snapshot = loaded.get("task-placeholder").unwrap();
        let expected_rows = vec![
            ProgressRow::Work(WorkProgressRow {
                session_id: "session-placeholder".to_string(),
                role: "implementation".to_string(),
                mark: ProgressMark::Working,
                status_label: Some("tool".to_string()),
                note: Some("cached note".to_string()),
                pane_id: None,
            }),
            ProgressRow::ChecklistPullRequest {
                batch_id: "batch-placeholder".to_string(),
                row: PrProgressRow {
                    session_id: "delegate-placeholder".to_string(),
                    title: "Placeholder change".to_string(),
                    number: Some(7),
                    url: Some("https://example.test/pull/7".to_string()),
                    pane_id: None,
                    is_ended: false,
                    lookup_unavailable: false,
                    stages: StageMarks {
                        implementation: ProgressMark::Done,
                        diff_review: ProgressMark::User,
                        pr_body: ProgressMark::Working,
                        ci: ProgressMark::Waiting,
                        merge: ProgressMark::NotStarted,
                    },
                },
            },
        ];

        assert_eq!(
            (
                snapshot.rows.clone(),
                snapshot.implementation.clone(),
                snapshot.request_scope.clone(),
                snapshot.fetched_at,
                snapshot.last_attempt_at,
                snapshot.error.clone(),
                snapshot.delegated_sessions.len(),
            ),
            (
                expected_rows,
                Some(implementation_plan()),
                Some(ProgressFetchScope {
                    task_id: Some("task-placeholder".to_string()),
                    session_ids: None,
                }),
                Some(fetched_at),
                Some(last_attempt_at),
                None,
                0,
            ),
        );
    }

    #[rstest]
    fn progress_cache_store_retains_the_most_recent_128_tasks(cache_directory: TempDir) {
        let path = cache_directory.path().join("watch-progress.json");
        let timestamp = Utc.timestamp_opt(1_800_000_000, 0).single().unwrap();
        let entries = (0..129)
            .map(|index| {
                let fetched_at = timestamp + Duration::seconds(index);
                let task_id = format!("task-placeholder-{index}");
                (
                    format!("task:{task_id}"),
                    cached_progress(fetched_at, fetched_at),
                )
            })
            .collect::<HashMap<_, _>>();

        store_to(&path, &entries).unwrap();
        let stored = load_from(&path).unwrap().unwrap();

        assert_eq!(
            (
                stored.len(),
                stored.contains_key("task-placeholder-128"),
                stored.contains_key("task-placeholder-0"),
            ),
            (128, true, false),
        );
    }

    fn cached_progress(
        fetched_at: DateTime<Utc>,
        last_attempt_at: DateTime<Utc>,
    ) -> CachedProgress {
        CachedProgress {
            rows: vec![
                ProgressRow::Work(WorkProgressRow {
                    session_id: "session-placeholder".to_string(),
                    role: "implementation".to_string(),
                    mark: ProgressMark::Working,
                    status_label: Some("tool".to_string()),
                    note: Some("cached note".to_string()),
                    pane_id: Some("%999".to_string()),
                }),
                ProgressRow::ChecklistPullRequest {
                    batch_id: "batch-placeholder".to_string(),
                    row: PrProgressRow {
                        session_id: "delegate-placeholder".to_string(),
                        title: "Placeholder change".to_string(),
                        number: Some(7),
                        url: Some("https://example.test/pull/7".to_string()),
                        pane_id: Some("%998".to_string()),
                        is_ended: false,
                        lookup_unavailable: false,
                        stages: StageMarks {
                            implementation: ProgressMark::Done,
                            diff_review: ProgressMark::User,
                            pr_body: ProgressMark::Working,
                            ci: ProgressMark::Waiting,
                            merge: ProgressMark::NotStarted,
                        },
                    },
                },
            ],
            error: Some("temporary error".to_string()),
            implementation: Some(implementation_plan()),
            request_scope: Some(ProgressFetchScope {
                task_id: Some("task-placeholder".to_string()),
                session_ids: Some(vec!["session-placeholder".to_string()]),
            }),
            delegated_sessions: Vec::new(),
            fetched_at: Some(fetched_at),
            last_attempt_at: Some(last_attempt_at),
        }
    }

    fn implementation_plan() -> ImplementationPlan {
        let pull_request = PrProgressInfo {
            pull_request: PrInfo {
                number: 7,
                title: "Placeholder change".to_string(),
                state: PrState::Open,
                url: "https://example.test/pull/7".to_string(),
            },
            ci_state: PrCiState::InProgress,
        };
        let checklist = TqChecklist {
            id: "checklist-placeholder".to_string(),
            name: "Implementation".to_string(),
            items: vec![TqChecklistItem {
                id: "batch-placeholder".to_string(),
                content: "Batch placeholder".to_string(),
                checked_at: None,
                github_link_id: None,
                children: vec![TqChecklistItem {
                    id: "item-placeholder".to_string(),
                    content: "PR 7: Placeholder change".to_string(),
                    checked_at: None,
                    github_link_id: Some("link-placeholder".to_string()),
                    children: Vec::new(),
                }],
            }],
        };
        let github_link = TqGithubLink {
            id: "link-placeholder".to_string(),
            kind: Some("pull_request".to_string()),
            owner: Some("placeholder-org".to_string()),
            repo: Some("placeholder-project".to_string()),
            number: Some(7),
            title: Some("Placeholder change".to_string()),
            url: Some("https://example.test/pull/7".to_string()),
            state: Some("open".to_string()),
        };
        let pull_requests = HashMap::from([(
            (
                "placeholder-org".to_string(),
                "placeholder-project".to_string(),
                7,
            ),
            Some(pull_request),
        )]);

        build_implementation_plan(&checklist, &[github_link], &[], &pull_requests, false)
    }
}
