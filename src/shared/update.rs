use self_update::cargo_crate_version;
use std::fs;
use std::path::Path;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::shared::cache;
use crate::shared::command;

const REPO_OWNER: &str = "fohte";
const REPO_NAME: &str = "armyknife";
const BIN_NAME: &str = "a";

const CHECK_INTERVAL_SECS: u64 = 24 * 60 * 60; // 24 hours
const RELEASE_ASSET_RETRY_INTERVAL: Duration = Duration::from_secs(30);
const RELEASE_ASSET_MAX_WAIT: Duration = Duration::from_secs(30 * 60);

fn should_check_for_update_with_path(path: &Path, now_secs: u64) -> bool {
    fs::read_to_string(path)
        .ok()
        .and_then(|contents| contents.trim().parse::<u64>().ok())
        .is_none_or(|last_check| now_secs.saturating_sub(last_check) >= CHECK_INTERVAL_SECS)
}

fn should_check_for_update() -> bool {
    let Some(path) = cache::update_last_check() else {
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
    let Some(path) = cache::update_last_check() else {
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
/// Only checks once per 24 hours (cached).
/// Runs in a separate blocking thread to avoid nested tokio runtime issues.
pub async fn auto_update() {
    run_update_with(
        should_check_for_update,
        update_last_check_time,
        do_update_silent,
    )
    .await;
}

async fn run_update_with<C, T, U>(should_check: C, update_time: T, updater: U)
where
    C: FnOnce() -> bool + Send + 'static,
    T: FnOnce() + Send + 'static,
    U: FnOnce() -> Result<(), Box<dyn std::error::Error + Send + Sync>> + Send + 'static,
{
    if !should_check() {
        return;
    }

    update_time();

    // Run the updater in a separate blocking thread to avoid nested runtime issues.
    // self_update crate creates its own tokio runtime internally.
    let result = tokio::task::spawn_blocking(updater).await;

    match result {
        Ok(Ok(())) => {}
        Ok(Err(e)) => eprintln!("Auto-update failed: {e}"),
        Err(e) => eprintln!("Auto-update task failed: {e}"),
    }
}

const TOKEN_ENV_VARS: &[&str] = &["ARMYKNIFE_GITHUB_TOKEN", "GITHUB_TOKEN", "GH_TOKEN"];

fn env_var(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

fn gh_auth_token() -> Option<String> {
    if !command::is_command_available("gh") {
        return None;
    }
    let output = command::new("gh").args(["auth", "token"]).output().ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout).ok()
}

fn non_empty_trimmed(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

fn resolve_github_token_with<E, G>(env_lookup: E, gh_fallback: G) -> Option<String>
where
    E: Fn(&str) -> Option<String>,
    G: FnOnce() -> Option<String>,
{
    for name in TOKEN_ENV_VARS {
        if let Some(value) = env_lookup(name).and_then(|v| non_empty_trimmed(&v)) {
            return Some(value);
        }
    }
    gh_fallback().and_then(|v| non_empty_trimmed(&v))
}

fn resolve_github_token() -> Option<String> {
    resolve_github_token_with(env_var, gh_auth_token)
}

fn base_update_builder() -> self_update::backends::github::UpdateBuilder {
    let mut builder = self_update::backends::github::Update::configure();
    builder
        .repo_owner(REPO_OWNER)
        .repo_name(REPO_NAME)
        .bin_name(BIN_NAME)
        .current_version(cargo_crate_version!());
    if let Some(token) = resolve_github_token() {
        builder.auth_token(&token);
    }
    builder
}

fn do_update_silent() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
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

fn is_release_assets_pending(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<self_update::errors::Error>()
        .is_some_and(|error| match error {
            self_update::errors::Error::Release(message) => {
                message == "No assets found"
                    || (message.starts_with("No asset found for target: `")
                        && message.ends_with('`'))
            }
            _ => false,
        })
}

fn update_with_retry<T, U, W, N>(mut update: U, mut wait: W, mut now: N) -> anyhow::Result<T>
where
    U: FnMut() -> anyhow::Result<T>,
    W: FnMut(Duration),
    N: FnMut() -> Duration,
{
    let started_at = now();

    loop {
        match update() {
            Ok(result) => return Ok(result),
            Err(error) if is_release_assets_pending(&error) => {
                let elapsed = now().saturating_sub(started_at);
                if elapsed >= RELEASE_ASSET_MAX_WAIT {
                    return Err(error);
                }

                let wait_duration = RELEASE_ASSET_RETRY_INTERVAL
                    .min(RELEASE_ASSET_MAX_WAIT.saturating_sub(elapsed));
                wait(wait_duration);
            }
            Err(error) => return Err(error),
        }
    }
}

pub fn do_update() -> anyhow::Result<()> {
    let updater = base_update_builder()
        .show_download_progress(true)
        .no_confirm(true)
        .build()?;
    let started_at = Instant::now();
    let status = update_with_retry(
        || updater.update().map_err(anyhow::Error::new),
        |duration| {
            println!(
                "Release assets are not available yet. Retrying in {} seconds...",
                duration.as_secs()
            );
            std::thread::sleep(duration);
        },
        || started_at.elapsed(),
    )?;

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
    use std::cell::{Cell, RefCell};
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

    #[rstest]
    #[case::armyknife_wins_over_others(
        &[("ARMYKNIFE_GITHUB_TOKEN", "ak"), ("GITHUB_TOKEN", "gt"), ("GH_TOKEN", "ght")],
        Some("from-gh-cli"),
        Some("ak"),
    )]
    #[case::github_token_when_armyknife_absent(
        &[("GITHUB_TOKEN", "gt"), ("GH_TOKEN", "ght")],
        Some("from-gh-cli"),
        Some("gt"),
    )]
    #[case::gh_token_when_others_absent(
        &[("GH_TOKEN", "ght")],
        Some("from-gh-cli"),
        Some("ght"),
    )]
    #[case::empty_env_skipped_falls_back_to_gh(
        &[("ARMYKNIFE_GITHUB_TOKEN", ""), ("GITHUB_TOKEN", "   ")],
        Some("from-gh-cli"),
        Some("from-gh-cli"),
    )]
    #[case::gh_output_trimmed(
        &[],
        Some("  trimmed\n"),
        Some("trimmed"),
    )]
    #[case::gh_empty_returns_none(
        &[],
        Some("  \n"),
        None,
    )]
    #[case::gh_unavailable_returns_none(
        &[],
        None,
        None,
    )]
    fn resolve_github_token_cases(
        #[case] env: &[(&str, &str)],
        #[case] gh_output: Option<&str>,
        #[case] expected: Option<&str>,
    ) {
        let env_map: std::collections::HashMap<String, String> = env
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect();

        let result = resolve_github_token_with(
            |name| env_map.get(name).cloned(),
            || gh_output.map(String::from),
        );

        assert_eq!(result, expected.map(String::from));
    }

    #[test]
    fn write_creates_cache_file_with_parent_dirs() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path().join("subdir").join("last_update_check");

        write_last_check_time(&path, 1234567890).unwrap();

        assert_eq!(fs::read_to_string(&path).unwrap(), "1234567890");
    }

    #[rstest]
    #[case::empty_release_assets("No assets found", true)]
    #[case::missing_target_asset("No asset found for target: `example-target`", true)]
    #[case::no_release("No releases found", false)]
    #[case::other_release_error("release API failed", false)]
    fn is_release_assets_pending_cases(#[case] message: &str, #[case] expected: bool) {
        let error = anyhow::Error::new(self_update::errors::Error::Release(message.to_string()));

        assert_eq!(is_release_assets_pending(&error), expected);
    }

    #[test]
    fn update_retries_pending_release_assets_until_success() {
        let attempts = Cell::new(0);
        let elapsed = Cell::new(Duration::ZERO);
        let waits = RefCell::new(Vec::new());

        let result = update_with_retry(
            || {
                let attempt = attempts.get();
                attempts.set(attempt + 1);
                if attempt < 2 {
                    Err(anyhow::Error::new(self_update::errors::Error::Release(
                        "No assets found".to_string(),
                    )))
                } else {
                    Ok("updated")
                }
            },
            |duration| {
                waits.borrow_mut().push(duration);
                elapsed.set(elapsed.get() + duration);
            },
            || elapsed.get(),
        );

        assert_eq!(
            (
                result.map_err(|error| error.to_string()),
                attempts.get(),
                waits.into_inner(),
            ),
            (Ok("updated"), 3, vec![Duration::from_secs(30); 2],),
        );
    }

    #[test]
    fn update_returns_non_asset_errors_without_waiting() {
        let attempts = Cell::new(0);
        let waits = RefCell::new(Vec::new());

        let result: anyhow::Result<()> = update_with_retry(
            || {
                attempts.set(attempts.get() + 1);
                Err(anyhow::Error::new(self_update::errors::Error::Release(
                    "No releases found".to_string(),
                )))
            },
            |duration| waits.borrow_mut().push(duration),
            || Duration::ZERO,
        );

        assert_eq!(
            (
                result.map_err(|error| error.to_string()),
                attempts.get(),
                waits.into_inner()
            ),
            (
                Err("ReleaseError: No releases found".to_string()),
                1,
                vec![]
            ),
        );
    }

    #[test]
    fn update_returns_last_asset_error_after_max_wait() {
        let attempts = Cell::new(0);
        let elapsed = Cell::new(Duration::ZERO);
        let waits = RefCell::new(Vec::new());

        let result: anyhow::Result<()> = update_with_retry(
            || {
                attempts.set(attempts.get() + 1);
                Err(anyhow::Error::new(self_update::errors::Error::Release(
                    "No assets found".to_string(),
                )))
            },
            |duration| {
                waits.borrow_mut().push(duration);
                elapsed.set(elapsed.get() + duration);
            },
            || elapsed.get(),
        );

        assert_eq!(
            (
                result.map_err(|error| error.to_string()),
                attempts.get(),
                waits.into_inner(),
            ),
            (
                Err("ReleaseError: No assets found".to_string()),
                61,
                vec![Duration::from_secs(30); 60],
            ),
        );
    }
}
