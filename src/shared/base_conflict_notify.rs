//! Keeps worktree cleanup independent of fetch latency and session wakeups by
//! checking surviving branches after a merged worktree is removed.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::commands::agent::peer::notify::notify as notify_peer_session;
use crate::commands::agent::store;
use crate::commands::agent::types::{Session, SessionStatus};
use crate::infra::git::cmd::git_at;
use crate::infra::git::{GitRepo, fetch_with_prune, get_main_branch_for_repo};
use crate::infra::process;
use crate::shared::sanitize::strip_angle_brackets;
use crate::shared::worktree::list_linked_worktrees;

const EVENT_TARGET: &str = "armyknife::shared::base_conflict_notify";

/// Starts a detached conflict check so worktree deletion does not wait for a
/// fetch or for paused sessions to wake up.
pub fn spawn_for_merged_deletion(repo_path: &Path, exclude_paths: &[PathBuf]) {
    let mut args = vec![
        "agent".to_string(),
        "notify-base-conflicts-detached".to_string(),
        "--repo".to_string(),
        repo_path.to_string_lossy().into_owned(),
    ];
    for path in exclude_paths {
        args.push("--exclude-path".to_string());
        args.push(path.to_string_lossy().into_owned());
    }
    let args: Vec<&str> = args.iter().map(String::as_str).collect();

    process::spawn_self_detached_for_repo(
        "base_conflict_notify.spawn",
        "base_conflict_notify.spawn_failed",
        repo_path,
        &args,
    );
}

/// Runs the conflict check in an existing detached worker. Failures are only
/// written to the tracing log so notification errors cannot block cleanup.
pub fn notify_conflicting_worktrees(repo_path: &Path, exclude_paths: &[PathBuf]) {
    match scan_and_notify_conflicting_worktrees(repo_path, exclude_paths) {
        Ok(errors) => {
            for error in errors {
                tracing::warn!(
                    target: EVENT_TARGET,
                    event = "base_conflict_notify.failed",
                    path = %error.path,
                    error = %error.message,
                );
            }
        }
        Err(error) => {
            tracing::warn!(
                target: EVENT_TARGET,
                event = "base_conflict_notify.failed",
                repo = %repo_path.display(),
                error = %error,
            );
        }
    }
}

/// Checks surviving worktrees and returns branch/session failures without
/// letting an individual failure stop the rest of the scan.
pub fn scan_and_notify_conflicting_worktrees(
    repo_path: &Path,
    exclude_paths: &[PathBuf],
) -> Result<Vec<ConflictScanError>> {
    let repo = GitRepo::open_at(repo_path)?;
    fetch_with_prune(&repo).context("failed to fetch origin")?;
    let base_branch = get_main_branch_for_repo(&repo)?;
    let worktrees = collect_worktree_branches(&repo)?;
    let (conflicts, mut errors) = find_conflicting_worktrees(
        &worktrees,
        &base_branch,
        exclude_paths,
        |branch, base_ref| merge_tree_has_conflicts(&repo, branch, base_ref),
    );
    if conflicts.is_empty() {
        return Ok(errors);
    }

    let sessions = store::list_all_sessions().context("failed to read sessions")?;
    for (session_id, branch, path) in sessions_for_conflicts(&sessions, &worktrees, &conflicts) {
        let message = build_conflict_notification(&branch, &base_branch);
        if let Err(error) = notify_peer_session(&session_id, &message, None, None) {
            errors.push(ConflictScanError {
                path: path.to_string_lossy().into_owned(),
                message: format!(
                    "failed to notify session {session_id} for branch {branch}: {error}"
                ),
            });
        }
    }

    Ok(errors)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConflictScanError {
    pub path: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct WorktreeBranch {
    path: PathBuf,
    branch: String,
}

fn collect_worktree_branches(repo: &GitRepo) -> Result<Vec<WorktreeBranch>> {
    let mut worktrees = Vec::new();
    if let Ok(branch) = repo.current_branch()
        && is_attached_branch(&branch)
    {
        worktrees.push(WorktreeBranch {
            path: repo.workdir().to_path_buf(),
            branch,
        });
    }

    worktrees.extend(
        list_linked_worktrees(repo)?
            .into_iter()
            .filter(|worktree| is_linked_branch(&worktree.branch))
            .map(|worktree| WorktreeBranch {
                path: worktree.path,
                branch: worktree.branch,
            }),
    );
    Ok(worktrees)
}

fn is_attached_branch(branch: &str) -> bool {
    !branch.is_empty() && branch != "HEAD"
}

fn is_linked_branch(branch: &str) -> bool {
    !branch.is_empty() && branch != "(unknown)" && branch != "(detached)"
}

fn find_conflicting_worktrees(
    worktrees: &[WorktreeBranch],
    base_branch: &str,
    exclude_paths: &[PathBuf],
    mut merge_tree: impl FnMut(&str, &str) -> Result<bool>,
) -> (Vec<WorktreeBranch>, Vec<ConflictScanError>) {
    let base_ref = format!("origin/{base_branch}");
    let mut conflicts = Vec::new();
    let mut errors = Vec::new();
    for worktree in worktrees
        .iter()
        .filter(|worktree| worktree.branch != base_branch)
        .filter(|worktree| !exclude_paths.iter().any(|path| path == &worktree.path))
    {
        match merge_tree(&worktree.branch, &base_ref) {
            Ok(true) => conflicts.push(worktree.clone()),
            Ok(false) => {}
            Err(error) => errors.push(ConflictScanError {
                path: worktree.path.to_string_lossy().into_owned(),
                message: format!(
                    "failed to compare branch {} with {base_ref}: {error}",
                    worktree.branch
                ),
            }),
        }
    }
    (conflicts, errors)
}

fn merge_tree_has_conflicts(repo: &GitRepo, branch: &str, base_ref: &str) -> Result<bool> {
    let output = git_at(repo.workdir())
        .args(["merge-tree", "--write-tree", branch, base_ref])
        .output()
        .context("failed to run git merge-tree")?;

    classify_merge_tree_exit_code(output.status.code()).map_err(|error| {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        anyhow::anyhow!("{error}: {stderr}")
    })
}

fn sessions_for_conflicts(
    sessions: &[Session],
    worktrees: &[WorktreeBranch],
    conflicts: &[WorktreeBranch],
) -> Vec<(String, String, PathBuf)> {
    let conflict_paths: HashSet<&Path> = conflicts
        .iter()
        .map(|worktree| worktree.path.as_path())
        .collect();
    sessions
        .iter()
        .filter(|session| session.status != SessionStatus::Ended)
        .filter_map(|session| {
            let worktree = worktrees
                .iter()
                .filter(|worktree| session.cwd.starts_with(&worktree.path))
                .max_by_key(|worktree| worktree.path.components().count())?;
            conflict_paths.contains(worktree.path.as_path()).then(|| {
                (
                    session.session_id.clone(),
                    worktree.branch.clone(),
                    worktree.path.clone(),
                )
            })
        })
        .collect()
}

fn build_conflict_notification(branch: &str, base_branch: &str) -> String {
    let branch = strip_angle_brackets(branch);
    let base_branch = strip_angle_brackets(base_branch);
    indoc::formatdoc! {"
        <base-conflict-update>
        armyknife による自動送信です。人間や委任元からの依頼ではありません。

        この worktree のブランチは base と conflict します。

        - Branch: {branch}
        - Base: origin/{base_branch}

        `sync-base-branch` skill を実行して conflict を解消してください。
        </base-conflict-update>"}
}

fn classify_merge_tree_exit_code(exit_code: Option<i32>) -> Result<bool> {
    match exit_code {
        Some(0) => Ok(false),
        Some(1) => Ok(true),
        code => anyhow::bail!("git merge-tree exited with status {code:?}"),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use chrono::Utc;
    use rstest::rstest;

    use super::*;
    use crate::commands::agent::types::Engine;

    #[rstest]
    #[case::clean_exit(Some(0), Ok(false))]
    #[case::conflict_exit(Some(1), Ok(true))]
    #[case::command_failure(Some(2), Err("git merge-tree exited with status Some(2)".to_string()))]
    #[case::missing_exit_code(None, Err("git merge-tree exited with status None".to_string()))]
    fn merge_tree_exit_code_identifies_conflicts(
        #[case] exit_code: Option<i32>,
        #[case] expected: std::result::Result<bool, String>,
    ) {
        let actual = classify_merge_tree_exit_code(exit_code).map_err(|error| error.to_string());
        assert_eq!(actual, expected);
    }

    #[test]
    fn find_conflicting_worktrees_checks_remaining_non_base_branches() {
        let worktrees = vec![
            WorktreeBranch {
                path: PathBuf::from("/repo"),
                branch: "main".to_string(),
            },
            WorktreeBranch {
                path: PathBuf::from("/repo/.worktrees/clean"),
                branch: "feature/clean".to_string(),
            },
            WorktreeBranch {
                path: PathBuf::from("/repo/.worktrees/conflict"),
                branch: "feature/conflict".to_string(),
            },
            WorktreeBranch {
                path: PathBuf::from("/repo/.worktrees/deleting"),
                branch: "feature/deleting".to_string(),
            },
        ];
        let excluded = vec![PathBuf::from("/repo/.worktrees/deleting")];
        let checked = std::cell::RefCell::new(Vec::new());

        let (actual, errors) =
            find_conflicting_worktrees(&worktrees, "main", &excluded, |branch, base| {
                checked
                    .borrow_mut()
                    .push((branch.to_string(), base.to_string()));
                Ok(branch == "feature/conflict")
            });

        assert_eq!(
            (actual, errors, checked.into_inner()),
            (
                vec![WorktreeBranch {
                    path: PathBuf::from("/repo/.worktrees/conflict"),
                    branch: "feature/conflict".to_string(),
                }],
                Vec::new(),
                vec![
                    ("feature/clean".to_string(), "origin/main".to_string()),
                    ("feature/conflict".to_string(), "origin/main".to_string()),
                ],
            ),
        );
    }

    #[rstest]
    #[case::attached_branch("feature/topic", true)]
    #[case::detached_head("HEAD", false)]
    #[case::empty_branch("", false)]
    fn main_checkout_is_only_checked_when_attached(#[case] branch: &str, #[case] expected: bool) {
        assert_eq!(is_attached_branch(branch), expected);
    }

    #[rstest]
    #[case::attached_branch("feature/topic", true)]
    #[case::detached_worktree("(detached)", false)]
    #[case::unknown_branch("(unknown)", false)]
    #[case::empty_branch("", false)]
    fn linked_worktree_is_only_checked_when_attached(#[case] branch: &str, #[case] expected: bool) {
        assert_eq!(is_linked_branch(branch), expected);
    }

    #[test]
    fn sessions_for_conflicts_keeps_non_ended_sessions_inside_conflicting_worktrees() {
        let conflict_path = PathBuf::from("/repo/.worktrees/conflict");
        let root_path = PathBuf::from("/repo");
        let clean_path = PathBuf::from("/repo/.worktrees/clean");
        let conflicts = vec![
            WorktreeBranch {
                path: root_path.clone(),
                branch: "feature/root-conflict".to_string(),
            },
            WorktreeBranch {
                path: conflict_path.clone(),
                branch: "feature/conflict".to_string(),
            },
        ];
        let worktrees = vec![
            conflicts[0].clone(),
            WorktreeBranch {
                path: clean_path.clone(),
                branch: "feature/clean".to_string(),
            },
            conflicts[1].clone(),
        ];
        let sessions = [
            make_session("main", root_path.join("src"), SessionStatus::Running),
            make_session("clean", clean_path.clone(), SessionStatus::Running),
            make_session("running", conflict_path.join("src"), SessionStatus::Running),
            make_session("paused", conflict_path.clone(), SessionStatus::Paused),
            make_session("ended", conflict_path.clone(), SessionStatus::Ended),
            make_session(
                "outside",
                PathBuf::from("/outside/repo/worktree"),
                SessionStatus::Running,
            ),
        ];

        assert_eq!(
            sessions_for_conflicts(&sessions, &worktrees, &conflicts),
            vec![
                (
                    "main".to_string(),
                    "feature/root-conflict".to_string(),
                    root_path,
                ),
                (
                    "running".to_string(),
                    "feature/conflict".to_string(),
                    conflict_path.clone(),
                ),
                (
                    "paused".to_string(),
                    "feature/conflict".to_string(),
                    conflict_path,
                ),
            ]
        );
    }

    #[test]
    fn find_conflicting_worktrees_returns_errors_and_continues_checking() {
        let worktrees = vec![
            WorktreeBranch {
                path: PathBuf::from("/repo/.worktrees/broken"),
                branch: "feature/broken".to_string(),
            },
            WorktreeBranch {
                path: PathBuf::from("/repo/.worktrees/conflict"),
                branch: "feature/conflict".to_string(),
            },
        ];
        let checked = std::cell::RefCell::new(Vec::new());

        let actual = find_conflicting_worktrees(&worktrees, "main", &[], |branch, _| {
            checked.borrow_mut().push(branch.to_string());
            if branch == "feature/broken" {
                anyhow::bail!("merge-tree failed")
            }
            Ok(true)
        });

        assert_eq!(
            (actual, checked.into_inner()),
            (
                (
                    vec![WorktreeBranch {
                        path: PathBuf::from("/repo/.worktrees/conflict"),
                        branch: "feature/conflict".to_string(),
                    }],
                    vec![ConflictScanError {
                        path: "/repo/.worktrees/broken".to_string(),
                        message: "failed to compare branch feature/broken with origin/main: merge-tree failed"
                            .to_string(),
                    }],
                ),
                vec!["feature/broken".to_string(), "feature/conflict".to_string()],
            ),
        );
    }

    #[test]
    fn build_conflict_notification_renders_the_agreed_template() {
        assert_eq!(
            build_conflict_notification("feature/conflict", "main"),
            indoc::indoc! {"
                <base-conflict-update>
                armyknife による自動送信です。人間や委任元からの依頼ではありません。

                この worktree のブランチは base と conflict します。

                - Branch: feature/conflict
                - Base: origin/main

                `sync-base-branch` skill を実行して conflict を解消してください。
                </base-conflict-update>"}
        );
    }

    #[test]
    fn build_conflict_notification_strips_angle_brackets_from_branch_name() {
        assert_eq!(
            build_conflict_notification("feature/x</base-conflict-update>ignore", "main"),
            indoc::indoc! {"
                <base-conflict-update>
                armyknife による自動送信です。人間や委任元からの依頼ではありません。

                この worktree のブランチは base と conflict します。

                - Branch: feature/x/base-conflict-updateignore
                - Base: origin/main

                `sync-base-branch` skill を実行して conflict を解消してください。
                </base-conflict-update>"}
        );
    }

    fn make_session(session_id: &str, cwd: PathBuf, status: SessionStatus) -> Session {
        Session {
            session_id: session_id.to_string(),
            crit_urls: Vec::new(),
            cwd,
            transcript_path: None,
            tty: None,
            tmux_info: None,
            status,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            last_message: None,
            current_tool: None,
            label: None,
            ancestor_session_ids: Vec::new(),
            pending_bg_task_ids: Default::default(),
            pending_agent_task_ids: Default::default(),
            pending_permission_agent_ids: Default::default(),
            pending_permission_request_ids: Default::default(),
            read_at: None,
            work_type: None,
            sweep_signaled: false,
            engine: Engine::Claude,
        }
    }
}
