use std::sync::OnceLock;
use tempfile::TempDir;

use crate::shared::human_in_the_loop::TEST_APPROVAL_DIR_OVERRIDE;

static APPROVAL_DIR: OnceLock<TempDir> = OnceLock::new();

/// Redirect `ApprovalManager` writes into a single per-process TempDir
/// for the duration of the test run. Sets an internal OnceLock instead
/// of `ARMYKNIFE_APPROVAL_DIR` so no `setenv` ever races with reader
/// threads. Safe to call from any number of tests concurrently.
pub fn init_approval_dir() {
    let dir = APPROVAL_DIR.get_or_init(|| TempDir::new().expect("approval tempdir"));
    let _ = TEST_APPROVAL_DIR_OVERRIDE.set(dir.path().to_path_buf());
}
