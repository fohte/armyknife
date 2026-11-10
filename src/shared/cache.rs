#[cfg(test)]
use std::path::Path;
use std::path::PathBuf;

/// Base cache directory for armyknife.
/// Production builds return ~/.cache/armyknife on all platforms.
#[cfg(not(test))]
pub fn base_dir() -> Option<PathBuf> {
    super::dirs::cache_dir().map(|d| d.join("armyknife"))
}

/// Keep tests that use the default cache path away from user data.
#[cfg(test)]
pub fn base_dir() -> Option<PathBuf> {
    let temp_dir = std::env::temp_dir();
    let cache_home = super::dirs::cache_dir()
        .filter(|path| is_in_temp_dir(path, &temp_dir))
        .unwrap_or_else(|| temp_dir.join(format!("armyknife-test-cache-{}", std::process::id())));

    Some(cache_home.join("armyknife"))
}

#[cfg(test)]
fn is_in_temp_dir(path: &Path, temp_dir: &Path) -> bool {
    let temp_root = temp_dir
        .ancestors()
        .find(|ancestor| {
            ancestor.file_name().is_some_and(|name| {
                name.eq_ignore_ascii_case("tmp") || name.eq_ignore_ascii_case("temp")
            })
        })
        .unwrap_or(temp_dir);

    canonicalize_with_missing_suffix(path)
        .zip(canonicalize_with_missing_suffix(temp_root))
        .is_some_and(|(path, temp_root)| path.starts_with(temp_root))
}

#[cfg(test)]
fn canonicalize_with_missing_suffix(path: &Path) -> Option<PathBuf> {
    let mut existing = path;
    let mut missing_suffix = Vec::new();

    loop {
        if let Ok(mut canonical_path) = existing.canonicalize() {
            for component in missing_suffix.into_iter().rev() {
                canonical_path.push(component);
            }
            return Some(canonical_path);
        }

        missing_suffix.push(existing.file_name()?.to_os_string());
        existing = existing.parent()?;
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

/// Cache directory for gh pr-review.
/// Returns ~/.cache/armyknife/gh-pr-review, falling back to .cache/armyknife/gh-pr-review
/// when the home directory cannot be determined.
pub fn pr_review_dir() -> PathBuf {
    base_dir()
        .map(|d| d.join("gh-pr-review"))
        .unwrap_or_else(|| {
            PathBuf::from(".cache")
                .join("armyknife")
                .join("gh-pr-review")
        })
}

/// Cache directory for gh issue-agent.
/// Returns ~/.cache/armyknife/gh-issue-agent, falling back to .cache/armyknife/gh-issue-agent
/// when the home directory cannot be determined.
pub fn issue_agent_dir() -> PathBuf {
    base_dir()
        .map(|d| d.join("gh-issue-agent"))
        .unwrap_or_else(|| {
            PathBuf::from(".cache")
                .join("armyknife")
                .join("gh-issue-agent")
        })
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use rstest::rstest;

    use super::base_dir;

    #[rstest]
    #[case::xdg_cache_home(Some("/example/cache"), Some("/example/home"))]
    #[case::home_cache(None, Some("/example/home"))]
    fn base_dir_uses_a_process_specific_temporary_directory_by_default(
        #[case] xdg_cache_home: Option<&str>,
        #[case] home: Option<&str>,
    ) {
        let expected = Some(
            std::env::temp_dir()
                .join(format!("armyknife-test-cache-{}", std::process::id()))
                .join("armyknife"),
        );

        temp_env::with_vars([("XDG_CACHE_HOME", xdg_cache_home), ("HOME", home)], || {
            assert_eq!(base_dir(), expected)
        });
    }

    #[test]
    fn base_dir_uses_an_explicit_temporary_cache_home() {
        let cache_home = tempfile::tempdir().expect("temporary cache home should be created");
        let expected = Some(PathBuf::from(cache_home.path()).join("armyknife"));
        let cache_home_path = cache_home.path().to_str().expect("path should be UTF-8");

        temp_env::with_vars([("XDG_CACHE_HOME", Some(cache_home_path))], || {
            assert_eq!(base_dir(), expected);
        });
    }
}
