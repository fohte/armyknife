use std::future::Future;
use std::path::{Path, PathBuf};

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
        let repo = GitRepo::open_at(session_path).ok();
        let worktree_path = repo
            .as_ref()
            .map(|repo| repo.workdir().to_path_buf())
            .unwrap_or_else(|| session_path.to_path_buf());
        Self::inspect_with(
            worktree_path,
            changed_files,
            move |_| async move {
                match repo {
                    Some(repo) => match repo.current_branch() {
                        Ok(branch) if branch != "HEAD" => {
                            let status = get_merge_status_for_repo(&repo, &branch).await;
                            Some((branch, status))
                        }
                        _ => None,
                    },
                    None => None,
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
        BranchFuture: Future<Output = Option<(String, MergeStatus)>>,
        Processes: FnOnce(&Path) -> Option<Vec<ProcessInPath>>,
    {
        let files = inspect_files(&worktree_path);
        let branch_status = inspect_branch(&worktree_path).await;
        let process_inspection = inspect_processes(&worktree_path);

        Self::from_observations(worktree_path, files, branch_status, process_inspection)
    }

    pub(super) fn from_observations(
        worktree_path: PathBuf,
        files: Vec<String>,
        branch_status: Option<(String, MergeStatus)>,
        process_inspection: Option<Vec<ProcessInPath>>,
    ) -> Self {
        let (branch, merged, reason) = match branch_status {
            Some((branch, status)) => {
                let merged = status.is_merged();
                let reason = (!merged).then(|| status.reason().to_string());
                (Some(branch), Some(merged), reason)
            }
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
                "feature/fictional".to_string(),
                MergeStatus::NotMerged {
                    reason: "pull request is open".to_string(),
                },
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

    #[tokio::test]
    async fn inspect_runs_each_guard_check_before_building_the_payload() {
        use std::cell::Cell;
        use std::rc::Rc;

        let files_checked = Rc::new(Cell::new(false));
        let branch_checked = Rc::new(Cell::new(false));
        let processes_checked = Rc::new(Cell::new(false));
        let files_flag = Rc::clone(&files_checked);
        let branch_flag = Rc::clone(&branch_checked);
        let processes_flag = Rc::clone(&processes_checked);
        let checks = CloseChecks::inspect_with(
            PathBuf::from("/fictional/worktree"),
            move |_| {
                files_flag.set(true);
                vec!["src/fictional.rs".to_string()]
            },
            move |_| {
                branch_flag.set(true);
                async {
                    Some((
                        "feature/fictional".to_string(),
                        MergeStatus::NotMerged {
                            reason: "pull request is open".to_string(),
                        },
                    ))
                }
            },
            move |_| {
                processes_flag.set(true);
                Some(vec![ProcessInPath {
                    pid: 12345,
                    comm: "fictional-worker".to_string(),
                }])
            },
        )
        .await;

        assert_eq!(
            (
                files_checked.get(),
                branch_checked.get(),
                processes_checked.get(),
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
