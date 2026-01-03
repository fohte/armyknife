use self_update::cargo_crate_version;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

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
    let Ok(contents) = fs::read_to_string(path) else {
        return true;
    };

    let Ok(last_check) = contents.trim().parse::<u64>() else {
        return true;
    };

    now_secs.saturating_sub(last_check) >= CHECK_INTERVAL_SECS
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

    let _ = write_last_check_time(&path, now.as_secs());
}

fn check_for_update() -> Option<String> {
    if !should_check_for_update() {
        return None;
    }

    // Update timestamp before fetching to avoid repeated requests if fetch is slow
    update_last_check_time();

    let releases = self_update::backends::github::ReleaseList::configure()
        .repo_owner(REPO_OWNER)
        .repo_name(REPO_NAME)
        .build()
        .ok()?
        .fetch()
        .ok()?;

    let latest = releases.first()?;
    let current = semver::Version::parse(cargo_crate_version!()).ok()?;
    let latest_version = semver::Version::parse(&latest.version).ok()?;

    if latest_version > current {
        Some(latest.version.clone())
    } else {
        None
    }
}

pub fn spawn_update_check() -> mpsc::Receiver<Option<String>> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let result = check_for_update();
        let _ = tx.send(result);
    });
    rx
}

pub fn print_update_notification(rx: mpsc::Receiver<Option<String>>) {
    // Wait up to 500ms for the update check to complete
    // This balances responsiveness with giving the check time to finish
    if let Ok(Some(version)) = rx.recv_timeout(Duration::from_millis(500)) {
        eprintln!();
        eprintln!("A new version of armyknife is available: v{version}");
        eprintln!("Run `a update` to update to the latest version.");
    }
}

pub fn do_update() -> Result<(), Box<dyn std::error::Error>> {
    let status = self_update::backends::github::Update::configure()
        .repo_owner(REPO_OWNER)
        .repo_name(REPO_NAME)
        .bin_name(BIN_NAME)
        .show_download_progress(true)
        .current_version(cargo_crate_version!())
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
}
