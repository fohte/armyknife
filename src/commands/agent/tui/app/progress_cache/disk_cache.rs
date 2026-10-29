use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::super::progress::CachedProgress;
use super::task_keys_to_evict;
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
    let Some(cache) = read_cache_file(path)? else {
        return Ok(None);
    };
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
                        other_task_session_ids: vec![],
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

fn read_cache_file(path: &Path) -> Result<Option<ProgressCacheFile>> {
    match fs::read(path) {
        Ok(content) => {
            let cache: ProgressCacheFile = match serde_json::from_slice(&content) {
                Ok(cache) => cache,
                Err(error) => {
                    tracing::warn!(
                        "ignoring invalid progress cache {}: {error}",
                        path.display()
                    );
                    return Ok(None);
                }
            };
            if cache.format_version != FORMAT_VERSION {
                tracing::warn!(
                    "ignoring progress cache {} with unsupported format version {}",
                    path.display(),
                    cache.format_version
                );
                return Ok(None);
            }
            Ok(Some(cache))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => {
            Err(error).with_context(|| format!("failed to read cache file: {}", path.display()))
        }
    }
}

fn store_to(path: &Path, entries: &HashMap<String, CachedProgress>) -> Result<()> {
    store_to_with(path, entries, || {})
}

fn store_to_with(
    path: &Path,
    entries: &HashMap<String, CachedProgress>,
    after_merge: impl FnOnce(),
) -> Result<()> {
    let mut tasks: HashMap<String, PersistedProgress> = entries
        .iter()
        .filter_map(|(key, snapshot)| persisted_progress(key, snapshot))
        .collect();
    if tasks.is_empty() {
        return Ok(());
    }

    let parent = path
        .parent()
        .context("cache file path has no parent directory")?;
    fs::create_dir_all(parent)
        .with_context(|| format!("failed to create cache directory: {}", parent.display()))?;
    let _lock = lock_progress_cache(path)?;
    if let Some(existing) = read_cache_file(path)? {
        for (task_id, progress) in existing.tasks {
            merge_task(&mut tasks, task_id, progress);
        }
    }
    prune_persisted_tasks(&mut tasks);
    after_merge();

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

fn persisted_progress(key: &str, snapshot: &CachedProgress) -> Option<(String, PersistedProgress)> {
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
}

fn merge_task(
    tasks: &mut HashMap<String, PersistedProgress>,
    task_id: String,
    incoming: PersistedProgress,
) {
    let Some(existing) = tasks.get_mut(&task_id) else {
        tasks.insert(task_id, incoming);
        return;
    };

    let latest_attempt = existing
        .last_attempt_at
        .as_ref()
        .max(incoming.last_attempt_at.as_ref())
        .cloned();
    if incoming.fetched_at > existing.fetched_at {
        *existing = incoming;
    }
    existing.last_attempt_at = latest_attempt;
}

fn prune_persisted_tasks(tasks: &mut HashMap<String, PersistedProgress>) {
    let evicted = task_keys_to_evict(tasks.iter().map(|(task_id, progress)| {
        (
            task_id.clone(),
            progress
                .last_attempt_at
                .as_ref()
                .max(Some(&progress.fetched_at))
                .cloned(),
        )
    }));
    for task_id in evicted {
        tasks.remove(&task_id);
    }
}

fn lock_progress_cache(path: &Path) -> Result<File> {
    let file_name = path
        .file_name()
        .context("cache file path has no file name")?;
    let mut lock_name = file_name.to_os_string();
    lock_name.push(".lock");
    let lock_path = path.with_file_name(lock_name);
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)
        .with_context(|| {
            format!(
                "failed to open progress cache lock: {}",
                lock_path.display()
            )
        })?;
    file.lock()
        .with_context(|| format!("failed to lock progress cache: {}", lock_path.display()))?;
    Ok(file)
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
        PrProgressRow, ProgressMark, ProgressRow, ProgressSelectionKey, StageMarks, WorkProgressRow,
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
    #[case::invalid_json("not-json")]
    #[case::unsupported_version(r#"{"formatVersion":0,"tasks":{}}"#)]
    fn invalid_progress_cache_returns_none(cache_directory: TempDir, #[case] content: &str) {
        let path = cache_directory.path().join("watch-progress.json");
        fs::write(&path, content).unwrap();

        assert_eq!(load_from(&path).unwrap().map(|entries| entries.len()), None);
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
                checklist_item_id: "item-placeholder".to_string(),
                row: PrProgressRow {
                    session_id: "delegate-placeholder".to_string(),
                    repo: Some("sample-repo".to_string()),
                    title: "Placeholder change".to_string(),
                    number: Some(7),
                    url: Some("https://example.test/pull/7".to_string()),
                    pane_id: None,
                    is_ended: false,
                    lookup_unavailable: false,
                    pr_stage_info: None,
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
                    other_task_session_ids: vec![],
                }),
                Some(fetched_at),
                Some(last_attempt_at),
                None,
                0,
            ),
        );
    }

    #[test]
    fn cached_pr_row_defaults_repository_when_loading_legacy_data() {
        let row = serde_json::from_value::<PrProgressRow>(serde_json::json!({
            "session_id": "delegate-placeholder",
            "title": "Placeholder change",
            "number": 7,
            "url": "https://example.test/pull/7",
            "pane_id": null,
            "is_ended": true,
            "lookup_unavailable": false,
            "stages": {
                "implementation": "Done",
                "diff_review": "User",
                "pr_body": "Working",
                "ci": "Waiting",
                "merge": "NotStarted"
            }
        }))
        .unwrap();

        assert_eq!(
            row,
            PrProgressRow {
                session_id: "delegate-placeholder".to_string(),
                repo: None,
                title: "Placeholder change".to_string(),
                number: Some(7),
                url: Some("https://example.test/pull/7".to_string()),
                pane_id: None,
                is_ended: true,
                lookup_unavailable: false,
                pr_stage_info: None,
                stages: StageMarks {
                    implementation: ProgressMark::Done,
                    diff_review: ProgressMark::User,
                    pr_body: ProgressMark::Working,
                    ci: ProgressMark::Waiting,
                    merge: ProgressMark::NotStarted,
                },
            },
        );
    }

    #[test]
    fn legacy_checklist_row_defaults_item_id_and_selection_key() {
        let row = serde_json::from_value::<ProgressRow>(serde_json::json!({
            "ChecklistPullRequest": {
                "batch_id": "batch-placeholder",
                "row": {
                    "session_id": "delegate-placeholder",
                    "repo": null,
                    "title": "Placeholder change",
                    "number": 7,
                    "url": "https://example.test/pull/7",
                    "pane_id": null,
                    "is_ended": true,
                    "lookup_unavailable": false,
                    "stages": {
                        "implementation": "Done",
                        "diff_review": "User",
                        "pr_body": "Working",
                        "ci": "Waiting",
                        "merge": "NotStarted"
                    }
                }
            }
        }))
        .unwrap();

        assert_eq!(
            (row.clone(), row.selection_key()),
            (
                ProgressRow::ChecklistPullRequest {
                    batch_id: "batch-placeholder".to_string(),
                    checklist_item_id: String::new(),
                    row: PrProgressRow {
                        session_id: "delegate-placeholder".to_string(),
                        repo: None,
                        title: "Placeholder change".to_string(),
                        number: Some(7),
                        url: Some("https://example.test/pull/7".to_string()),
                        pane_id: None,
                        is_ended: true,
                        lookup_unavailable: false,
                        pr_stage_info: None,
                        stages: StageMarks {
                            implementation: ProgressMark::Done,
                            diff_review: ProgressMark::User,
                            pr_body: ProgressMark::Working,
                            ci: ProgressMark::Waiting,
                            merge: ProgressMark::NotStarted,
                        },
                    },
                },
                Some(ProgressSelectionKey::ChecklistItem {
                    batch_id: "batch-placeholder".to_string(),
                    item_id: "delegate-placeholder".to_string(),
                }),
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
                let last_attempt_at = if index == 128 { timestamp } else { fetched_at };
                (
                    format!("task:{task_id}"),
                    cached_progress(fetched_at, last_attempt_at),
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

    #[rstest]
    fn progress_cache_store_merges_stale_writer_data(cache_directory: TempDir) {
        let path = cache_directory.path().join("watch-progress.json");
        let t0 = Utc.timestamp_opt(1_800_000_000, 0).single().unwrap();
        let t1 = t0 + Duration::seconds(10);
        let t2 = t0 + Duration::seconds(20);
        let t3 = t0 + Duration::seconds(30);

        let initial = HashMap::from([
            (
                "task:task-alpha".to_string(),
                cached_progress_with_note(t0, t0, "alpha-old"),
            ),
            (
                "task:task-bravo".to_string(),
                cached_progress_with_note(t0, t0, "bravo-old"),
            ),
        ]);
        store_to(&path, &initial).unwrap();

        let writer_a_snapshot = HashMap::from([
            (
                "task:task-alpha".to_string(),
                cached_progress_with_note(t1, t1, "alpha-new"),
            ),
            (
                "task:task-charlie".to_string(),
                cached_progress_with_note(t1, t1, "charlie-new"),
            ),
            (
                "task:task-bravo".to_string(),
                cached_progress_with_note(t0, t0, "bravo-old"),
            ),
        ]);
        store_to(&path, &writer_a_snapshot).unwrap();

        let stale_writer_b_snapshot = HashMap::from([
            (
                "task:task-alpha".to_string(),
                cached_progress_with_note(t0, t3, "alpha-old"),
            ),
            (
                "task:task-bravo".to_string(),
                cached_progress_with_note(t2, t2, "bravo-new"),
            ),
        ]);
        store_to(&path, &stale_writer_b_snapshot).unwrap();

        let stored = load_from(&path).unwrap().unwrap();
        let actual = stored
            .into_iter()
            .map(|(task_id, snapshot)| {
                (
                    task_id,
                    (snapshot.fetched_at, snapshot.last_attempt_at, snapshot.rows),
                )
            })
            .collect::<HashMap<_, _>>();
        let expected = HashMap::from([
            (
                "task-alpha".to_string(),
                (
                    Some(t1),
                    Some(t3),
                    cached_progress_with_note(t1, t1, "alpha-new").rows,
                ),
            ),
            (
                "task-charlie".to_string(),
                (
                    Some(t1),
                    Some(t1),
                    cached_progress_with_note(t1, t1, "charlie-new").rows,
                ),
            ),
            (
                "task-bravo".to_string(),
                (
                    Some(t2),
                    Some(t2),
                    cached_progress_with_note(t2, t2, "bravo-new").rows,
                ),
            ),
        ]);

        assert_eq!(actual, expected);
    }

    #[rstest]
    fn progress_cache_store_serializes_concurrent_writers(cache_directory: TempDir) {
        let path = cache_directory.path().join("watch-progress.json");
        let t0 = Utc.timestamp_opt(1_800_000_000, 0).single().unwrap();
        let t1 = t0 + Duration::seconds(10);
        let t2 = t0 + Duration::seconds(20);
        store_to(
            &path,
            &HashMap::from([(
                "task:task-base".to_string(),
                cached_progress_with_note(t0, t0, "base"),
            )]),
        )
        .unwrap();

        let (first_merged_tx, first_merged_rx) = std::sync::mpsc::channel();
        let (release_first_tx, release_first_rx) = std::sync::mpsc::channel();
        let first_path = path.clone();
        let first_entries = HashMap::from([(
            "task:task-alpha".to_string(),
            cached_progress_with_note(t1, t1, "alpha-new"),
        )]);
        let first_writer = std::thread::spawn(move || {
            store_to_with(&first_path, &first_entries, || {
                first_merged_tx.send(()).unwrap();
                release_first_rx.recv().unwrap();
            })
            .unwrap();
        });
        first_merged_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();

        let (second_started_tx, second_started_rx) = std::sync::mpsc::channel();
        let (second_merged_tx, second_merged_rx) = std::sync::mpsc::channel();
        let (release_second_tx, release_second_rx) = std::sync::mpsc::channel();
        let second_path = path.clone();
        let second_entries = HashMap::from([(
            "task:task-bravo".to_string(),
            cached_progress_with_note(t2, t2, "bravo-new"),
        )]);
        let second_writer = std::thread::spawn(move || {
            second_started_tx.send(()).unwrap();
            store_to_with(&second_path, &second_entries, || {
                second_merged_tx.send(()).unwrap();
                release_second_rx.recv().unwrap();
            })
            .unwrap();
        });
        second_started_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        let second_merged_before_first_write = second_merged_rx
            .recv_timeout(std::time::Duration::from_millis(100))
            .is_ok();

        release_first_tx.send(()).unwrap();
        first_writer.join().unwrap();
        if !second_merged_before_first_write {
            second_merged_rx
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap();
        }
        release_second_tx.send(()).unwrap();
        second_writer.join().unwrap();

        let stored = load_from(&path).unwrap().unwrap();
        let actual = stored
            .into_iter()
            .map(|(task_id, snapshot)| {
                (
                    task_id,
                    (snapshot.fetched_at, snapshot.last_attempt_at, snapshot.rows),
                )
            })
            .collect::<HashMap<_, _>>();
        let expected = HashMap::from([
            (
                "task-base".to_string(),
                (
                    Some(t0),
                    Some(t0),
                    cached_progress_with_note(t0, t0, "base").rows,
                ),
            ),
            (
                "task-alpha".to_string(),
                (
                    Some(t1),
                    Some(t1),
                    cached_progress_with_note(t1, t1, "alpha-new").rows,
                ),
            ),
            (
                "task-bravo".to_string(),
                (
                    Some(t2),
                    Some(t2),
                    cached_progress_with_note(t2, t2, "bravo-new").rows,
                ),
            ),
        ]);

        assert_eq!(actual, expected);
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
                    checklist_item_id: "item-placeholder".to_string(),
                    row: PrProgressRow {
                        session_id: "delegate-placeholder".to_string(),
                        repo: Some("sample-repo".to_string()),
                        title: "Placeholder change".to_string(),
                        number: Some(7),
                        url: Some("https://example.test/pull/7".to_string()),
                        pane_id: Some("%998".to_string()),
                        is_ended: false,
                        lookup_unavailable: false,
                        pr_stage_info: None,
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
                other_task_session_ids: vec![],
            }),
            delegated_sessions: Vec::new(),
            fetched_at: Some(fetched_at),
            last_attempt_at: Some(last_attempt_at),
        }
    }

    fn cached_progress_with_note(
        fetched_at: DateTime<Utc>,
        last_attempt_at: DateTime<Utc>,
        note: &str,
    ) -> CachedProgress {
        let mut progress = cached_progress(fetched_at, last_attempt_at);
        let Some(ProgressRow::Work(row)) = progress.rows.first_mut() else {
            unreachable!();
        };
        row.note = Some(note.to_string());
        row.pane_id = None;
        if let Some(ProgressRow::ChecklistPullRequest { row, .. }) = progress.rows.get_mut(1) {
            row.pane_id = None;
        }
        progress
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
