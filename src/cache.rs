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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_dir_returns_some() {
        // dirs::cache_dir() should return Some on all supported platforms
        assert!(base_dir().is_some());
    }

    #[test]
    fn base_dir_ends_with_armyknife() {
        let path = base_dir().unwrap();
        assert!(path.ends_with("armyknife"));
    }

    #[test]
    fn update_last_check_has_correct_structure() {
        let path = update_last_check().unwrap();
        assert!(path.ends_with("last_update_check"));
        assert!(path.parent().unwrap().ends_with("armyknife"));
    }

    #[test]
    fn wm_prompt_has_correct_structure() {
        let path = wm_prompt("my-repo").unwrap();
        assert!(path.ends_with("prompt.md"));
        assert!(path.parent().unwrap().ends_with("my-repo"));
        assert!(path.parent().unwrap().parent().unwrap().ends_with("wm"));
    }
}
