use std::cell::RefCell;
use std::path::PathBuf;

thread_local! {
    /// Thread-local override for the cache directory (used in tests).
    static CACHE_DIR_OVERRIDE: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
}

/// Base cache directory for armyknife.
/// Returns ~/.cache/armyknife (Linux) or ~/Library/Caches/armyknife (macOS).
///
/// Can be overridden per-thread using `set_base_dir_override` (for testing).
pub fn base_dir() -> Option<PathBuf> {
    // Check thread-local override first (for tests)
    if let Some(dir) = CACHE_DIR_OVERRIDE.with(|cell| cell.borrow().clone()) {
        return Some(dir);
    }
    dirs::cache_dir().map(|d| d.join("armyknife"))
}

/// Sets a thread-local override for the cache directory.
/// Returns a guard that clears the override when dropped.
///
/// # Example
/// ```ignore
/// let _guard = cache::set_base_dir_override(temp_dir.path().to_path_buf());
/// // cache::base_dir() now returns the temp_dir path
/// // override is cleared when _guard is dropped
/// ```
#[cfg(test)]
pub fn set_base_dir_override(path: PathBuf) -> CacheDirOverrideGuard {
    CACHE_DIR_OVERRIDE.with(|cell| {
        *cell.borrow_mut() = Some(path);
    });
    CacheDirOverrideGuard
}

/// Guard that clears the cache directory override when dropped.
#[cfg(test)]
pub struct CacheDirOverrideGuard;

#[cfg(test)]
impl Drop for CacheDirOverrideGuard {
    fn drop(&mut self) {
        CACHE_DIR_OVERRIDE.with(|cell| {
            *cell.borrow_mut() = None;
        });
    }
}

/// Cache path for update check timestamp.
/// Returns ~/.cache/armyknife/last_update_check
pub fn update_last_check() -> Option<PathBuf> {
    base_dir().map(|d| d.join("last_update_check"))
}

/// Cache path for wm prompt recovery.
/// Returns ~/.cache/armyknife/wm/<repo-name>/prompt.md
pub fn wm_prompt(repo_name: &str) -> Option<PathBuf> {
    base_dir().map(|d| d.join("wm").join(repo_name).join("prompt.md"))
}
