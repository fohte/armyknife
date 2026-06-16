use std::sync::OnceLock;
use tempfile::TempDir;

static APPROVAL_DIR: OnceLock<TempDir> = OnceLock::new();

/// Point `ARMYKNIFE_APPROVAL_DIR` at a single per-process TempDir so the
/// production `ApprovalManager` writes approval records into a sandbox-
/// writable location during tests. Each test still uses its own document
/// TempDir, so the HMAC-derived ids never collide across parallel tests.
///
/// Safe to call from many tests concurrently — the TempDir is created
/// exactly once.
#[expect(
    clippy::disallowed_methods,
    reason = "set_var runs once per process inside OnceLock init"
)]
pub fn init_approval_dir() {
    APPROVAL_DIR.get_or_init(|| {
        let dir = TempDir::new().expect("approval tempdir");
        // SAFETY: executed exactly once per process, before any test thread
        // reads ARMYKNIFE_APPROVAL_DIR, so getenv cannot race with setenv.
        unsafe { std::env::set_var("ARMYKNIFE_APPROVAL_DIR", dir.path()) };
        dir
    });
}
