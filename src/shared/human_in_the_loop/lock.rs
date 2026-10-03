use std::fs;
use std::path::{Path, PathBuf};

use super::error::Result;
use crate::infra::tmux;

/// RAII guard for the lock file owned by a review launcher.
pub struct LockGuard {
    lock_path: PathBuf,
    lock_file: fs::File,
    disarmed: bool,
}

impl LockGuard {
    /// Create a lock file and return a guard.
    pub fn acquire(document_path: &Path) -> Result<Self> {
        let lock_path = Self::lock_path(document_path);
        let lock_file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&lock_path)?;
        Ok(Self {
            lock_path,
            lock_file,
            disarmed: false,
        })
    }

    /// Check if a lock file exists for the given document.
    pub fn is_locked(document_path: &Path) -> bool {
        Self::lock_path(document_path).exists()
    }

    /// Get the lock file path for a document.
    pub fn lock_path(document_path: &Path) -> PathBuf {
        let mut lock_path = document_path.as_os_str().to_os_string();
        lock_path.push(".lock");
        PathBuf::from(lock_path)
    }

    /// Prevent this guard from removing the lock file on drop.
    pub fn disarm(&mut self) {
        self.disarmed = true;
    }
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        if !self.disarmed
            && let (Ok(owned), Ok(path)) = (
                self.lock_file.metadata(),
                fs::symlink_metadata(&self.lock_path),
            )
            && same_file(&owned, &path)
        {
            let _ = fs::remove_file(&self.lock_path);
        }
    }
}

fn same_file(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;

    left.dev() == right.dev() && left.ino() == right.ino()
}

/// Child-side cleanup guard for normal review completion.
///
/// The parent also holds the lock so it can remove it if the child is killed.
/// This guard removes the lock if the parent exits before the editor does.
pub struct CleanupGuard {
    lock_path: PathBuf,
    tmux_target: Option<String>,
}

impl CleanupGuard {
    pub fn new(document_path: &Path, tmux_target: Option<String>) -> Self {
        Self {
            lock_path: LockGuard::lock_path(document_path),
            tmux_target,
        }
    }
}

impl Drop for CleanupGuard {
    fn drop(&mut self) {
        // Remove lock file
        let _ = fs::remove_file(&self.lock_path);

        if let Some(ref target) = self.tmux_target {
            let _ = tmux::focus_pane(target);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::{fixture, rstest};
    use tempfile::TempDir;

    struct ReviewDocumentFixture {
        _temp_dir: TempDir,
        path: PathBuf,
    }

    #[fixture]
    fn review_document() -> ReviewDocumentFixture {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("test.md");
        fs::write(&path, "content").unwrap();
        ReviewDocumentFixture {
            _temp_dir: temp_dir,
            path,
        }
    }

    #[rstest]
    #[case::markdown("file.md", "file.md.lock")]
    #[case::text("file.txt", "file.txt.lock")]
    #[case::extensionless("file", "file.lock")]
    #[case::compound_extension("file.tar.gz", "file.tar.gz.lock")]
    fn lock_path_appends_lock_extension(#[case] filename: &str, #[case] expected: &str) {
        let path = PathBuf::from(filename);
        let lock_path = LockGuard::lock_path(&path);
        assert_eq!(lock_path, PathBuf::from(expected));
    }

    #[rstest]
    fn lock_guard_creates_and_removes_lock_file(review_document: ReviewDocumentFixture) {
        let doc_path = &review_document.path;
        let lock_path = LockGuard::lock_path(doc_path);

        let initially_locked = LockGuard::is_locked(doc_path);
        let while_held;
        {
            let _guard = LockGuard::acquire(doc_path).unwrap();
            while_held = (lock_path.exists(), LockGuard::is_locked(doc_path));
        }
        let after_drop = (lock_path.exists(), LockGuard::is_locked(doc_path));

        assert_eq!(
            (initially_locked, while_held, after_drop),
            (false, (true, true), (false, false)),
        );
    }

    #[rstest]
    fn lock_guard_does_not_replace_an_existing_lock(review_document: ReviewDocumentFixture) {
        let doc_path = &review_document.path;
        let lock_path = LockGuard::lock_path(doc_path);
        fs::write(&lock_path, "another review").unwrap();
        let acquired = LockGuard::acquire(doc_path).is_ok();
        let contents = fs::read_to_string(lock_path).unwrap();

        assert_eq!((acquired, contents), (false, "another review".into()));
    }

    #[rstest]
    fn lock_guard_does_not_remove_a_replacement_lock(review_document: ReviewDocumentFixture) {
        let doc_path = &review_document.path;
        let old_guard = LockGuard::acquire(doc_path).unwrap();
        fs::remove_file(LockGuard::lock_path(doc_path)).unwrap();
        let new_guard = LockGuard::acquire(doc_path).unwrap();
        drop(old_guard);
        let replacement_remains = LockGuard::is_locked(doc_path);
        drop(new_guard);

        assert_eq!(
            (replacement_remains, LockGuard::is_locked(doc_path)),
            (true, false),
        );
    }

    #[rstest]
    fn lock_guard_disarm_prevents_cleanup(review_document: ReviewDocumentFixture) {
        let doc_path = &review_document.path;
        let lock_path = LockGuard::lock_path(doc_path);

        let while_held;
        {
            let mut guard = LockGuard::acquire(doc_path).unwrap();
            while_held = lock_path.exists();
            guard.disarm();
        }

        assert_eq!((while_held, lock_path.exists()), (true, true));
    }

    #[rstest]
    fn cleanup_guard_removes_lock_file(review_document: ReviewDocumentFixture) {
        let doc_path = &review_document.path;
        let lock_path = LockGuard::lock_path(doc_path);
        fs::write(&lock_path, "").unwrap();

        let existed_before_cleanup = lock_path.exists();

        {
            let _guard = CleanupGuard::new(doc_path, None);
        }

        // Lock file should be removed when cleanup guard is dropped
        let exists_after_cleanup = lock_path.exists();
        assert_eq!(
            (existed_before_cleanup, exists_after_cleanup),
            (true, false)
        );
    }
}
