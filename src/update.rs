use self_update::cargo_crate_version;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const REPO_OWNER: &str = "fohte";
const REPO_NAME: &str = "armyknife";
const BIN_NAME: &str = "a";

const CHECK_INTERVAL_SECS: u64 = 24 * 60 * 60; // 24 hours

fn cache_dir() -> Option<PathBuf> {
    dirs::cache_dir().map(|d| d.join("armyknife"))
}

fn last_check_file() -> Option<PathBuf> {
    cache_dir().map(|d| d.join("last_update_check"))
}

fn should_check_for_update_with_path(path: &Path, now_secs: u64) -> bool {
    fs::read_to_string(path)
        .ok()
        .and_then(|contents| contents.trim().parse::<u64>().ok())
        .is_none_or(|last_check| {
            now_secs.saturating_sub(last_check) >= CHECK_INTERVAL_SECS
        })
}

fn should_check_for_update() -> bool {
    let Some(path) = last_check_file() else {
        return true;
    };

    let Ok(now) = SystemTime::now().duration_since(UNIX_EPOCH) else {
        return true;
    };

    should_check_for_update_with_path(&path, now.as_secs())
}

fn write_last_check_time(path: &Path, timestamp: u64) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, timestamp.to_string())
}

fn update_last_check_time() {
    let Some(path) = last_check_file() else {
        return;
    };

    let Ok(now) = SystemTime::now().duration_since(UNIX_EPOCH) else {
        return;
    };

    if let Err(e) = write_last_check_time(&path, now.as_secs()) {
        eprintln!("Failed to write last update check time: {e}");
    }
}

/// Automatically check for updates and apply if available.
/// Runs synchronously but only checks once per 24 hours (cached).
pub fn auto_update() {
    auto_update_impl(should_check_for_update, update_last_check_time, do_update_silent);
}

fn auto_update_impl<C, T, U>(should_check: C, update_time: T, updater: U)
where
    C: FnOnce() -> bool,
    T: FnOnce(),
    U: FnOnce() -> Result<(), Box<dyn std::error::Error>>,
{
    if !should_check() {
        return;
    }

    update_time();

    if let Err(e) = updater() {
        eprintln!("Auto-update failed: {e}");
    }
}

fn base_update_builder() -> self_update::backends::github::UpdateBuilder {
    let mut builder = self_update::backends::github::Update::configure();
    builder
        .repo_owner(REPO_OWNER)
        .repo_name(REPO_NAME)
        .bin_name(BIN_NAME)
        .current_version(cargo_crate_version!());
    builder
}

fn do_update_silent() -> Result<(), Box<dyn std::error::Error>> {
    let mut builder = base_update_builder();
    let status = builder
        .show_download_progress(false)
        .no_confirm(true)
        .build()?
        .update()?;

    if status.updated() {
        eprintln!("Updated to version {}.", status.version());
    }

    Ok(())
}

pub fn do_update() -> Result<(), Box<dyn std::error::Error>> {
    let mut builder = base_update_builder();
    let status = builder
        .show_download_progress(true)
        .build()?
        .update()?;

    if status.updated() {
        println!("Updated to version {}!", status.version());
    } else {
        println!("Already up to date (version {}).", status.version());
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;
    use std::fs;
    use std::sync::atomic::{AtomicBool, Ordering};
    use tempfile::TempDir;

    const NOW: u64 = 1000000;

    #[rstest]
    #[case(None, true)] // file does not exist
    #[case(Some("invalid"), true)] // invalid content
    #[case(Some("996400"), false)] // 1 hour ago (NOW - 3600)
    #[case(Some("913599"), true)] // just over 24 hours ago (NOW - CHECK_INTERVAL_SECS - 1)
    #[case(Some("913600"), true)] // exactly 24 hours ago (NOW - CHECK_INTERVAL_SECS)
    fn should_check_for_update(#[case] content: Option<&str>, #[case] expected: bool) {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("last_update_check");

        if let Some(c) = content {
            fs::write(&path, c).unwrap();
        }

        assert_eq!(should_check_for_update_with_path(&path, NOW), expected);
    }

    #[test]
    fn write_creates_cache_file_with_parent_dirs() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("subdir").join("last_update_check");

        write_last_check_time(&path, 1234567890).unwrap();

        assert_eq!(fs::read_to_string(&path).unwrap(), "1234567890");
    }

    #[rstest]
    #[case(true, true)]
    #[case(false, false)]
    fn auto_update_runs_based_on_check_flag(
        #[case] should_check: bool,
        #[case] expected_called: bool,
    ) {
        let updater_called = AtomicBool::new(false);

        auto_update_impl(
            || should_check,
            || {},
            || {
                updater_called.store(true, Ordering::SeqCst);
                Ok(())
            },
        );

        assert_eq!(updater_called.load(Ordering::SeqCst), expected_called);
    }
}
