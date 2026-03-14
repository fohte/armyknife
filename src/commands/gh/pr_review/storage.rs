use std::path::PathBuf;

use sha2::{Digest, Sha256};

use super::error::PrReviewError;

/// Manages local storage for PR review thread files.
///
/// Files are stored under `$XDG_CACHE_HOME/gh-pr-review/<owner>/<repo>/<pr_number>/`.
pub struct ThreadStorage {
    dir: PathBuf,
}

impl ThreadStorage {
    pub fn new(owner: &str, repo: &str, pr_number: u64) -> Self {
        let dir = crate::shared::cache::pr_review_dir()
            .unwrap_or_else(|| {
                std::path::PathBuf::from(".cache")
                    .join("armyknife")
                    .join("gh-pr-review")
            })
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

    /// Returns the path to the hash file used for local change detection.
    fn hash_path(&self) -> PathBuf {
        self.dir.join(".threads.md.sha256")
    }

    /// Write threads.md and record its hash for change detection.
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

        // Record hash for local change detection
        let hash = compute_hash(content);
        std::fs::write(self.hash_path(), hash).map_err(|e| PrReviewError::StorageWriteError {
            path: self.hash_path().display().to_string(),
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

    /// Detect whether the local threads.md has been modified since last pull.
    pub fn has_local_changes(&self) -> Result<bool, PrReviewError> {
        if !self.exists() {
            return Ok(false);
        }

        let hash_path = self.hash_path();
        if !hash_path.exists() {
            // No recorded hash means we can't tell; treat as changed
            return Ok(true);
        }

        let current_content = self.read_threads()?;
        let current_hash = compute_hash(&current_content);

        let stored_hash =
            std::fs::read_to_string(&hash_path).map_err(|e| PrReviewError::StorageReadError {
                path: hash_path.display().to_string(),
                details: e.to_string(),
            })?;

        Ok(current_hash != stored_hash)
    }
}

fn compute_hash(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    format!("{:x}", hasher.finalize())
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

    #[rstest]
    fn test_has_local_changes_no_file() {
        let tmp = TempDir::new().unwrap();
        let storage = ThreadStorage::with_dir(tmp.path().join("owner/repo/1"));
        assert!(!storage.has_local_changes().unwrap());
    }

    #[rstest]
    fn test_has_local_changes_unmodified() {
        let tmp = TempDir::new().unwrap();
        let storage = ThreadStorage::with_dir(tmp.path().join("owner/repo/1"));

        storage.write_threads("content").unwrap();
        assert!(!storage.has_local_changes().unwrap());
    }

    #[rstest]
    fn test_has_local_changes_modified() {
        let tmp = TempDir::new().unwrap();
        let storage = ThreadStorage::with_dir(tmp.path().join("owner/repo/1"));

        storage.write_threads("original").unwrap();
        // Modify the file directly (simulating user edits)
        std::fs::write(storage.threads_path(), "modified").unwrap();
        assert!(storage.has_local_changes().unwrap());
    }

    #[rstest]
    fn test_has_local_changes_no_hash_file() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join("owner/repo/1");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("threads.md"), "content").unwrap();

        let storage = ThreadStorage::with_dir(dir);
        // No hash file -> treat as changed
        assert!(storage.has_local_changes().unwrap());
    }
}
