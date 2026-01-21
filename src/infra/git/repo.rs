//! Repository operations.

use git2::{BranchType, Cred, FetchOptions, RemoteCallbacks, Repository};
use std::path::Path;

use super::error::{GitError, Result};
use super::github::get_owner_repo;

/// Open a git repository from the current directory or any parent.
pub fn open_repo() -> Result<Repository> {
    Repository::open_from_env().map_err(|_| GitError::NotInRepo)
}

/// Open a git repository from a specific path.
pub fn open_repo_at(path: &Path) -> Result<Repository> {
    use git2::RepositoryOpenFlags;
    Repository::open_ext(
        path,
        RepositoryOpenFlags::empty(),
        std::iter::empty::<&Path>(),
    )
    .map_err(|_| GitError::NotInRepo)
}

/// Get the main worktree root (the first entry in `git worktree list`).
/// This is always the main repository, regardless of which worktree we're in.
/// For bare repositories, this is the bare repo directory.
/// For regular repositories, this is the main working tree root.
pub fn get_repo_root() -> Result<String> {
    let cwd = std::env::current_dir().map_err(|e| GitError::CommandFailed(e.to_string()))?;
    get_repo_root_in(&cwd)
}

/// Get the main worktree root from the specified directory.
pub fn get_repo_root_in(cwd: &Path) -> Result<String> {
    let repo = open_repo_at(cwd)?;

    let path = if repo.is_worktree() {
        // For worktrees, commondir() points to the main repo's .git
        // The main worktree's workdir is the parent of commondir
        let commondir = repo.commondir();
        commondir.parent().ok_or(GitError::NotInRepo)?
    } else {
        // For the main repo, workdir() gives us the working directory
        repo.workdir().ok_or(GitError::NotInRepo)?
    };

    // Normalize path: remove trailing slash for consistency
    let path_str = path.to_string_lossy();
    Ok(path_str.trim_end_matches('/').to_string())
}

/// Get the current branch name.
/// Returns "HEAD" if in detached HEAD state.
pub fn current_branch(repo: &Repository) -> Result<String> {
    let head = repo.head()?;
    Ok(head.shorthand().unwrap_or("HEAD").to_string())
}

/// macOS-specific system gitconfig paths that libgit2 doesn't recognize.
/// libgit2 only looks at /etc/gitconfig for system config, but macOS has
/// credential.helper configured in these paths.
/// See: https://github.com/libgit2/libgit2/issues/6883
#[cfg(target_os = "macos")]
const MACOS_SYSTEM_CONFIGS: &[&str] = &[
    "/opt/homebrew/etc/gitconfig", // Homebrew (Apple Silicon)
    "/usr/local/etc/gitconfig",    // Homebrew (Intel)
    "/Library/Developer/CommandLineTools/usr/share/git-core/gitconfig", // Xcode CLT
];

/// Build a git config that includes additional system gitconfig paths.
fn build_config_with_system_paths(repo: &Repository) -> Result<git2::Config> {
    #[cfg(target_os = "macos")]
    let extra_paths = MACOS_SYSTEM_CONFIGS;
    #[cfg(not(target_os = "macos"))]
    let extra_paths: &[&str] = &[];

    build_config_with_extra_paths(repo, extra_paths)
}

/// Build a git config with additional config file paths.
///
/// This function adds extra gitconfig files to the repository's config.
/// Files are added at the system level (lowest priority) so they don't
/// override user or repo-level settings.
fn build_config_with_extra_paths(repo: &Repository, extra_paths: &[&str]) -> Result<git2::Config> {
    let mut config = repo.config()?;

    for path_str in extra_paths {
        let path = std::path::Path::new(path_str);
        if path.exists() {
            // Add at system level so it has lowest priority.
            // Intentionally ignore errors: if the file is unreadable or malformed,
            // we should still attempt the fetch with the remaining config.
            let _ = config.add_file(path, git2::ConfigLevel::System, false);
        }
    }

    Ok(config)
}

/// Get the main branch name (main or master)
pub fn get_main_branch() -> Result<String> {
    let repo = open_repo()?;
    get_main_branch_for_repo(&repo)
}

/// Get the main branch name for a specific repository.
///
/// Returns `Ok("main")` if `origin/main` exists, `Ok("master")` if `origin/master` exists,
/// or `Err` if neither exists (caller should fall back to GitHub API or default).
pub fn get_main_branch_for_repo(repo: &Repository) -> Result<String> {
    // Check for origin/main first
    if repo.find_branch("origin/main", BranchType::Remote).is_ok() {
        return Ok("main".to_string());
    }

    // Check for origin/master
    if repo
        .find_branch("origin/master", BranchType::Remote)
        .is_ok()
    {
        return Ok("master".to_string());
    }

    // Neither exists - caller should fall back to GitHub API
    Err(GitError::NotFound(
        "Neither origin/main nor origin/master found".to_string(),
    ))
}

/// Fetch from origin with prune to remove stale remote-tracking references
pub fn fetch_with_prune(repo: &Repository) -> Result<()> {
    let mut remote = repo
        .find_remote("origin")
        .map_err(|e| GitError::CommandFailed(format!("Failed to find origin remote: {e}")))?;

    let config = build_config_with_system_paths(repo)?;

    let mut callbacks = RemoteCallbacks::new();
    callbacks.credentials(|url, username_from_url, allowed_types| {
        // Try SSH agent first for SSH URLs
        if allowed_types.contains(git2::CredentialType::SSH_KEY)
            && let Some(username) = username_from_url
            && let Ok(cred) = Cred::ssh_key_from_agent(username)
        {
            return Ok(cred);
        }

        // For HTTPS, use git2's native credential helper support
        if allowed_types.contains(git2::CredentialType::USER_PASS_PLAINTEXT)
            && let Ok(cred) = Cred::credential_helper(&config, url, username_from_url)
        {
            return Ok(cred);
        }

        // Fallback to default credentials
        Cred::default()
    });

    let mut fetch_opts = FetchOptions::new();
    fetch_opts.prune(git2::FetchPrune::On);
    fetch_opts.remote_callbacks(callbacks);

    remote
        .fetch(&[] as &[&str], Some(&mut fetch_opts), None)
        .map_err(|e| GitError::CommandFailed(format!("git fetch failed: {e}")))?;

    Ok(())
}

/// Get the remote URL for "origin".
pub fn origin_url(repo: &Repository) -> Result<String> {
    let remote = repo
        .find_remote("origin")
        .map_err(|_| GitError::NoOriginRemote)?;
    remote
        .url()
        .map(str::to_string)
        .ok_or(GitError::NoOriginRemote)
}

/// Parse "owner/repo" string into (owner, repo) tuple.
///
/// Validates that the string contains exactly one slash with non-empty parts.
/// Note: If there are multiple slashes (e.g., "org/repo/extra"), only the first
/// slash is used for splitting, resulting in ("org", "repo/extra").
pub fn parse_repo(repo: &str) -> Result<(String, String)> {
    repo.split_once('/')
        .filter(|(owner, name)| !owner.is_empty() && !name.is_empty())
        .map(|(owner, name)| (owner.to_string(), name.to_string()))
        .ok_or_else(|| {
            GitError::InvalidInput(format!(
                "Invalid repository format: {repo}. Expected owner/repo"
            ))
        })
}

/// Get repository owner and name from argument or git remote origin.
///
/// If a repo argument is provided (e.g., "owner/repo"), parses and returns it.
/// Otherwise, attempts to determine the repository from git remote origin.
pub fn get_repo_owner_and_name(repo_arg: Option<&str>) -> Result<(String, String)> {
    if let Some(repo) = repo_arg {
        return parse_repo(repo);
    }

    // Get from git remote origin
    get_owner_repo()
        .ok_or_else(|| GitError::NotFound("Failed to determine current repository".to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::git::test_utils::TempRepo;
    use rstest::rstest;

    /// Helper to create remote tracking branch references in a test repo.
    fn create_remote_branches(repo: &Repository, branches: &[&str]) {
        let head_commit = repo.head().unwrap().peel_to_commit().unwrap();
        for branch in branches {
            repo.reference(
                &format!("refs/remotes/origin/{branch}"),
                head_commit.id(),
                true,
                "create fake remote branch for test",
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

    mod build_config_with_extra_paths {
        use super::*;

        #[rstest]
        #[case::empty_paths(&[])]
        #[case::nonexistent_path(&["/nonexistent/path/to/gitconfig"])]
        #[case::multiple_nonexistent(&["/nonexistent/a", "/nonexistent/b"])]
        fn succeeds_with_various_paths(#[case] paths: &[&str]) {
            let temp = TempRepo::new("owner", "repo", "master");
            let repo = temp.open();

            let result = build_config_with_extra_paths(&repo, paths);

            assert!(result.is_ok());
        }

        #[rstest]
        fn preserves_repo_config_values() {
            let temp = TempRepo::new("owner", "repo", "master");
            let repo = temp.open();

            // Set a value in the repo config
            repo.config()
                .unwrap()
                .set_str("test.value", "from-repo")
                .unwrap();

            let config = build_config_with_extra_paths(&repo, &[]).unwrap();

            // Repo config values should be preserved
            assert_eq!(config.get_string("test.value").unwrap(), "from-repo");
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
            assert!(matches!(result, Err(GitError::InvalidInput(_))));
        }

        #[test]
        fn test_multiple_slashes_takes_first() {
            // split_once splits at first occurrence, so "a/b/c" -> ("a", "b/c")
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
