use std::path::PathBuf;

/// Environment variable to override the cache directory (primarily for testing).
const CACHE_DIR_ENV: &str = "ARMYKNIFE_CACHE_DIR";

/// Base cache directory for armyknife.
/// Returns ~/.cache/armyknife (Linux) or ~/Library/Caches/armyknife (macOS).
///
/// Can be overridden by setting the `ARMYKNIFE_CACHE_DIR` environment variable.
pub fn base_dir() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var(CACHE_DIR_ENV) {
        return Some(PathBuf::from(dir));
    }
    dirs::cache_dir().map(|d| d.join("armyknife"))
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
