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
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn should_check_when_file_does_not_exist() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("last_update_check");
        let now = 1000000;

        assert!(should_check_for_update_with_path(&path, now));
    }

    #[test]
    fn should_check_when_file_contains_invalid_content() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("last_update_check");
        fs::write(&path, "invalid").unwrap();
        let now = 1000000;

        assert!(should_check_for_update_with_path(&path, now));
    }

    #[test]
    fn should_not_check_when_recently_checked() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("last_update_check");
        let now = 1000000u64;
        let last_check = now - 3600; // 1 hour ago
        fs::write(&path, last_check.to_string()).unwrap();

        assert!(!should_check_for_update_with_path(&path, now));
    }

    #[test]
    fn should_check_when_interval_passed() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("last_update_check");
        let now = 1000000u64;
        let last_check = now - CHECK_INTERVAL_SECS - 1; // Just over 24 hours ago
        fs::write(&path, last_check.to_string()).unwrap();

        assert!(should_check_for_update_with_path(&path, now));
    }

    #[test]
    fn should_check_at_exact_interval() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("last_update_check");
        let now = 1000000u64;
        let last_check = now - CHECK_INTERVAL_SECS; // Exactly 24 hours ago
        fs::write(&path, last_check.to_string()).unwrap();

        assert!(should_check_for_update_with_path(&path, now));
    }

    #[test]
    fn write_creates_cache_file() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("subdir").join("last_update_check");
        let timestamp = 1234567890u64;

        write_last_check_time(&path, timestamp).unwrap();

        let contents = fs::read_to_string(&path).unwrap();
        assert_eq!(contents, "1234567890");
    }
}
