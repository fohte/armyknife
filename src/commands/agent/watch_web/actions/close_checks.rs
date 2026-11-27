use std::future::Future;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Serialize;

use crate::commands::agent::types::Session;
use crate::infra::git::cmd::run_git;
use crate::infra::git::get_main_branch_for_repo;
use crate::infra::process::ProcessInPath;
use crate::infra::process::find_non_shell_processes_in_path;
use crate::shared::cleanup::{self, WorktreeProcessGuardError};
use crate::shared::worktree;
use crate::shared::worktree_delete::{self, MergeGuardStatus, WorktreeDeleteTarget};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct CloseChecks {
    path: String,
    files: Vec<String>,
    changes: Vec<CloseChange>,
    commits: Vec<CloseCommit>,
    branch: Option<String>,
    merged: Option<bool>,
    reason: Option<String>,
    processes: Vec<CloseProcess>,
    process_inspection_unavailable: bool,
}

#[derive(Debug, Serialize)]
struct CloseChange {
    status: String,
    path: String,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
pub(super) struct CloseCommit {
    sha: String,
    subject: String,
}

#[derive(Debug, Serialize)]
struct CloseProcess {
    pid: u32,
    name: String,
}

async fn inspect_close_merge_status(
    branch: String,
    status: impl Future<Output = crate::infra::git::MergeStatus>,
) -> Option<MergeGuardStatus> {
    tokio::time::timeout(Duration::from_secs(5), status)
        .await
        .ok()
        .map(|status| worktree_delete::classify_merge_guard_status(branch, status))
}

impl CloseChecks {
    pub(super) async fn inspect(session: &Session, filter_stopped_agent: bool) -> Self {
        let session_path = session.cwd.clone();
        let target_path = session_path.clone();
        let target = tokio::task::spawn_blocking(move || {
            worktree_delete::resolve_target(&target_path)
                .ok()
                .map(|target| {
                    let branch = worktree_delete::resolve_merge_guard_branch(
                        &target.main_repo,
                        target.branch_name.as_deref(),
                    );
                    (target, branch)
                })
        })
        .await
        .ok()
        .flatten();
        match target {
            Some((target, branch)) => {
                Self::inspect_target(target, branch, session.clone(), filter_stopped_agent).await
            }
            None => {
                Self::inspect_without_target(session_path, session.clone(), filter_stopped_agent)
                    .await
            }
        }
    }

    async fn inspect_target(
        target: WorktreeDeleteTarget,
        branch: Option<String>,
        session: Session,
        filter_stopped_agent: bool,
    ) -> Self {
        let worktree_path = target.worktree_path.clone();
        let repo = target.main_repo;
        let merge_repo = repo.clone();
        let commits_repo = repo.clone();
        let process_session = session;
        Self::inspect_with(
            worktree_path,
            branch,
            |path| worktree::uncommitted_changes(path).unwrap_or_default(),
            move |branch| async move {
                // The preview checks local refs; close fetches and revalidates before deletion.
                let status_branch = branch.clone();
                inspect_close_merge_status(branch, async move {
                    worktree_delete::get_merge_status_for_branch_from_local_refs(
                        &merge_repo,
                        &status_branch,
                    )
                    .await
                })
                .await
            },
            move |branch| {
                let Ok(default_branch) = get_main_branch_for_repo(&commits_repo) else {
                    return Vec::new();
                };
                let range = format!("origin/{default_branch}..refs/heads/{branch}");
                run_git(
                    commits_repo.workdir(),
                    ["log", "--format=%h%x00%s", range.as_str()],
                )
                .ok()
                .map(|log| parse_commits(&log))
                .unwrap_or_default()
            },
            move |path| {
                check_process_guard_for_close(
                    path,
                    &process_session,
                    filter_stopped_agent,
                    |session| {
                        crate::commands::agent::close::resolve_agent_pid_for_session(session)
                            .ok()
                            .flatten()
                    },
                    find_non_shell_processes_in_path,
                )
            },
        )
        .await
    }

    async fn inspect_without_target(
        worktree_path: PathBuf,
        session: Session,
        filter_stopped_agent: bool,
    ) -> Self {
        let process_session = session;
        Self::inspect_with(
            worktree_path,
            None,
            |path| worktree::uncommitted_changes(path).unwrap_or_default(),
            |_| async { None },
            |_| Vec::new(),
            move |path| {
                check_process_guard_for_close(
                    path,
                    &process_session,
                    filter_stopped_agent,
                    |session| {
                        crate::commands::agent::close::resolve_agent_pid_for_session(session)
                            .ok()
                            .flatten()
                    },
                    find_non_shell_processes_in_path,
                )
            },
        )
        .await
    }

    async fn inspect_with<Changes, Merge, MergeFuture, Commits, Processes>(
        worktree_path: PathBuf,
        branch: Option<String>,
        inspect_changes: Changes,
        inspect_merge_status: Merge,
        inspect_commits: Commits,
        inspect_process_guard: Processes,
    ) -> Self
    where
        Changes: FnOnce(&Path) -> Vec<worktree::UncommittedChange> + Send + 'static,
        Merge: FnOnce(String) -> MergeFuture,
        MergeFuture: Future<Output = Option<MergeGuardStatus>>,
        Commits: FnOnce(String) -> Vec<CloseCommit> + Send + 'static,
        Processes:
            FnOnce(&Path) -> Result<Vec<ProcessInPath>, WorktreeProcessGuardError> + Send + 'static,
    {
        let inspection_path = worktree_path.clone();
        let files_and_processes = tokio::task::spawn_blocking(move || {
            let changes = inspect_changes(&inspection_path);
            let processes = inspect_process_guard(&inspection_path);
            (changes, processes)
        });
        let merge_status = match branch.clone() {
            Some(branch) => inspect_merge_status(branch).await,
            None => None,
        };
        let commits = match merge_status.as_ref() {
            Some(status) if !status.should_cleanup => {
                let branch = status.branch.clone();
                tokio::task::spawn_blocking(move || inspect_commits(branch))
                    .await
                    .unwrap_or_default()
            }
            _ => Vec::new(),
        };
        let (changes, processes) = files_and_processes
            .await
            .map(|(changes, processes)| (changes, Some(processes)))
            .unwrap_or_default();

        Self::from_observations(
            worktree_path,
            changes,
            commits,
            branch,
            merge_status,
            processes,
        )
    }

    pub(super) fn from_observations(
        worktree_path: PathBuf,
        changes: Vec<worktree::UncommittedChange>,
        commits: Vec<CloseCommit>,
        branch: Option<String>,
        merge_status: Option<MergeGuardStatus>,
        process_inspection: Option<Result<Vec<ProcessInPath>, WorktreeProcessGuardError>>,
    ) -> Self {
        let (branch, merged, reason) = match merge_status {
            Some(status) => {
                let reason = (!status.should_cleanup).then(|| status.status.reason().to_string());
                (Some(status.branch), Some(status.should_cleanup), reason)
            }
            None => (branch, None, None),
        };
        let (processes, process_inspection_unavailable) = match process_inspection {
            Some(Ok(processes)) => (processes, false),
            Some(Err(WorktreeProcessGuardError::ActiveProcesses { processes, .. })) => {
                (processes, false)
            }
            Some(Err(WorktreeProcessGuardError::InspectionUnavailable { .. })) | None => {
                (Vec::new(), true)
            }
        };
        let processes = processes
            .into_iter()
            .map(|process| CloseProcess {
                pid: process.pid,
                name: process.comm,
            })
            .collect();
        let files = changes.iter().map(|change| change.path.clone()).collect();
        let changes = changes
            .into_iter()
            .map(|change| CloseChange {
                status: change.status,
                path: change.path,
            })
            .collect();

        Self {
            path: worktree_path.display().to_string(),
            files,
            changes,
            commits,
            branch,
            merged,
            reason,
            processes,
            process_inspection_unavailable,
        }
    }
}

fn parse_commits(log: &str) -> Vec<CloseCommit> {
    log.lines()
        .filter_map(|line| {
            let (sha, subject) = line.split_once('\0')?;
            Some(CloseCommit {
                sha: sha.to_string(),
                subject: subject.to_string(),
            })
        })
        .collect()
}

pub(super) fn check_process_guard_for_close(
    worktree_path: &Path,
    session: &Session,
    filter_stopped_agent: bool,
    resolve_agent_pid: impl FnOnce(&Session) -> Option<u32>,
    detect_processes: impl FnOnce(&Path) -> Option<Vec<ProcessInPath>>,
) -> Result<Vec<ProcessInPath>, WorktreeProcessGuardError> {
    let stopped_agent_pid = filter_stopped_agent
        .then(|| resolve_agent_pid(session))
        .flatten();
    cleanup::check_process_guard(worktree_path, false, |path| {
        detect_processes(path).map(|processes| {
            processes
                .into_iter()
                .filter(|process| Some(process.pid) != stopped_agent_pid)
                .collect()
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::git::MergeStatus;
    use serde_json::json;

    #[test]
    fn serializes_all_close_guard_observations() {
        let checks = CloseChecks::from_observations(
            PathBuf::from("/fictional/worktree"),
            vec![change("M", "src/fictional.rs")],
            vec![commit("abc1234", "preserve fictional work")],
            Some("feature/fictional".to_string()),
            Some(worktree_delete::classify_merge_guard_status(
                "feature/fictional".to_string(),
                MergeStatus::NotMerged {
                    reason: "pull request is open".to_string(),
                },
            )),
            Some(Err(WorktreeProcessGuardError::ActiveProcesses {
                worktree_path: PathBuf::from("/fictional/worktree"),
                process_list: "12345 (fictional-worker)".to_string(),
                processes: vec![ProcessInPath {
                    pid: 12345,
                    comm: "fictional-worker".to_string(),
                }],
            })),
        );

        assert_eq!(
            serde_json::to_value(checks).unwrap(),
            json!({
                "path": "/fictional/worktree",
                "files": ["src/fictional.rs"],
                "changes": [{ "status": "M", "path": "src/fictional.rs" }],
                "commits": [{ "sha": "abc1234", "subject": "preserve fictional work" }],
                "branch": "feature/fictional",
                "merged": false,
                "reason": "pull request is open",
                "processes": [{ "pid": 12345, "name": "fictional-worker" }],
                "processInspectionUnavailable": false,
            }),
        );
    }

    #[rstest::rstest]
    #[case::merged(crate::infra::git::MergeStatus::Merged { reason: "fictional change merged".to_string() })]
    #[case::closed(crate::infra::git::MergeStatus::Closed { reason: "fictional change closed".to_string() })]
    fn merged_uses_the_worktree_cleanup_decision(#[case] status: MergeStatus) {
        let checks = CloseChecks::from_observations(
            PathBuf::from("/fictional/worktree"),
            Vec::new(),
            Vec::new(),
            Some("feature/fictional".to_string()),
            Some(worktree_delete::classify_merge_guard_status(
                "feature/fictional".to_string(),
                status,
            )),
            Some(Ok(Vec::new())),
        );

        assert_eq!(
            serde_json::to_value(checks).unwrap(),
            json!({
                "path": "/fictional/worktree",
                "files": [],
                "changes": [],
                "commits": [],
                "branch": "feature/fictional",
                "merged": true,
                "reason": null,
                "processes": [],
                "processInspectionUnavailable": false,
            }),
        );
    }

    #[tokio::test]
    async fn no_unique_commits_are_cleanup_eligible_in_watch_close_check() {
        let checks = CloseChecks::inspect_with(
            PathBuf::from("/fictional/worktree"),
            Some("feature/fictional".to_string()),
            |_| Vec::new(),
            |branch| async move {
                inspect_close_merge_status(branch, async {
                    MergeStatus::NoUniqueCommits {
                        reason: "No commits unique to branch".to_string(),
                    }
                })
                .await
            },
            |_| vec![commit("abc1234", "must not be listed")],
            |_| Ok(Vec::new()),
        )
        .await;

        assert_eq!(
            serde_json::to_value(checks).unwrap(),
            json!({
                "path": "/fictional/worktree",
                "files": [],
                "changes": [],
                "commits": [],
                "branch": "feature/fictional",
                "merged": true,
                "reason": null,
                "processes": [],
                "processInspectionUnavailable": false,
            }),
        );
    }

    #[test]
    fn missing_observations_use_the_documented_null_and_empty_values() {
        let checks = CloseChecks::from_observations(
            PathBuf::from("/fictional/detached-worktree"),
            Vec::new(),
            Vec::new(),
            None,
            None,
            Some(Err(WorktreeProcessGuardError::InspectionUnavailable {
                worktree_path: PathBuf::from("/fictional/detached-worktree"),
            })),
        );

        assert_eq!(
            serde_json::to_value(checks).unwrap(),
            json!({
                "path": "/fictional/detached-worktree",
                "files": [],
                "changes": [],
                "commits": [],
                "branch": null,
                "merged": null,
                "reason": null,
                "processes": [],
                "processInspectionUnavailable": true,
            }),
        );
    }

    #[test]
    fn unavailable_merge_status_keeps_the_known_branch_without_a_reason() {
        let checks = CloseChecks::from_observations(
            PathBuf::from("/fictional/worktree"),
            Vec::new(),
            Vec::new(),
            Some("feature/fictional".to_string()),
            None,
            Some(Ok(Vec::new())),
        );

        assert_eq!(
            serde_json::to_value(checks).unwrap(),
            json!({
                "path": "/fictional/worktree",
                "files": [],
                "changes": [],
                "commits": [],
                "branch": "feature/fictional",
                "merged": null,
                "reason": null,
                "processes": [],
                "processInspectionUnavailable": false,
            }),
        );
    }

    fn fictional_session() -> Session {
        serde_json::from_value(json!({
            "session_id": "fictional-session",
            "cwd": "/fictional/worktree",
            "tmux_info": {
                "session_name": "fictional-session",
                "window_name": "fictional-window",
                "window_index": 0,
                "pane_id": "%12",
                "socket_path": "/fictional/tmux.sock",
            },
            "status": "running",
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z",
            "last_message": null,
        }))
        .unwrap()
    }

    #[rstest::rstest]
    #[case::stopped_agent_is_filtered(
        true,
        "23456 (fictional-worker)",
        vec![(23456, "fictional-worker")],
    )]
    #[case::running_agent_remains_a_blocker(
        false,
        "12345 (fictional-agent), 23456 (fictional-worker)",
        vec![(12345, "fictional-agent"), (23456, "fictional-worker")],
    )]
    fn process_guard_filters_only_the_agent_selected_for_stopping(
        #[case] filter_stopped_agent: bool,
        #[case] expected_process_list: &str,
        #[case] expected_processes: Vec<(u32, &str)>,
    ) {
        let worktree_path = PathBuf::from("/fictional/worktree");
        let session = fictional_session();
        let actual = check_process_guard_for_close(
            &worktree_path,
            &session,
            filter_stopped_agent,
            |session| (session.session_id == "fictional-session").then_some(12345),
            |_| {
                Some(vec![
                    ProcessInPath {
                        pid: 12345,
                        comm: "fictional-agent".to_string(),
                    },
                    ProcessInPath {
                        pid: 23456,
                        comm: "fictional-worker".to_string(),
                    },
                ])
            },
        );
        let Err(WorktreeProcessGuardError::ActiveProcesses {
            worktree_path,
            process_list,
            processes,
        }) = actual
        else {
            panic!("expected active process guard error");
        };

        assert_eq!(
            (
                worktree_path.display().to_string(),
                process_list,
                processes
                    .into_iter()
                    .map(|process| (process.pid, process.comm))
                    .collect::<Vec<_>>(),
            ),
            (
                "/fictional/worktree".to_string(),
                expected_process_list.to_string(),
                expected_processes
                    .into_iter()
                    .map(|(pid, comm)| (pid, comm.to_string()))
                    .collect::<Vec<_>>(),
            ),
        );
    }

    #[tokio::test]
    async fn inspect_runs_each_guard_check_before_building_the_payload() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicBool, Ordering};

        let files_checked = Arc::new(AtomicBool::new(false));
        let branch_checked = Arc::new(AtomicBool::new(false));
        let processes_checked = Arc::new(AtomicBool::new(false));
        let files_flag = Arc::clone(&files_checked);
        let branch_flag = Arc::clone(&branch_checked);
        let processes_flag = Arc::clone(&processes_checked);
        let checks = CloseChecks::inspect_with(
            PathBuf::from("/fictional/worktree"),
            Some("feature/fictional".to_string()),
            move |_| {
                files_flag.store(true, Ordering::SeqCst);
                vec![change("M", "src/fictional.rs")]
            },
            move |branch| {
                branch_flag.store(true, Ordering::SeqCst);
                async move {
                    Some(worktree_delete::classify_merge_guard_status(
                        branch,
                        MergeStatus::NotMerged {
                            reason: "pull request is open".to_string(),
                        },
                    ))
                }
            },
            |_| vec![commit("abc1234", "preserve fictional work")],
            move |path| {
                processes_flag.store(true, Ordering::SeqCst);
                Err(WorktreeProcessGuardError::ActiveProcesses {
                    worktree_path: path.to_path_buf(),
                    process_list: "12345 (fictional-worker)".to_string(),
                    processes: vec![ProcessInPath {
                        pid: 12345,
                        comm: "fictional-worker".to_string(),
                    }],
                })
            },
        )
        .await;

        assert_eq!(
            (
                files_checked.load(Ordering::SeqCst),
                branch_checked.load(Ordering::SeqCst),
                processes_checked.load(Ordering::SeqCst),
                serde_json::to_value(checks).unwrap(),
            ),
            (
                true,
                true,
                true,
                json!({
                    "path": "/fictional/worktree",
                    "files": ["src/fictional.rs"],
                    "changes": [{ "status": "M", "path": "src/fictional.rs" }],
                    "commits": [{ "sha": "abc1234", "subject": "preserve fictional work" }],
                    "branch": "feature/fictional",
                    "merged": false,
                    "reason": "pull request is open",
                    "processes": [{ "pid": 12345, "name": "fictional-worker" }],
                    "processInspectionUnavailable": false,
                }),
            ),
        );
    }

    #[test]
    fn parses_commit_log_records_without_running_git() {
        assert_eq!(
            parse_commits("abc1234\0preserve fictional work\ndef5678\0add fictional detail"),
            vec![
                commit("abc1234", "preserve fictional work"),
                commit("def5678", "add fictional detail"),
            ],
        );
    }

    fn change(status: &str, path: &str) -> worktree::UncommittedChange {
        worktree::UncommittedChange {
            status: status.to_string(),
            path: path.to_string(),
        }
    }

    fn commit(sha: &str, subject: &str) -> CloseCommit {
        CloseCommit {
            sha: sha.to_string(),
            subject: subject.to_string(),
        }
    }
}
