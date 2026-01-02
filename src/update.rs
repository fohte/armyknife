use self_update::cargo_crate_version;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

const REPO_OWNER: &str = "fohte";
const REPO_NAME: &str = "armyknife";
const BIN_NAME: &str = "a";

fn check_for_update() -> Option<String> {
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
