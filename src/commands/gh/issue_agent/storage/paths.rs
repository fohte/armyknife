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
    fn test_get_issue_dir_appends_repo_and_number() {
        temp_env::with_vars(
            [
                ("XDG_CACHE_HOME", Some("/tmp/test-cache")),
                ("HOME", Some("/home/user")),
            ],
            || {
                let dir = get_issue_dir("fohte/armyknife", 42);
                assert_eq!(
                    dir,
                    PathBuf::from("/tmp/test-cache/armyknife/gh-issue-agent/fohte/armyknife/42")
                );
            },
        );
    }

    #[rstest]
    fn test_get_new_issue_dir_appends_repo_and_new() {
        temp_env::with_vars(
            [
                ("XDG_CACHE_HOME", Some("/tmp/test-cache")),
                ("HOME", Some("/home/user")),
            ],
            || {
                let dir = get_new_issue_dir("fohte/armyknife");
                assert_eq!(
                    dir,
                    PathBuf::from("/tmp/test-cache/armyknife/gh-issue-agent/fohte/armyknife/new")
                );
            },
        );
    }
}
