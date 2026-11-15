use std::future::Future;
use std::path::Path;

use crate::infra::git::GitRepo;
use crate::shared::hooks;
use crate::shared::merge_notify::notify_delegator_of_merge;
use crate::shared::worktree_error::Result;

use super::CleanWorktreeInfo;

/// Prints the indented detail lines for a successful worktree cleanup
/// (branch/tmux/session/process results), shared by the single-repo and
/// all-repos deletion loops.
fn print_cleanup_result(result: &crate::shared::cleanup::WorktreeCleanupResult) {
    if let Some(branch) = &result.branch_deleted {
        println!("  Branch deleted: {branch}");
    }
    if result.windows_closed > 0 {
        println!("  Tmux windows closed: {}", result.windows_closed);
    }
    if result.sessions_cleaned > 0 {
        println!("  Sessions cleaned: {}", result.sessions_cleaned);
    }
    if result.process_groups_signaled > 0 {
        println!(
            "  Process groups signaled: {}",
            result.process_groups_signaled
        );
    }
}

pub(super) async fn delete_worktrees_single_repo(
    repo: &GitRepo,
    worktrees: &[CleanWorktreeInfo],
    force: bool,
) -> Result<usize> {
    let mut deleted_count = 0;

    let failed_count = delete_worktrees_sequentially(
        worktrees.iter().cloned(),
        |info| async move { delete_one_worktree(repo, &info, force).await },
        |info, result| {
            let result = match result {
                Ok(result) => result,
                Err(error) => {
                    eprintln!("Skipped {}: {error:#}", info.wt.name);
                    return;
                }
            };

            if result.worktree_deleted {
                println!("Deleted: {}", info.wt.name);
                deleted_count += 1;
                print_cleanup_result(&result);
            } else {
                eprintln!(
                    "Failed to delete {}: worktree was not removed",
                    info.wt.name
                );
            }
        },
    )
    .await;

    println!();
    println!("Done. Deleted {deleted_count} worktree(s).");

    Ok(failed_count)
}

pub(super) async fn delete_worktrees_all_repos(
    repos_root: &Path,
    worktrees: &[CleanWorktreeInfo],
    force: bool,
) -> Result<usize> {
    let mut deleted_count = 0;
    let mut failed_count = 0;

    // Group worktrees by repository so input order doesn't matter
    let mut by_repo: std::collections::BTreeMap<&str, Vec<&CleanWorktreeInfo>> =
        std::collections::BTreeMap::new();
    for info in worktrees {
        let repo_name = info.repo_name.as_deref().unwrap_or("");
        by_repo.entry(repo_name).or_default().push(info);
    }

    for (repo_name, infos) in &by_repo {
        let repo_path = repos_root.join(repo_name);
        let repo = match GitRepo::open_at(&repo_path) {
            Ok(r) => r,
            Err(e) => {
                eprintln!(
                    "Warning: Failed to open {repo_name}; skipped {} worktree(s): {e}",
                    infos.len()
                );
                failed_count += infos.len();
                continue;
            }
        };

        let repo_ref = &repo;
        failed_count += delete_worktrees_sequentially(
            infos.iter().map(|info| (**info).clone()),
            |info| async move { delete_one_worktree(repo_ref, &info, force).await },
            |info, result| {
                let result = match result {
                    Ok(result) => result,
                    Err(error) => {
                        eprintln!("Skipped {repo_name}/{}: {error:#}", info.wt.name);
                        return;
                    }
                };

                if result.worktree_deleted {
                    println!("Deleted: {repo_name}/{}", info.wt.name);
                    deleted_count += 1;
                    print_cleanup_result(&result);
                } else {
                    eprintln!(
                        "Failed to delete {repo_name}/{}: worktree was not removed",
                        info.wt.name
                    );
                }
            },
        )
        .await;
    }

    println!();
    println!("Done. Deleted {deleted_count} worktree(s).");

    Ok(failed_count)
}

async fn delete_worktrees_sequentially<F, Fut>(
    worktrees: impl IntoIterator<Item = CleanWorktreeInfo>,
    mut delete: F,
    mut handle_result: impl FnMut(
        CleanWorktreeInfo,
        anyhow::Result<crate::shared::cleanup::WorktreeCleanupResult>,
    ),
) -> usize
where
    F: FnMut(CleanWorktreeInfo) -> Fut,
    Fut: Future<Output = anyhow::Result<crate::shared::cleanup::WorktreeCleanupResult>>,
{
    let mut failed_count = 0;
    for info in worktrees {
        let result = delete(info.clone()).await;
        if !result
            .as_ref()
            .is_ok_and(|cleanup_result| cleanup_result.worktree_deleted)
        {
            failed_count += 1;
        }
        handle_result(info, result);
    }
    failed_count
}

async fn delete_one_worktree(
    repo: &GitRepo,
    info: &CleanWorktreeInfo,
    force: bool,
) -> Result<crate::shared::cleanup::WorktreeCleanupResult> {
    crate::shared::worktree::ensure_clean_for_deletion(&info.wt.path, force)?;

    // Delegate metadata is stored in session records that cleanup removes.
    if info.status.is_merged() {
        notify_delegator_of_merge(repo, &info.wt.branch, &info.wt.path).await;
    }

    crate::shared::cleanup::cleanup_worktree_by_name(
        repo,
        &info.wt.name,
        &info.wt.path,
        force,
        || {
            hooks::spawn_post_worktree_delete_hook(
                repo.workdir(),
                &info.wt.path,
                hook_branch_name(&info.wt.branch),
                info.status.is_merged(),
            );
        },
    )
}

fn hook_branch_name(branch: &str) -> Option<&str> {
    (!branch.is_empty() && branch != "(unknown)" && branch != "(detached)").then_some(branch)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::git::MergeStatus;
    use crate::shared::worktree::LinkedWorktree;
    use std::cell::RefCell;
    use std::path::PathBuf;

    fn make_worktree_info(name: &str) -> CleanWorktreeInfo {
        CleanWorktreeInfo {
            wt: LinkedWorktree {
                name: name.to_string(),
                path: PathBuf::from(format!("/worktrees/{name}")),
                branch: format!("fohte/{name}"),
                commit: "abc1234".to_string(),
            },
            status: MergeStatus::Merged {
                reason: "merged".to_string(),
            },
            repo_name: None,
            has_active_session: false,
            has_uncommitted_changes: false,
            change_check_error: None,
        }
    }

    #[tokio::test]
    async fn deletion_continues_after_errors_and_non_deletions() {
        let worktrees = vec![
            make_worktree_info("unreadable"),
            make_worktree_info("not-removed"),
            make_worktree_info("clean"),
        ];
        let attempted = RefCell::new(Vec::new());
        let outcomes = RefCell::new(Vec::new());

        let failed_count = delete_worktrees_sequentially(
            worktrees,
            |info| {
                let attempted = &attempted;
                async move {
                    attempted.borrow_mut().push(info.wt.name.clone());
                    if info.wt.name == "unreadable" {
                        Err(anyhow::anyhow!("git status failed"))
                    } else if info.wt.name == "not-removed" {
                        Ok(crate::shared::cleanup::WorktreeCleanupResult::default())
                    } else {
                        Ok(crate::shared::cleanup::WorktreeCleanupResult {
                            worktree_deleted: true,
                            ..Default::default()
                        })
                    }
                }
            },
            |info, result| {
                outcomes.borrow_mut().push((
                    info.wt.name,
                    result
                        .map(|result| result.worktree_deleted)
                        .map_err(|error| error.to_string()),
                ));
            },
        )
        .await;

        assert_eq!(
            (attempted.into_inner(), outcomes.into_inner(), failed_count),
            (
                vec![
                    "unreadable".to_string(),
                    "not-removed".to_string(),
                    "clean".to_string()
                ],
                vec![
                    (
                        "unreadable".to_string(),
                        Err("git status failed".to_string())
                    ),
                    ("not-removed".to_string(), Ok(false)),
                    ("clean".to_string(), Ok(true)),
                ],
                2,
            ),
        );
    }
}
