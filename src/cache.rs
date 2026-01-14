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
    use rstest::rstest;

    #[rstest]
    #[case::base_dir(base_dir(), &["armyknife"])]
    #[case::update_last_check(update_last_check(), &["armyknife", "last_update_check"])]
    #[case::wm_prompt(wm_prompt("my-repo"), &["armyknife", "wm", "my-repo", "prompt.md"])]
    fn path_has_correct_structure(#[case] path: Option<PathBuf>, #[case] expected_parts: &[&str]) {
        let path = path.expect("path should be Some on all supported platforms");
        for part in expected_parts {
            assert!(
                path.to_string_lossy().contains(part),
                "path {:?} should contain {:?}",
                path,
                part
            );
        }
    }
}
