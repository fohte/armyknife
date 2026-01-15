use std::path::PathBuf;

/// Base cache directory for armyknife.
/// Returns ~/.cache/armyknife (Linux) or ~/Library/Caches/armyknife (macOS).
pub fn base_dir() -> Option<PathBuf> {
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
