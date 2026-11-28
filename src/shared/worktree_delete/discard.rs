use std::collections::HashSet;
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::infra::git::cmd::run_git;
use crate::infra::git::{MergeStatus, get_default_branch_for_repo};
use crate::shared::worktree;
use crate::shared::worktree_delete::{
    UnmergedWorktreePolicy, WorktreeDeletePlan, check_merge_status, resolve_target,
};

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ConfirmedDiscard {
    pub(crate) path: String,
    pub(crate) branch: Option<String>,
    pub(crate) changes: Vec<ConfirmedChange>,
    pub(crate) commits: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ConfirmedChange {
    pub(crate) status: String,
    pub(crate) path: String,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CloseCommit {
    pub(crate) sha: String,
    pub(crate) subject: String,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct CommitInspection {
    pub(crate) commits: Vec<CloseCommit>,
    pub(crate) unavailable: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ObservedDiscard {
    pub(crate) path: String,
    pub(crate) branch: Option<String>,
    pub(crate) changes: Vec<ConfirmedChange>,
    pub(crate) commits: Vec<String>,
    pub(crate) commits_unavailable: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum DiscardVerificationError {
    TargetChanged,
    CommitsUnavailable,
    Unconfirmed {
        changes: Vec<ConfirmedChange>,
        commits: Vec<String>,
    },
}

#[derive(Debug, thiserror::Error)]
#[error("Worktree changed since discard confirmation; review the current close checks and retry")]
pub(crate) struct DiscardMismatchError;

impl DiscardMismatchError {
    pub(crate) fn new(_verification: DiscardVerificationError) -> Self {
        Self
    }
}

pub(crate) fn verify(
    confirmed: &ConfirmedDiscard,
    observed: &ObservedDiscard,
) -> std::result::Result<(), DiscardVerificationError> {
    if confirmed.path != observed.path || confirmed.branch != observed.branch {
        return Err(DiscardVerificationError::TargetChanged);
    }
    if observed.commits_unavailable {
        return Err(DiscardVerificationError::CommitsUnavailable);
    }

    // shortcut: Upgrade when status inspection can fingerprint file contents; Git status misses edits to listed paths.
    // shortcut: Upgrade when normal Git status reports nested untracked files; it hides files under listed directories.
    let confirmed_changes = confirmed
        .changes
        .iter()
        .map(|change| (&change.status, &change.path))
        .collect::<HashSet<_>>();
    let confirmed_commits = confirmed
        .commits
        .iter()
        .map(String::as_str)
        .collect::<HashSet<_>>();
    let changes = observed
        .changes
        .iter()
        .filter(|change| !confirmed_changes.contains(&(&change.status, &change.path)))
        .cloned()
        .collect::<Vec<_>>();
    let commits = observed
        .commits
        .iter()
        .filter(|commit| !confirmed_commits.contains(commit.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    if changes.is_empty() && commits.is_empty() {
        Ok(())
    } else {
        Err(DiscardVerificationError::Unconfirmed { changes, commits })
    }
}

pub(crate) async fn prepare_discard(
    worktree_path: &Path,
    confirmed: ConfirmedDiscard,
) -> Result<WorktreeDeletePlan> {
    let target = resolve_target(worktree_path)?;
    let changes = worktree::uncommitted_changes(&target.worktree_path)?;
    let merge_status = check_merge_status(
        &target.main_repo,
        target.branch_name.as_deref(),
        true,
        UnmergedWorktreePolicy::Reject,
    )
    .await?;
    let commits = inspect_commits_for_discard(
        target.branch_name.as_deref(),
        merge_status.as_ref(),
        |branch| {
            read_branch_commits(
                &target.worktree_path,
                branch,
                || get_default_branch_for_repo(&target.main_repo).ok(),
                |path, args| run_git(path, args).ok(),
            )
        },
    );
    let observed = observed_discard(
        &target.worktree_path,
        target.branch_name.as_deref(),
        changes,
        commits,
    );
    verify(&confirmed, &observed).map_err(|_| DiscardMismatchError)?;

    Ok(WorktreeDeletePlan {
        target,
        merge_status,
        force: false,
        discard: Some(confirmed),
    })
}

fn inspect_commits_for_discard(
    branch: Option<&str>,
    merge_status: Option<&MergeStatus>,
    inspect_branch_commits: impl FnOnce(&str) -> CommitInspection,
) -> CommitInspection {
    match (branch, merge_status) {
        (Some(branch), Some(MergeStatus::NotMerged { .. } | MergeStatus::Closed { .. })) => {
            inspect_branch_commits(branch)
        }
        (None, _) => CommitInspection {
            commits: Vec::new(),
            unavailable: true,
        },
        _ => CommitInspection {
            commits: Vec::new(),
            unavailable: false,
        },
    }
}

pub(crate) fn verify_before_delete_with(
    confirmed: &ConfirmedDiscard,
    worktree_path: &Path,
    merge_status: Option<&MergeStatus>,
    read_branch: impl FnOnce() -> Result<Option<String>>,
    read_changes: impl FnOnce(&Path) -> Result<Vec<worktree::UncommittedChange>>,
    read_commits: impl FnOnce(&Path, &str) -> CommitInspection,
) -> Result<()> {
    let current_branch =
        read_branch().context("Could not inspect worktree branch before discard")?;
    let changes = read_changes(worktree_path)?;
    let commits = inspect_commits_for_discard(current_branch.as_deref(), merge_status, |branch| {
        read_commits(worktree_path, branch)
    });
    let observed = observed_discard(worktree_path, current_branch.as_deref(), changes, commits);
    verify(confirmed, &observed).map_err(|_| DiscardMismatchError)?;
    Ok(())
}

fn observed_discard(
    worktree_path: &Path,
    branch: Option<&str>,
    changes: Vec<worktree::UncommittedChange>,
    commits: CommitInspection,
) -> ObservedDiscard {
    ObservedDiscard {
        path: worktree_path.display().to_string(),
        branch: branch.map(str::to_string),
        changes: changes
            .into_iter()
            .map(|change| ConfirmedChange {
                status: change.status,
                path: change.path,
            })
            .collect(),
        commits: commits
            .commits
            .into_iter()
            .map(|commit| commit.sha)
            .collect(),
        commits_unavailable: commits.unavailable,
    }
}

pub(crate) fn parse_commits(log: &str) -> Vec<CloseCommit> {
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

pub(crate) fn read_branch_commits(
    worktree_path: &Path,
    branch: &str,
    get_default_branch: impl FnOnce() -> Option<String>,
    run_git: impl FnOnce(&Path, [&str; 3]) -> Option<String>,
) -> CommitInspection {
    let Some(default_branch) = get_default_branch() else {
        return CommitInspection {
            commits: Vec::new(),
            unavailable: true,
        };
    };
    let range = format!("origin/{default_branch}..refs/heads/{branch}");
    let Some(log) = run_git(worktree_path, ["log", "--format=%H%x00%s", range.as_str()]) else {
        return CommitInspection {
            commits: Vec::new(),
            unavailable: true,
        };
    };
    CommitInspection {
        commits: parse_commits(&log),
        unavailable: false,
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use rstest::{fixture, rstest};

    use super::*;

    #[derive(Clone, Copy)]
    enum VerifyCase {
        ExactMatch,
        FewerChanges,
        NewFile,
        StatusChanged,
        NewCommit,
        RewrittenCommit,
        CommitsUnavailable,
        BranchChanged,
        PathChanged,
    }

    #[fixture]
    fn confirmed() -> ConfirmedDiscard {
        ConfirmedDiscard {
            path: "/fictional/worktree".to_string(),
            branch: Some("feature/fictional".to_string()),
            changes: vec![
                change("M", "src/fictional.rs"),
                change("??", "notes/fictional.txt"),
            ],
            commits: vec!["sha-a".to_string(), "sha-b".to_string()],
        }
    }

    #[fixture]
    fn observed() -> ObservedDiscard {
        ObservedDiscard {
            path: "/fictional/worktree".to_string(),
            branch: Some("feature/fictional".to_string()),
            changes: vec![
                change("M", "src/fictional.rs"),
                change("??", "notes/fictional.txt"),
            ],
            commits: vec!["sha-a".to_string(), "sha-b".to_string()],
            commits_unavailable: false,
        }
    }

    #[rstest]
    #[case::exact_match(VerifyCase::ExactMatch, Ok(()))]
    #[case::fewer_changes(VerifyCase::FewerChanges, Ok(()))]
    #[case::new_file(
        VerifyCase::NewFile,
        Err(DiscardVerificationError::Unconfirmed {
            changes: vec![change("??", "src/new-fictional.txt")],
            commits: vec![],
        })
    )]
    #[case::status_changed(
        VerifyCase::StatusChanged,
        Err(DiscardVerificationError::Unconfirmed {
            changes: vec![change("D", "src/fictional.rs")],
            commits: vec![],
        })
    )]
    #[case::new_commit(
        VerifyCase::NewCommit,
        Err(DiscardVerificationError::Unconfirmed {
            changes: vec![],
            commits: vec!["sha-c".to_string()],
        })
    )]
    #[case::rewritten_commit(
        VerifyCase::RewrittenCommit,
        Err(DiscardVerificationError::Unconfirmed {
            changes: vec![],
            commits: vec!["sha-rewritten".to_string()],
        })
    )]
    #[case::commits_unavailable(
        VerifyCase::CommitsUnavailable,
        Err(DiscardVerificationError::CommitsUnavailable)
    )]
    #[case::branch_changed(
        VerifyCase::BranchChanged,
        Err(DiscardVerificationError::TargetChanged)
    )]
    #[case::path_changed(VerifyCase::PathChanged, Err(DiscardVerificationError::TargetChanged))]
    fn verifies_confirmed_discard_contents(
        confirmed: ConfirmedDiscard,
        mut observed: ObservedDiscard,
        #[case] case: VerifyCase,
        #[case] expected: std::result::Result<(), DiscardVerificationError>,
    ) {
        match case {
            VerifyCase::ExactMatch => {}
            VerifyCase::FewerChanges => {
                observed.changes.pop();
                observed.commits.pop();
            }
            VerifyCase::NewFile => observed.changes.push(change("??", "src/new-fictional.txt")),
            VerifyCase::StatusChanged => {
                observed.changes[0].status = "D".to_string();
            }
            VerifyCase::NewCommit => observed.commits.push("sha-c".to_string()),
            VerifyCase::RewrittenCommit => {
                observed.commits[0] = "sha-rewritten".to_string();
            }
            VerifyCase::CommitsUnavailable => observed.commits_unavailable = true,
            VerifyCase::BranchChanged => {
                observed.branch = Some("feature/renamed".to_string());
            }
            VerifyCase::PathChanged => observed.path = "/fictional/other-worktree".to_string(),
        }

        assert_eq!(verify(&confirmed, &observed), expected);
    }

    #[rstest]
    #[case::merged(MergeStatus::Merged { reason: "fictional merge".to_string() }, false)]
    #[case::not_merged(MergeStatus::NotMerged { reason: "fictional open pull request".to_string() }, true)]
    fn before_delete_reads_commits_only_for_unmerged_branches(
        confirmed: ConfirmedDiscard,
        #[case] merge_status: MergeStatus,
        #[case] expected_commits_read: bool,
    ) {
        let merge_status = Some(merge_status);
        let commits_read = std::cell::Cell::new(false);
        let result = verify_before_delete_with(
            &confirmed,
            Path::new("/fictional/worktree"),
            merge_status.as_ref(),
            || Ok(Some("feature/fictional".to_string())),
            |_| {
                Ok(vec![
                    worktree::UncommittedChange {
                        status: "M".to_string(),
                        path: "src/fictional.rs".to_string(),
                    },
                    worktree::UncommittedChange {
                        status: "??".to_string(),
                        path: "notes/fictional.txt".to_string(),
                    },
                ])
            },
            |_, _| {
                commits_read.set(true);
                CommitInspection {
                    commits: vec![commit("sha-a", "preserve fictional work")],
                    unavailable: false,
                }
            },
        )
        .map_err(|error| error.to_string());

        assert_eq!(
            (result, commits_read.get()),
            (Ok(()), expected_commits_read)
        );
    }

    #[test]
    fn parses_commit_log_records_without_running_git() {
        let log = indoc::indoc! {
            r#"
            sha-a\0preserve fictional work
            sha-b\0add fictional detail
            "#
        }
        .replace("\\0", "\0");

        assert_eq!(
            parse_commits(&log),
            vec![
                commit("sha-a", "preserve fictional work"),
                commit("sha-b", "add fictional detail"),
            ],
        );
    }

    #[test]
    fn reads_branch_commits_from_the_expected_default_branch_range() {
        let mut observed_git_call = None;
        let inspection = read_branch_commits(
            Path::new("/fictional/worktree"),
            "feature/fictional",
            || Some("main".to_string()),
            |path, args| {
                observed_git_call = Some((path.to_path_buf(), args.map(str::to_string)));
                Some("sha-a\0fictional commit".to_string())
            },
        );

        assert_eq!(
            (inspection, observed_git_call),
            (
                CommitInspection {
                    commits: vec![commit("sha-a", "fictional commit")],
                    unavailable: false,
                },
                Some((
                    PathBuf::from("/fictional/worktree"),
                    [
                        "log".to_string(),
                        "--format=%H%x00%s".to_string(),
                        "origin/main..refs/heads/feature/fictional".to_string(),
                    ],
                )),
            ),
        );
    }

    #[rstest]
    #[case::default_branch_unavailable(None, false, false)]
    #[case::git_query_failed(Some("main"), false, true)]
    fn reports_commit_inspection_failures(
        #[case] default_branch: Option<&str>,
        #[case] git_result: bool,
        #[case] expected_git_call: bool,
    ) {
        let mut git_called = false;
        let inspection = read_branch_commits(
            Path::new("/fictional/worktree"),
            "feature/fictional",
            || default_branch.map(str::to_string),
            |_, _| {
                git_called = true;
                git_result.then(String::new)
            },
        );

        assert_eq!(
            (inspection, git_called),
            (
                CommitInspection {
                    commits: Vec::new(),
                    unavailable: true,
                },
                expected_git_call,
            ),
        );
    }

    fn change(status: &str, path: &str) -> ConfirmedChange {
        ConfirmedChange {
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
