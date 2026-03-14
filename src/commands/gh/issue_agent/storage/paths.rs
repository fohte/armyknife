use std::path::PathBuf;

/// Returns the cache directory for gh-issue-agent.
/// Uses the unified armyknife cache directory (~/.cache/armyknife/gh-issue-agent).
fn get_cache_dir() -> PathBuf {
    crate::shared::cache::issue_agent_dir().unwrap_or_else(|| {
        PathBuf::from(".cache")
            .join("armyknife")
            .join("gh-issue-agent")
    })
}

/// Returns the directory path for a specific issue.
/// Format: <cache_dir>/<repo>/<issue_number>
pub fn get_issue_dir(repo: &str, issue_number: i64) -> PathBuf {
    get_cache_dir().join(repo).join(issue_number.to_string())
}

/// Returns the directory path for a new issue (not yet created on GitHub).
/// Format: <cache_dir>/<repo>/new
pub fn get_new_issue_dir(repo: &str) -> PathBuf {
    get_cache_dir().join(repo).join("new")
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[rstest]
    #[case::issue_dir("/cache", "owner/repo", 123, "/cache/owner/repo/123")]
    #[case::issue_dir_full(
        "/home/user/.cache/armyknife/gh-issue-agent",
        "fohte/armyknife",
        42,
        "/home/user/.cache/armyknife/gh-issue-agent/fohte/armyknife/42"
    )]
    fn test_get_issue_dir_with_base(
        #[case] base: &str,
        #[case] repo: &str,
        #[case] issue_number: i64,
        #[case] expected: &str,
    ) {
        let dir = PathBuf::from(base)
            .join(repo)
            .join(issue_number.to_string());
        assert_eq!(dir, PathBuf::from(expected));
    }

    #[rstest]
    #[case::new_issue("/cache", "owner/repo", "/cache/owner/repo/new")]
    #[case::new_issue_full(
        "/home/user/.cache/armyknife/gh-issue-agent",
        "fohte/armyknife",
        "/home/user/.cache/armyknife/gh-issue-agent/fohte/armyknife/new"
    )]
    fn test_get_new_issue_dir_with_base(
        #[case] base: &str,
        #[case] repo: &str,
        #[case] expected: &str,
    ) {
        let dir = PathBuf::from(base).join(repo).join("new");
        assert_eq!(dir, PathBuf::from(expected));
    }
}
