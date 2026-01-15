use std::path::PathBuf;

/// Returns the cache directory for gh-issue-agent.
/// Uses XDG_CACHE_HOME if set, otherwise defaults to ~/.cache/gh-issue-agent
pub fn get_cache_dir() -> PathBuf {
    if let Ok(xdg_cache) = std::env::var("XDG_CACHE_HOME") {
        PathBuf::from(xdg_cache).join("gh-issue-agent")
    } else if let Some(home) = dirs::home_dir() {
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
mod tests {
    use super::*;
    use serial_test::serial;

    #[test]
    #[serial]
    fn test_get_issue_dir() {
        // Ensure XDG_CACHE_HOME is not set for this test
        let original = std::env::var("XDG_CACHE_HOME").ok();
        // SAFETY: Test is serialized and we restore the original value after
        unsafe {
            std::env::remove_var("XDG_CACHE_HOME");
        }

        let dir = get_issue_dir("owner/repo", 123);
        let cache_dir = get_cache_dir();
        assert_eq!(dir, cache_dir.join("owner/repo").join("123"));

        // Restore env
        // SAFETY: Test is serialized and we're restoring the original value
        unsafe {
            if let Some(val) = original {
                std::env::set_var("XDG_CACHE_HOME", val);
            }
        }
    }

    #[test]
    #[serial]
    fn test_get_cache_dir_with_xdg() {
        // Save current env
        let original = std::env::var("XDG_CACHE_HOME").ok();

        // SAFETY: Test is serialized and we restore the original value after
        unsafe {
            std::env::set_var("XDG_CACHE_HOME", "/tmp/test-cache");
        }
        let dir = get_cache_dir();
        assert_eq!(dir, PathBuf::from("/tmp/test-cache/gh-issue-agent"));

        // Restore env
        // SAFETY: Test is serialized and we're restoring the original value
        unsafe {
            match original {
                Some(val) => std::env::set_var("XDG_CACHE_HOME", val),
                None => std::env::remove_var("XDG_CACHE_HOME"),
            }
        }
    }
}
