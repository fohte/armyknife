use std::path::{Path, PathBuf};

use super::paths::{get_issue_dir, get_new_issue_dir};

/// Storage handler for a single GitHub issue.
///
/// Provides read/write operations for issue data stored locally.
/// Directory structure:
/// ```text
/// <cache_dir>/<owner>/<repo>/<issue_number>/
/// ├── issue.md
/// ├── metadata.json
/// └── comments/
///     ├── 001_comment_<databaseId>.md
///     └── new_<name>.md
/// ```
#[derive(Debug, Clone)]
pub struct IssueStorage {
    pub(super) dir: PathBuf,
}

impl IssueStorage {
    /// Create a new IssueStorage for the given repo and issue number.
    /// Uses the default cache directory.
    pub fn new(repo: &str, issue_number: i64) -> Self {
        Self {
            dir: get_issue_dir(repo, issue_number),
        }
    }

    /// Create a new IssueStorage for a new issue (not yet created on GitHub).
    /// Uses the "new" directory under the repository cache.
    pub fn new_for_new_issue(repo: &str) -> Self {
        Self {
            dir: get_new_issue_dir(repo),
        }
    }

    /// Create an IssueStorage from an existing directory path.
    /// Useful for custom storage locations (e.g., after creating a new issue).
    pub fn from_dir(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    /// Returns the directory path for this issue storage.
    pub fn dir(&self) -> &Path {
        &self.dir
    }
}
