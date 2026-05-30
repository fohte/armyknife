//! Repository operations.
//!
//! Wraps the `git` CLI with a [`GitRepo`] handle so callers can perform
//! repository queries without depending on libgit2.

use std::path::{Path, PathBuf};

use anyhow::Context;

use super::cmd::{run_git, run_git_optional};
use super::error::{GitError, Result};
use super::github::get_owner_repo;

/// Handle to a git repository (regular or linked worktree).
///
/// Holds the worktree's working directory and the shared `.git` common dir,
/// which is enough to dispatch any subsequent `git -C <workdir>` call.
#[derive(Debug, Clone)]
pub struct GitRepo {
    workdir: PathBuf,
    common_dir: PathBuf,
    is_worktree: bool,
}

impl GitRepo {
    /// Open the repository containing the current working directory.
    pub fn open_from_env() -> Result<Self> {
        let cwd = std::env::current_dir().map_err(|_| GitError::NotInRepo)?;
        Self::open_at(&cwd)
    }

    /// Open the repository containing `path`.
    pub fn open_at(path: &Path) -> Result<Self> {
        // Probe with --show-toplevel; non-zero exit means "not a git dir".
        let workdir =
            run_git(path, ["rev-parse", "--show-toplevel"]).map_err(|_| GitError::NotInRepo)?;
        if workdir.is_empty() {
            return Err(GitError::NotInRepo.into());
        }
        let workdir = PathBuf::from(workdir);

        let common_dir_raw = run_git(&workdir, ["rev-parse", "--git-common-dir"])
            .map_err(|_| GitError::NotInRepo)?;
        let git_dir_raw =
            run_git(&workdir, ["rev-parse", "--git-dir"]).map_err(|_| GitError::NotInRepo)?;

        // In a linked worktree `--git-dir` resolves to
        // `<common-dir>/worktrees/<name>` while `--git-common-dir` resolves to
        // `<common-dir>` itself; in the main worktree the two are identical.
        // Compare raw strings first (covers the common case without touching
        // the filesystem), then fall back to absolute-path comparison.
        let is_worktree = if git_dir_raw == common_dir_raw {
            false
        } else {
            let abs_common = resolve_relative(&workdir, &common_dir_raw);
            let abs_git = resolve_relative(&workdir, &git_dir_raw);
            abs_git != abs_common
        };
        let common_dir = resolve_relative(&workdir, &common_dir_raw);

        Ok(Self {
            workdir,
            common_dir,
            is_worktree,
        })
    }

    pub fn workdir(&self) -> &Path {
        &self.workdir
    }

    pub fn is_worktree(&self) -> bool {
        self.is_worktree
    }

    /// Main worktree path (parent of `.git` common dir).
    pub fn main_workdir(&self) -> Result<PathBuf> {
        self.common_dir
            .parent()
            .map(|p| p.to_path_buf())
            .ok_or_else(|| GitError::NotInRepo.into())
    }

    /// Re-open the handle rooted at the main worktree.
    pub fn main_repo(&self) -> Result<Self> {
        if self.is_worktree {
            let main = self.main_workdir()?;
            Self::open_at(&main)
        } else {
            Ok(self.clone())
        }
    }

    /// Current branch name. Returns `"HEAD"` when detached.
    pub fn current_branch(&self) -> Result<String> {
        match run_git(&self.workdir, ["symbolic-ref", "--short", "HEAD"]) {
            Ok(s) if !s.is_empty() => Ok(s),
            _ => Ok("HEAD".to_string()),
        }
    }

    pub fn local_branch_exists(&self, branch: &str) -> bool {
        run_git_optional(
            &self.workdir,
            [
                "show-ref",
                "--verify",
                "--quiet",
                &format!("refs/heads/{branch}"),
            ],
        )
        .is_some()
    }

    pub fn remote_branch_exists(&self, branch: &str) -> bool {
        run_git_optional(
            &self.workdir,
            [
                "show-ref",
                "--verify",
                "--quiet",
                &format!("refs/remotes/{branch}"),
            ],
        )
        .is_some()
    }

    pub fn origin_url(&self) -> Result<String> {
        run_git(&self.workdir, ["remote", "get-url", "origin"])
            .map_err(|_| GitError::NoOriginRemote.into())
    }

    pub fn fetch_origin_prune(&self) -> Result<()> {
        run_git(&self.workdir, ["fetch", "--prune", "origin"]).map(|_| ())
    }

    pub fn delete_branch(&self, branch: &str) -> Result<()> {
        run_git(&self.workdir, ["branch", "-D", branch]).map(|_| ())
    }

    pub fn short_hash(&self, rev: &str) -> Result<String> {
        run_git(&self.workdir, ["rev-parse", "--short=7", rev])
    }
}

fn resolve_relative(base: &Path, raw: &str) -> PathBuf {
    let p = PathBuf::from(raw);
    if p.is_absolute() { p } else { base.join(p) }
}

/// Open a git repository from the current directory or any parent.
pub fn open_repo() -> Result<GitRepo> {
    GitRepo::open_from_env()
}

/// Open a git repository from a specific path.
pub fn open_repo_at(path: &Path) -> Result<GitRepo> {
    GitRepo::open_at(path)
}

/// Get the main worktree root (the first entry in `git worktree list`).
pub fn get_repo_root() -> Result<String> {
    let cwd = std::env::current_dir().context("Failed to get current directory")?;
    get_repo_root_in(&cwd)
}

/// Get the main worktree root from the specified directory.
pub fn get_repo_root_in(cwd: &Path) -> Result<String> {
    let repo = open_repo_at(cwd)?;
    let path = repo.main_workdir()?;
    let path_str = path.to_string_lossy();
    Ok(path_str.trim_end_matches('/').to_string())
}

/// Get the current branch name.
pub fn current_branch(repo: &GitRepo) -> Result<String> {
    repo.current_branch()
}

/// Get the main branch name (main or master).
pub fn get_main_branch() -> Result<String> {
    let repo = open_repo()?;
    get_main_branch_for_repo(&repo)
}

/// Get the main branch name for a specific repository.
pub fn get_main_branch_for_repo(repo: &GitRepo) -> Result<String> {
    if repo.remote_branch_exists("origin/main") {
        return Ok("main".to_string());
    }
    if repo.remote_branch_exists("origin/master") {
        return Ok("master".to_string());
    }
    Err(GitError::NotFound("Neither origin/main nor origin/master found".to_string()).into())
}

/// Fetch from origin with prune to remove stale remote-tracking references.
pub fn fetch_with_prune(repo: &GitRepo) -> Result<()> {
    repo.fetch_origin_prune()
}

/// Get the remote URL for "origin".
pub fn origin_url(repo: &GitRepo) -> Result<String> {
    repo.origin_url()
}

/// Parse "owner/repo" string into (owner, repo) tuple.
pub fn parse_repo(repo: &str) -> Result<(String, String)> {
    repo.split_once('/')
        .filter(|(owner, name)| !owner.is_empty() && !name.is_empty())
        .map(|(owner, name)| (owner.to_string(), name.to_string()))
        .ok_or_else(|| {
            GitError::InvalidInput(format!(
                "Invalid repository format: {repo}. Expected owner/repo"
            ))
            .into()
        })
}

/// Get repository owner and name from argument or git remote origin.
pub fn get_repo_owner_and_name(repo_arg: Option<&str>) -> Result<(String, String)> {
    if let Some(repo) = repo_arg {
        return parse_repo(repo);
    }
    get_owner_repo().ok_or_else(|| {
        GitError::NotFound("Failed to determine current repository".to_string()).into()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::git::test_utils::TempRepo;
    use rstest::rstest;

    fn create_remote_branches(repo: &GitRepo, branches: &[&str]) {
        let head_sha = run_git(repo.workdir(), ["rev-parse", "HEAD"]).unwrap();
        for branch in branches {
            run_git(
                repo.workdir(),
                [
                    "update-ref",
                    &format!("refs/remotes/origin/{branch}"),
                    head_sha.as_str(),
                ],
            )
            .unwrap();
        }
    }

    #[rstest]
    #[case::only_origin_main(vec!["main"], Some("main"))]
    #[case::only_origin_master(vec!["master"], Some("master"))]
    #[case::both_prefers_main(vec!["main", "master"], Some("main"))]
    #[case::no_remote_branches(vec![], None)]
    fn test_get_main_branch_for_repo(
        #[case] remote_branches: Vec<&str>,
        #[case] expected: Option<&str>,
    ) {
        let temp = TempRepo::new("owner", "repo", "master");
        let repo = temp.open();

        create_remote_branches(&repo, &remote_branches);

        let result = get_main_branch_for_repo(&repo);
        match expected {
            Some(branch) => assert_eq!(result.unwrap(), branch),
            None => assert!(result.is_err()),
        }
    }

    mod parse_repo_tests {
        use super::*;

        #[rstest]
        #[case::valid("owner/repo", ("owner", "repo"))]
        #[case::with_dashes("my-org/my-repo", ("my-org", "my-repo"))]
        #[case::with_numbers("org123/repo456", ("org123", "repo456"))]
        #[case::with_dots("org.name/repo.name", ("org.name", "repo.name"))]
        fn test_valid(#[case] input: &str, #[case] expected: (&str, &str)) {
            let result = parse_repo(input).unwrap();
            assert_eq!(result, (expected.0.to_string(), expected.1.to_string()));
        }

        #[rstest]
        #[case::no_slash("ownerrepo")]
        #[case::empty("")]
        #[case::only_slash("/")]
        #[case::empty_owner("/repo")]
        #[case::empty_repo("owner/")]
        fn test_invalid(#[case] input: &str) {
            let result = parse_repo(input);
            assert!(result.is_err());
            let err = result.unwrap_err();
            assert!(
                err.downcast_ref::<GitError>()
                    .is_some_and(|e| matches!(e, GitError::InvalidInput(_)))
            );
        }

        #[test]
        fn test_multiple_slashes_takes_first() {
            let result = parse_repo("org/repo/extra").unwrap();
            assert_eq!(result, ("org".to_string(), "repo/extra".to_string()));
        }
    }

    mod get_repo_owner_and_name_tests {
        use super::*;

        #[rstest]
        #[case::simple("owner/repo")]
        #[case::real_repo("fohte/armyknife")]
        #[case::with_special_chars("my-org/my_repo.rs")]
        fn test_with_arg_returns_parsed(#[case] repo: &str) {
            let (owner, name) = get_repo_owner_and_name(Some(repo)).unwrap();
            let expected: Vec<&str> = repo.splitn(2, '/').collect();
            assert_eq!(owner, expected[0]);
            assert_eq!(name, expected[1]);
        }

        #[rstest]
        #[case::no_slash("invalid")]
        #[case::empty("")]
        fn test_invalid_format(#[case] input: &str) {
            let result = get_repo_owner_and_name(Some(input));
            assert!(result.is_err());
        }
    }
}
