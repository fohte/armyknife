//! Common worktree operations shared between delete, clean, and list commands.

use std::path::PathBuf;

use anyhow::Context;

use super::error::{Result, WmError};
use crate::infra::git::GitRepo;
use crate::infra::git::cmd::run_git;

/// Get the main repository, resolving from a worktree if necessary.
pub fn get_main_repo(repo: &GitRepo) -> Result<GitRepo> {
    repo.main_repo().map_err(|_| WmError::NotInGitRepo.into())
}

/// Get the branch name associated with a worktree.
pub fn get_worktree_branch(repo: &GitRepo, worktree_name: &str) -> Option<String> {
    let wt = list_worktrees_raw(repo).ok()?;
    let entry = wt.into_iter().find(|w| w.name == worktree_name)?;
    Some(entry.branch).filter(|b| !b.is_empty() && b != "(detached)")
}

/// Delete a worktree by name. Returns true if successful.
pub fn delete_worktree(repo: &GitRepo, worktree_name: &str) -> Result<bool> {
    let entry = match list_worktrees_raw(repo) {
        Ok(list) => match list.into_iter().find(|w| w.name == worktree_name) {
            Some(e) => e,
            None => {
                eprintln!("Failed to find worktree {worktree_name}");
                return Ok(false);
            }
        },
        Err(e) => {
            eprintln!("Failed to list worktrees: {e}");
            return Ok(false);
        }
    };

    let remove = run_git(
        repo.workdir(),
        [
            "worktree",
            "remove",
            "--force",
            entry.path.to_string_lossy().as_ref(),
        ],
    );
    if remove.is_err() {
        // Path may have been deleted out-of-band; prune lingering admin data.
        let _ = run_git(repo.workdir(), ["worktree", "prune"]);
        let still_present = list_worktrees_raw(repo)
            .map(|l| l.iter().any(|w| w.name == worktree_name))
            .unwrap_or(false);
        if still_present {
            eprintln!("Failed to delete worktree {worktree_name}");
            return Ok(false);
        }
    }
    Ok(true)
}

/// Delete a local branch if it exists. Returns true if deleted.
pub fn delete_branch_if_exists(repo: &GitRepo, branch: &str) -> bool {
    if branch.is_empty() {
        return false;
    }
    if !repo.local_branch_exists(branch) {
        return false;
    }
    repo.delete_branch(branch).is_ok()
}

/// Find the worktree name from its path.
pub fn find_worktree_name(repo: &GitRepo, worktree_path: &str) -> Result<String> {
    let normalized = worktree_path.trim_end_matches('/');
    let entries = list_worktrees_raw(repo).context("Failed to list worktrees")?;
    for entry in entries {
        let p = entry.path.to_string_lossy();
        if p.trim_end_matches('/') == normalized {
            return Ok(entry.name);
        }
    }
    Err(WmError::WorktreeNotFound(worktree_path.to_string()).into())
}

/// Basic information about a linked worktree.
#[derive(Debug, Clone)]
pub struct LinkedWorktree {
    pub name: String,
    pub path: PathBuf,
    pub branch: String,
    pub commit: String,
}

/// List all linked worktrees (excludes main worktree).
pub fn list_linked_worktrees(repo: &GitRepo) -> Result<Vec<LinkedWorktree>> {
    let all = list_worktrees_raw(repo).context("Failed to list worktrees")?;
    let mut result = Vec::new();
    // First entry from --porcelain is always the main worktree.
    for entry in all.into_iter().skip(1) {
        result.push(LinkedWorktree {
            name: entry.name,
            path: entry.path,
            branch: if entry.branch.is_empty() {
                "(unknown)".to_string()
            } else {
                entry.branch
            },
            commit: entry.short_hash,
        });
    }
    Ok(result)
}

/// Get the main worktree path.
pub fn get_main_worktree_path(repo: &GitRepo) -> Result<PathBuf> {
    if repo.is_worktree() {
        repo.main_workdir()
    } else {
        Ok(repo.workdir().to_path_buf())
    }
}

/// Get the branch and commit for the main worktree.
pub fn get_main_worktree_info(repo: &GitRepo) -> (String, String) {
    let main_repo = if repo.is_worktree() {
        repo.main_repo().ok()
    } else {
        None
    };
    let target = main_repo.as_ref().unwrap_or(repo);
    let branch = target
        .current_branch()
        .unwrap_or_else(|_| "(unknown)".to_string());
    let commit = target
        .short_hash("HEAD")
        .unwrap_or_else(|_| "(none)".to_string());
    (branch, commit)
}

/// Raw worktree-list entry from `git worktree list --porcelain`.
#[derive(Debug, Clone)]
struct WorktreeEntry {
    /// Worktree name (last path component, matching git's internal name).
    name: String,
    path: PathBuf,
    /// Branch shorthand, `"(detached)"` for detached HEAD, or empty.
    branch: String,
    /// 7-char short hash.
    short_hash: String,
}

/// Run `git worktree list --porcelain` and parse the result. First entry is
/// the main worktree.
fn list_worktrees_raw(repo: &GitRepo) -> anyhow::Result<Vec<WorktreeEntry>> {
    let stdout = run_git(repo.workdir(), ["worktree", "list", "--porcelain"])?;
    Ok(parse_worktree_porcelain(&stdout))
}

fn parse_worktree_porcelain(text: &str) -> Vec<WorktreeEntry> {
    let mut out = Vec::new();
    let mut current: Option<(PathBuf, String, String, bool)> = None;

    fn flush(cur: Option<(PathBuf, String, String, bool)>, out: &mut Vec<WorktreeEntry>) {
        if let Some((path, head, branch, detached)) = cur {
            let name = path
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            let short_hash = head.chars().take(7).collect::<String>();
            let branch_display = if detached {
                "(detached)".to_string()
            } else {
                branch
                    .strip_prefix("refs/heads/")
                    .unwrap_or(&branch)
                    .to_string()
            };
            out.push(WorktreeEntry {
                name,
                path,
                branch: branch_display,
                short_hash,
            });
        }
    }

    for line in text.lines() {
        if line.is_empty() {
            flush(current.take(), &mut out);
            continue;
        }
        if let Some(rest) = line.strip_prefix("worktree ") {
            flush(current.take(), &mut out);
            current = Some((PathBuf::from(rest), String::new(), String::new(), false));
        } else if let Some(rest) = line.strip_prefix("HEAD ") {
            if let Some(c) = current.as_mut() {
                c.1 = rest.to_string();
            }
        } else if let Some(rest) = line.strip_prefix("branch ") {
            if let Some(c) = current.as_mut() {
                c.2 = rest.to_string();
            }
        } else if line == "detached"
            && let Some(c) = current.as_mut()
        {
            c.3 = true;
        }
    }
    flush(current.take(), &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shared::testing::TestRepo;

    #[test]
    fn get_main_repo_from_main_returns_same_repo() {
        let test_repo = TestRepo::new();
        let repo = test_repo.open();

        let main_repo = get_main_repo(&repo).unwrap();
        assert_eq!(
            main_repo.workdir().canonicalize().unwrap(),
            test_repo.path()
        );
    }

    #[test]
    fn get_main_repo_from_worktree_returns_main() {
        let test_repo = TestRepo::new();
        test_repo.create_worktree("feature");

        let wt_repo = crate::infra::git::open_repo_at(&test_repo.worktree_path("feature")).unwrap();
        let main_repo = get_main_repo(&wt_repo).unwrap();

        assert_eq!(
            main_repo.workdir().canonicalize().unwrap(),
            test_repo.path()
        );
    }

    #[test]
    fn get_worktree_branch_returns_branch_name() {
        let test_repo = TestRepo::new();
        test_repo.create_worktree("feature-branch");

        let repo = test_repo.open();
        let branch = get_worktree_branch(&repo, "feature-branch");

        assert_eq!(branch, Some("feature-branch".to_string()));
    }

    #[test]
    fn get_worktree_branch_returns_none_for_nonexistent() {
        let test_repo = TestRepo::new();
        let repo = test_repo.open();

        let branch = get_worktree_branch(&repo, "nonexistent");
        assert_eq!(branch, None);
    }

    #[test]
    fn delete_worktree_removes_worktree() {
        let test_repo = TestRepo::new();
        test_repo.create_worktree("to-delete");

        let repo = test_repo.open();

        assert!(
            list_worktrees_raw(&repo)
                .unwrap()
                .iter()
                .any(|w| w.name == "to-delete")
        );

        let result = delete_worktree(&repo, "to-delete").unwrap();
        assert!(result);

        assert!(
            !list_worktrees_raw(&repo)
                .unwrap()
                .iter()
                .any(|w| w.name == "to-delete")
        );
    }

    #[test]
    fn delete_worktree_returns_false_for_nonexistent() {
        let test_repo = TestRepo::new();
        let repo = test_repo.open();

        let result = delete_worktree(&repo, "nonexistent").unwrap();
        assert!(!result);
    }

    #[test]
    fn find_worktree_name_finds_by_path() {
        let test_repo = TestRepo::new();
        test_repo.create_worktree("my-feature");

        let repo = test_repo.open();
        let wt_path = test_repo.worktree_path("my-feature");

        let name = find_worktree_name(&repo, wt_path.to_str().unwrap()).unwrap();
        assert_eq!(name, "my-feature");
    }

    #[test]
    fn find_worktree_name_returns_error_for_nonexistent() {
        let test_repo = TestRepo::new();
        let repo = test_repo.open();

        let result = find_worktree_name(&repo, "/nonexistent/path");
        assert!(result.is_err());
    }

    #[test]
    fn list_linked_worktrees_empty_when_no_worktrees() {
        let test_repo = TestRepo::new();
        let repo = test_repo.open();

        let worktrees = list_linked_worktrees(&repo).unwrap();
        assert!(worktrees.is_empty());
    }

    #[test]
    fn list_linked_worktrees_returns_all_worktrees() {
        let test_repo = TestRepo::new();
        test_repo.create_worktree("feature-a");
        test_repo.create_worktree("feature-b");

        let repo = test_repo.open();
        let worktrees = list_linked_worktrees(&repo).unwrap();

        assert_eq!(worktrees.len(), 2);

        let mut names: Vec<&str> = worktrees.iter().map(|w| w.name.as_str()).collect();
        names.sort();
        assert_eq!(names, vec!["feature-a", "feature-b"]);
    }

    #[test]
    fn list_linked_worktrees_includes_branch_and_commit() {
        let test_repo = TestRepo::new();
        test_repo.create_worktree("feature");

        let repo = test_repo.open();
        let worktrees = list_linked_worktrees(&repo).unwrap();

        assert_eq!(worktrees.len(), 1);
        let wt = &worktrees[0];
        assert_eq!(wt.name, "feature");
        assert_eq!(wt.branch, "feature");
        assert_eq!(wt.commit.len(), 7);
        assert_eq!(wt.path, test_repo.worktree_path("feature"));
    }

    #[test]
    fn get_main_worktree_path_from_main() {
        let test_repo = TestRepo::new();
        let repo = test_repo.open();

        let path = get_main_worktree_path(&repo).unwrap();
        assert_eq!(path.canonicalize().unwrap(), test_repo.path());
    }

    #[test]
    fn get_main_worktree_path_from_worktree() {
        let test_repo = TestRepo::new();
        test_repo.create_worktree("feature");

        let wt_repo = crate::infra::git::open_repo_at(&test_repo.worktree_path("feature")).unwrap();
        let path = get_main_worktree_path(&wt_repo).unwrap();

        assert_eq!(path.canonicalize().unwrap(), test_repo.path());
    }

    #[test]
    fn get_main_worktree_info_returns_branch_and_commit() {
        let test_repo = TestRepo::new();
        let repo = test_repo.open();

        let (branch, commit) = get_main_worktree_info(&repo);

        assert_eq!(branch, "master");
        assert_eq!(commit.len(), 7);
    }

    #[test]
    fn get_main_worktree_info_from_worktree_returns_main_branch() {
        let test_repo = TestRepo::new();
        test_repo.create_worktree("feature");

        let wt_repo = crate::infra::git::open_repo_at(&test_repo.worktree_path("feature")).unwrap();

        let (branch, _commit) = get_main_worktree_info(&wt_repo);
        assert_eq!(branch, "master");
    }

    #[test]
    fn delete_branch_if_exists_works_after_worktree_deleted() {
        let test_repo = TestRepo::new();
        test_repo.create_worktree("feature-to-delete");

        let repo = test_repo.open();
        let wt_path = test_repo.worktree_path("feature-to-delete");

        assert!(
            repo.local_branch_exists("feature-to-delete"),
            "Branch should exist before deletion"
        );

        struct DirGuard(std::path::PathBuf);
        impl Drop for DirGuard {
            fn drop(&mut self) {
                let _ = std::env::set_current_dir(&self.0);
            }
        }

        let branch_deleted = {
            let _guard = DirGuard(std::env::current_dir().unwrap());
            std::env::set_current_dir(&wt_path).unwrap();

            let deleted = delete_worktree(&repo, "feature-to-delete").unwrap();
            assert!(deleted, "Worktree should be deleted");

            delete_branch_if_exists(&repo, "feature-to-delete")
        };

        assert!(
            branch_deleted,
            "Branch should be deleted even after worktree is removed"
        );

        assert!(
            !repo.local_branch_exists("feature-to-delete"),
            "Branch should not exist after deletion"
        );
    }

    #[test]
    fn parse_worktree_porcelain_handles_basic() {
        let input = indoc::indoc! {"
            worktree /repo
            HEAD abcdef0123456789
            branch refs/heads/main

            worktree /repo/.worktrees/feature
            HEAD fedcba9876543210
            branch refs/heads/feature

            worktree /repo/.worktrees/detached
            HEAD 1111111222222222
            detached
        "};
        let entries = parse_worktree_porcelain(input);
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].name, "repo");
        assert_eq!(entries[0].branch, "main");
        assert_eq!(entries[0].short_hash, "abcdef0");
        assert_eq!(entries[1].name, "feature");
        assert_eq!(entries[1].branch, "feature");
        assert_eq!(entries[2].branch, "(detached)");
    }
}
