use std::future::Future;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Serialize;

use crate::infra::git::cmd::run_git;
use crate::infra::git::{GitRepo, MergeStatus, get_merge_status_for_repo};
use crate::infra::process::ProcessInPath;
use crate::infra::process::find_non_shell_processes_in_path;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct CloseChecks {
    path: String,
    files: Vec<String>,
    branch: Option<String>,
    merged: Option<bool>,
    reason: Option<String>,
    processes: Vec<CloseProcess>,
    process_inspection_unavailable: bool,
}

#[derive(Debug, Serialize)]
struct CloseProcess {
    pid: u32,
    name: String,
}

impl CloseChecks {
    pub(super) async fn inspect(session_path: &Path) -> Self {
        let session_path = session_path.to_path_buf();
        let repo_path = session_path.clone();
        let repo = tokio::task::spawn_blocking(move || GitRepo::open_at(&repo_path).ok())
            .await
            .ok()
            .flatten();
        let worktree_path = repo
            .as_ref()
            .map(|repo| repo.workdir().to_path_buf())
            .unwrap_or_else(|| session_path.to_path_buf());
        Self::inspect_with(
            worktree_path,
            changed_files,
            move |_| {
                let runtime = tokio::runtime::Handle::current();
                async move {
                    tokio::task::spawn_blocking(move || {
                        runtime.block_on(async move {
                            match repo {
                                Some(repo) => match repo.current_branch() {
                                    Ok(branch) if branch != "HEAD" => {
                                        let status = tokio::time::timeout(
                                            Duration::from_secs(5),
                                            get_merge_status_for_repo(&repo, &branch),
                                        )
                                        .await
                                        .ok();
                                        Some((Some(branch), status))
                                    }
                                    _ => None,
                                },
                                None => None,
                            }
                        })
                    })
                    .await
                    .ok()
                    .flatten()
                }
            },
            find_non_shell_processes_in_path,
        )
        .await
    }

    async fn inspect_with<Files, Branch, BranchFuture, Processes>(
        worktree_path: PathBuf,
        inspect_files: Files,
        inspect_branch: Branch,
        inspect_processes: Processes,
    ) -> Self
    where
        Files: FnOnce(&Path) -> Vec<String>,
        Branch: FnOnce(&Path) -> BranchFuture,
        BranchFuture: Future<Output = Option<(Option<String>, Option<MergeStatus>)>>,
        Processes: FnOnce(&Path) -> Option<Vec<ProcessInPath>> + Send + 'static,
        Files: Send + 'static,
    {
        let files_path = worktree_path.clone();
        let files = tokio::task::spawn_blocking(move || inspect_files(&files_path))
            .await
            .unwrap_or_default();
        let branch_status = inspect_branch(&worktree_path).await;
        let processes_path = worktree_path.clone();
        let process_inspection =
            tokio::task::spawn_blocking(move || inspect_processes(&processes_path))
                .await
                .ok()
                .flatten();

        Self::from_observations(worktree_path, files, branch_status, process_inspection)
    }

    pub(super) fn from_observations(
        worktree_path: PathBuf,
        files: Vec<String>,
        branch_status: Option<(Option<String>, Option<MergeStatus>)>,
        process_inspection: Option<Vec<ProcessInPath>>,
    ) -> Self {
        let (branch, merged, reason) = match branch_status {
            Some((branch, status)) => match status {
                Some(status) => {
                    let merged = status.is_merged();
                    let reason = (!merged).then(|| status.reason().to_string());
                    (branch, Some(merged), reason)
                }
                None => (branch, None, None),
            },
            None => (None, None, None),
        };
        let process_inspection_unavailable = process_inspection.is_none();
        let processes = process_inspection
            .unwrap_or_default()
            .into_iter()
            .map(|process| CloseProcess {
                pid: process.pid,
                name: process.comm,
            })
            .collect();

        Self {
            path: worktree_path.display().to_string(),
            files,
            branch,
            merged,
            reason,
            processes,
            process_inspection_unavailable,
        }
    }
}

fn changed_files(worktree_path: &Path) -> Vec<String> {
    let Ok(status) = run_git(
        worktree_path,
        ["status", "--porcelain=v1", "-z", "--untracked-files=normal"],
    ) else {
        return Vec::new();
    };
    parse_changed_files(&status)
}

fn parse_changed_files(status: &str) -> Vec<String> {
    let mut records = status.split('\0');
    let mut files = Vec::new();
    while let Some(record) = records.next() {
        if record.len() < 4 {
            continue;
        }
        let status_code = &record[..2];
        files.push(record[3..].to_string());
        if status_code.contains('R') || status_code.contains('C') {
            let _ = records.next();
        }
    }
    files
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn serializes_all_close_guard_observations() {
        let checks = CloseChecks::from_observations(
            PathBuf::from("/fictional/worktree"),
            vec!["src/fictional.rs".to_string()],
            Some((
                Some("feature/fictional".to_string()),
                Some(MergeStatus::NotMerged {
                    reason: "pull request is open".to_string(),
                }),
            )),
            Some(vec![ProcessInPath {
                pid: 12345,
                comm: "fictional-worker".to_string(),
            }]),
        );

        assert_eq!(
            serde_json::to_value(checks).unwrap(),
            json!({
                "path": "/fictional/worktree",
                "files": ["src/fictional.rs"],
                "branch": "feature/fictional",
                "merged": false,
                "reason": "pull request is open",
                "processes": [{ "pid": 12345, "name": "fictional-worker" }],
                "processInspectionUnavailable": false,
            }),
        );
    }

    #[test]
    fn parse_changed_files_handles_untracked_and_renamed_paths() {
        assert_eq!(
            parse_changed_files("?? src/new.rs\0R  src/renamed.rs\0src/old.rs\0"),
            vec!["src/new.rs", "src/renamed.rs"],
        );
    }

    #[test]
    fn missing_observations_use_the_documented_null_and_empty_values() {
        let checks = CloseChecks::from_observations(
            PathBuf::from("/fictional/detached-worktree"),
            Vec::new(),
            None,
            None,
        );

        assert_eq!(
            serde_json::to_value(checks).unwrap(),
            json!({
                "path": "/fictional/detached-worktree",
                "files": [],
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
            Some((Some("feature/fictional".to_string()), None)),
            Some(Vec::new()),
        );

        assert_eq!(
            serde_json::to_value(checks).unwrap(),
            json!({
                "path": "/fictional/worktree",
                "files": [],
                "branch": "feature/fictional",
                "merged": null,
                "reason": null,
                "processes": [],
                "processInspectionUnavailable": false,
            }),
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
            move |_| {
                files_flag.store(true, Ordering::SeqCst);
                vec!["src/fictional.rs".to_string()]
            },
            move |_| {
                branch_flag.store(true, Ordering::SeqCst);
                async {
                    Some((
                        Some("feature/fictional".to_string()),
                        Some(MergeStatus::NotMerged {
                            reason: "pull request is open".to_string(),
                        }),
                    ))
                }
            },
            move |_| {
                processes_flag.store(true, Ordering::SeqCst);
                Some(vec![ProcessInPath {
                    pid: 12345,
                    comm: "fictional-worker".to_string(),
                }])
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
                    "branch": "feature/fictional",
                    "merged": false,
                    "reason": "pull request is open",
                    "processes": [{ "pid": 12345, "name": "fictional-worker" }],
                    "processInspectionUnavailable": false,
                }),
            ),
        );
    }
}
