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

/// Internal function for testability.
fn get_issue_dir_with_cache_dir(cache_dir: PathBuf, repo: &str, issue_number: i64) -> PathBuf {
    cache_dir.join(repo).join(issue_number.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_cache_dir_with_xdg() {
        let dir = get_cache_dir_with_env(Some("/tmp/test-cache".to_string()), None);
        assert_eq!(dir, PathBuf::from("/tmp/test-cache/gh-issue-agent"));
    }

    #[test]
    fn test_get_cache_dir_with_home() {
        let dir = get_cache_dir_with_env(None, Some(PathBuf::from("/home/user")));
        assert_eq!(dir, PathBuf::from("/home/user/.cache/gh-issue-agent"));
    }

    #[test]
    fn test_get_cache_dir_xdg_takes_priority() {
        let dir = get_cache_dir_with_env(
            Some("/custom/cache".to_string()),
            Some(PathBuf::from("/home/user")),
        );
        assert_eq!(dir, PathBuf::from("/custom/cache/gh-issue-agent"));
    }

    #[test]
    fn test_get_cache_dir_fallback() {
        let dir = get_cache_dir_with_env(None, None);
        assert_eq!(dir, PathBuf::from(".cache/gh-issue-agent"));
    }

    #[test]
    fn test_get_issue_dir() {
        let cache_dir = PathBuf::from("/cache");
        let dir = get_issue_dir_with_cache_dir(cache_dir, "owner/repo", 123);
        assert_eq!(dir, PathBuf::from("/cache/owner/repo/123"));
    }
}
