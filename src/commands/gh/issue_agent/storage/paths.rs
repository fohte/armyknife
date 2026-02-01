use std::path::PathBuf;

/// Returns the cache directory for gh-issue-agent.
/// Uses XDG_CACHE_HOME if set, otherwise defaults to ~/.cache/gh-issue-agent
pub fn get_cache_dir() -> PathBuf {
    get_cache_dir_with_env(std::env::var("XDG_CACHE_HOME").ok(), dirs::home_dir())
}

/// Internal function for testability without environment variable mutation.
fn get_cache_dir_with_env(xdg_cache_home: Option<String>, home_dir: Option<PathBuf>) -> PathBuf {
    if let Some(xdg_cache) = xdg_cache_home {
        PathBuf::from(xdg_cache).join("gh-issue-agent")
    } else if let Some(home) = home_dir {
        home.join(".cache").join("gh-issue-agent")
    } else {
        // Fallback to current directory if home is not available
        PathBuf::from(".cache").join("gh-issue-agent")
    }
}

/// Returns the directory path for a specific issue.
/// Format: <cache_dir>/<owner>/<repo>/<issue_number>
pub fn get_issue_dir(repo: &str, issue_number: i64) -> PathBuf {
    get_cache_dir().join(repo).join(issue_number.to_string())
}

#[cfg(test)]
/// Internal function for testability.
fn get_issue_dir_with_cache_dir(cache_dir: PathBuf, repo: &str, issue_number: i64) -> PathBuf {
    cache_dir.join(repo).join(issue_number.to_string())
}

#[cfg(test)]
fn get_new_issue_dir_with_cache_dir(cache_dir: PathBuf, repo: &str) -> PathBuf {
    cache_dir.join(repo).join("new")
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[rstest]
    #[case(Some("/tmp/test-cache".to_string()), None, "/tmp/test-cache/gh-issue-agent")]
    #[case(
        None,
        Some(PathBuf::from("/home/user")),
        "/home/user/.cache/gh-issue-agent"
    )]
    #[case(Some("/custom/cache".to_string()), Some(PathBuf::from("/home/user")), "/custom/cache/gh-issue-agent")]
    #[case(None, None, ".cache/gh-issue-agent")]
    fn test_get_cache_dir_with_env(
        #[case] xdg_cache_home: Option<String>,
        #[case] home_dir: Option<PathBuf>,
        #[case] expected: &str,
    ) {
        let dir = get_cache_dir_with_env(xdg_cache_home, home_dir);
        assert_eq!(dir, PathBuf::from(expected));
    }

    #[rstest]
    #[case("/cache", "owner/repo", 123, "/cache/owner/repo/123")]
    #[case(
        "/home/user/.cache/gh-issue-agent",
        "fohte/armyknife",
        42,
        "/home/user/.cache/gh-issue-agent/fohte/armyknife/42"
    )]
    fn test_get_issue_dir_with_cache_dir(
        #[case] cache_dir: &str,
        #[case] repo: &str,
        #[case] issue_number: i64,
        #[case] expected: &str,
    ) {
        let dir = get_issue_dir_with_cache_dir(PathBuf::from(cache_dir), repo, issue_number);
        assert_eq!(dir, PathBuf::from(expected));
    }

    #[rstest]
    #[case("/cache", "owner/repo", "/cache/owner/repo/new")]
    #[case(
        "/home/user/.cache/gh-issue-agent",
        "fohte/armyknife",
        "/home/user/.cache/gh-issue-agent/fohte/armyknife/new"
    )]
    fn test_get_new_issue_dir_with_cache_dir(
        #[case] cache_dir: &str,
        #[case] repo: &str,
        #[case] expected: &str,
    ) {
        let dir = get_new_issue_dir_with_cache_dir(PathBuf::from(cache_dir), repo);
        assert_eq!(dir, PathBuf::from(expected));
    }
}
