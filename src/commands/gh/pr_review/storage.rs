use std::path::PathBuf;

use super::error::PrReviewError;

/// Manages local storage for PR review thread files.
///
/// Files are stored under `~/.cache/armyknife/gh-pr-review/<owner>/<repo>/<pr_number>/`.
pub struct ThreadStorage {
    dir: PathBuf,
}

impl ThreadStorage {
    pub fn new(owner: &str, repo: &str, pr_number: u64) -> Self {
        let dir = crate::shared::cache::pr_review_dir()
            .join(owner)
            .join(repo)
            .join(pr_number.to_string());
        Self { dir }
    }

    #[cfg(test)]
    pub fn with_dir(dir: PathBuf) -> Self {
        Self { dir }
    }

    /// Returns the path to the threads.md file.
    pub fn threads_path(&self) -> PathBuf {
        self.dir.join("threads.md")
    }

    /// Write threads.md.
    pub fn write_threads(&self, content: &str) -> Result<(), PrReviewError> {
        std::fs::create_dir_all(&self.dir).map_err(|e| PrReviewError::StorageWriteError {
            path: self.dir.display().to_string(),
            details: e.to_string(),
        })?;

        let path = self.threads_path();
        std::fs::write(&path, content).map_err(|e| PrReviewError::StorageWriteError {
            path: path.display().to_string(),
            details: e.to_string(),
        })?;

        Ok(())
    }

    /// Read threads.md content.
    pub fn read_threads(&self) -> Result<String, PrReviewError> {
        let path = self.threads_path();
        std::fs::read_to_string(&path).map_err(|e| PrReviewError::StorageReadError {
            path: path.display().to_string(),
            details: e.to_string(),
        })
    }

    /// Check whether threads.md exists.
    pub fn exists(&self) -> bool {
        self.threads_path().exists()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;
    use tempfile::TempDir;

    #[rstest]
    fn test_threads_path() {
        let storage = ThreadStorage {
            dir: PathBuf::from("/cache/fohte/armyknife/42"),
        };
        assert_eq!(
            storage.threads_path(),
            PathBuf::from("/cache/fohte/armyknife/42/threads.md")
        );
    }

    #[rstest]
    fn test_write_and_read_threads() {
        let tmp = TempDir::new().unwrap();
        let storage = ThreadStorage::with_dir(tmp.path().join("owner/repo/1"));

        storage.write_threads("# Test content").unwrap();
        assert!(storage.exists());
        assert_eq!(storage.read_threads().unwrap(), "# Test content");
    }
}
