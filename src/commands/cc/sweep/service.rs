//! launchd agent lifecycle for `a cc sweep`.
//!
//! On macOS, `a cc sweep install` writes a per-user LaunchAgent plist to
//! `~/Library/LaunchAgents/fohte.armyknife.cc-sweep.plist` that runs
//! `a cc sweep run` on a `StartInterval` of 60 seconds, then bootstraps it
//! into the current GUI domain. `uninstall` reverses both steps. `status`
//! prints whether the service is currently bootstrapped.
//!
//! Mirrors the install/uninstall pattern used by skhd and yabai so that it
//! behaves predictably for users already familiar with those tools.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

use anyhow::{Context, Result, bail};

const SERVICE_LABEL: &str = "fohte.armyknife.cc-sweep";
/// How often launchd should invoke `a cc sweep run`. One minute is short
/// enough that a 30-minute idle timeout still feels responsive (worst-case
/// user wait is timeout + 60s), and long enough not to drain battery.
const START_INTERVAL_SECS: u32 = 60;

/// Install the LaunchAgent plist and bootstrap it.
pub fn install() -> Result<()> {
    ensure_macos()?;
    let plist_path = plist_path()?;
    let exe = std::env::current_exe().context("locating current executable")?;

    if let Some(parent) = plist_path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }

    let plist = render_plist(&exe.to_string_lossy());
    fs::write(&plist_path, plist).with_context(|| format!("writing {}", plist_path.display()))?;

    // Bootstrap into the current GUI domain so it starts running now and at
    // next login. If it is already bootstrapped we intentionally continue --
    // the install command is idempotent for the file write, and launchctl
    // will return a non-zero exit which we surface as a warning.
    let uid = unsafe { libc::getuid() };
    let domain = format!("gui/{uid}");
    let target = format!("{domain}/{SERVICE_LABEL}");

    // If the service is already bootstrapped, bootout first so we pick up
    // any changes to the plist (e.g., updated executable path after an
    // `a update`).
    if is_bootstrapped(&target) {
        run_launchctl(&["bootout", &target]).ok();
    }

    run_launchctl(&["bootstrap", &domain, plist_path.to_string_lossy().as_ref()])
        .context("bootstrapping launchd service")?;

    eprintln!("[armyknife] installed {}", plist_path.display());
    eprintln!("[armyknife] bootstrapped {target}");
    Ok(())
}

/// Remove the LaunchAgent plist and bootout the service.
pub fn uninstall() -> Result<()> {
    ensure_macos()?;
    let plist_path = plist_path()?;
    let uid = unsafe { libc::getuid() };
    let domain = format!("gui/{uid}");
    let target = format!("{domain}/{SERVICE_LABEL}");

    if is_bootstrapped(&target) {
        // bootout may return non-zero if the service is already gone; ignore.
        run_launchctl(&["bootout", &target]).ok();
    }

    if plist_path.exists() {
        fs::remove_file(&plist_path)
            .with_context(|| format!("removing {}", plist_path.display()))?;
        eprintln!("[armyknife] removed {}", plist_path.display());
    } else {
        eprintln!("[armyknife] no plist at {}", plist_path.display());
    }

    Ok(())
}

/// Print the current status of the service.
pub fn status() -> Result<()> {
    ensure_macos()?;
    let plist_path = plist_path()?;
    let uid = unsafe { libc::getuid() };
    let target = format!("gui/{uid}/{SERVICE_LABEL}");

    println!("label:        {SERVICE_LABEL}");
    println!("plist:        {}", plist_path.display());
    println!(
        "plist exists: {}",
        if plist_path.exists() { "yes" } else { "no" }
    );
    println!(
        "bootstrapped: {}",
        if is_bootstrapped(&target) {
            "yes"
        } else {
            "no"
        }
    );
    println!("interval:     {START_INTERVAL_SECS}s");
    Ok(())
}

fn ensure_macos() -> Result<()> {
    if cfg!(target_os = "macos") {
        Ok(())
    } else {
        bail!("`a cc sweep install/uninstall/status` is only supported on macOS")
    }
}

fn plist_path() -> Result<PathBuf> {
    let home = std::env::var_os("HOME").context("HOME env var not set")?;
    Ok(PathBuf::from(home)
        .join("Library")
        .join("LaunchAgents")
        .join(format!("{SERVICE_LABEL}.plist")))
}

fn is_bootstrapped(target: &str) -> bool {
    // `launchctl print` exits 0 iff the service is currently bootstrapped.
    Command::new("launchctl")
        .args(["print", target])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn run_launchctl(args: &[&str]) -> Result<()> {
    let output = Command::new("launchctl")
        .args(args)
        .output()
        .context("spawning launchctl")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("launchctl {:?} failed: {}", args, stderr.trim());
    }
    Ok(())
}

fn render_plist(exe: &str) -> String {
    // StandardErrorPath points at a file under the cache dir that armyknife
    // already owns, so log rotation can be done externally (e.g., by user).
    // We do not set StandardOutPath because `a cc sweep run` only prints on
    // error (the per-run stderr summary is the only output when paused > 0).
    let log_path = log_path_string();
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{SERVICE_LABEL}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{exe}</string>
        <string>cc</string>
        <string>sweep</string>
        <string>run</string>
    </array>
    <key>StartInterval</key>
    <integer>{START_INTERVAL_SECS}</integer>
    <key>RunAtLoad</key>
    <true/>
    <key>StandardErrorPath</key>
    <string>{log_path}</string>
    <key>ProcessType</key>
    <string>Background</string>
</dict>
</plist>
"#
    )
}

fn log_path_string() -> String {
    // Best-effort: if the cache dir resolver fails (no HOME), fall back to
    // /tmp so launchd still has a writable path.
    match crate::shared::cache::base_dir() {
        Some(d) => d
            .join("cc")
            .join("logs")
            .join("sweep.log")
            .to_string_lossy()
            .into_owned(),
        None => "/tmp/armyknife-cc-sweep.log".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use indoc::formatdoc;

    use super::*;

    #[test]
    fn render_plist_matches_expected_layout() {
        // Full-text comparison keeps the test hermetic and catches drifts in
        // any field (label, executable path, interval, log path) at once.
        let expected = formatdoc! {r#"
            <?xml version="1.0" encoding="UTF-8"?>
            <!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
            <plist version="1.0">
            <dict>
                <key>Label</key>
                <string>{label}</string>
                <key>ProgramArguments</key>
                <array>
                    <string>/usr/local/bin/a</string>
                    <string>cc</string>
                    <string>sweep</string>
                    <string>run</string>
                </array>
                <key>StartInterval</key>
                <integer>{interval}</integer>
                <key>RunAtLoad</key>
                <true/>
                <key>StandardErrorPath</key>
                <string>{log}</string>
                <key>ProcessType</key>
                <string>Background</string>
            </dict>
            </plist>
            "#,
            label = SERVICE_LABEL,
            interval = START_INTERVAL_SECS,
            log = log_path_string(),
        };
        assert_eq!(render_plist("/usr/local/bin/a"), expected);
    }
}
