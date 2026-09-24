use self_update::cargo_crate_version;
use std::fs;
use std::path::Path;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::shared::cache;
use crate::shared::command;

const REPO_OWNER: &str = "fohte";
const REPO_NAME: &str = "armyknife";
const BIN_NAME: &str = "a";
// Only retry targets that this repository can eventually publish.
const RELEASE_TARGETS: &[&str] = &[
    "aarch64-apple-darwin",
    "x86_64-unknown-linux-gnu",
    "aarch64-unknown-linux-gnu",
];

const CHECK_INTERVAL_SECS: u64 = 24 * 60 * 60; // 24 hours
const RELEASE_ASSET_RETRY_INTERVAL: Duration = Duration::from_secs(10);
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
                message.starts_with("No asset found for target: `")
            }
            _ => false,
        })
}

fn should_retry_release_asset_error(
    error: &anyhow::Error,
    target: &str,
    release_targets: &[&str],
) -> bool {
    release_targets.contains(&target) && is_release_assets_pending(error)
}

fn update_with_retry<T, U, W, N, P>(
    mut update: U,
    mut wait: W,
    mut now: N,
    should_retry: P,
) -> anyhow::Result<T>
where
    U: FnMut() -> anyhow::Result<T>,
    W: FnMut(Duration),
    N: FnMut() -> Duration,
    P: Fn(&anyhow::Error) -> bool,
{
    let started_at = now();
    let mut pending_error = None;

    loop {
        if let Some(error) = pending_error
            .take()
            .filter(|_| now().saturating_sub(started_at) >= RELEASE_ASSET_MAX_WAIT)
        {
            return Err(error);
        }

        match update() {
            Ok(result) => return Ok(result),
            Err(error) if should_retry(&error) => {
                let elapsed = now().saturating_sub(started_at);
                if elapsed >= RELEASE_ASSET_MAX_WAIT {
                    return Err(error);
                }

                let wait_duration = RELEASE_ASSET_RETRY_INTERVAL
                    .min(RELEASE_ASSET_MAX_WAIT.saturating_sub(elapsed));
                wait(wait_duration);
                pending_error = Some(error);
            }
            Err(error) => return Err(error),
        }
    }
}

pub fn do_update() -> anyhow::Result<()> {
    let mut builder = base_update_builder();
    builder.show_download_progress(true).no_confirm(true);
    let initial_updater = builder.build()?;
    builder.show_output(false);
    let retry_updater = builder.build()?;
    let started_at = Instant::now();
    let mut first_attempt = true;
    let status = update_with_retry(
        || {
            let updater = if first_attempt {
                first_attempt = false;
                &initial_updater
            } else {
                &retry_updater
            };
            updater.update().map_err(anyhow::Error::new)
        },
        |duration| {
            println!(
                "Release assets are not available yet. Waiting {} seconds...",
                duration.as_secs()
            );
            std::thread::sleep(duration);
        },
        || started_at.elapsed(),
        |error| should_retry_release_asset_error(error, self_update::get_target(), RELEASE_TARGETS),
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
    #[case::missing_target_asset("No asset found for target: `example-target`", true)]
    #[case::missing_assets_array("No assets found", false)]
    #[case::no_release("No releases found", false)]
    #[case::other_release_error("release API failed", false)]
    fn is_release_assets_pending_cases(#[case] message: &str, #[case] expected: bool) {
        let error = anyhow::Error::new(self_update::errors::Error::Release(message.to_string()));

        assert_eq!(is_release_assets_pending(&error), expected);
    }

    #[test]
    fn release_asset_errors_are_retried_only_for_published_targets() {
        let error = anyhow::Error::new(self_update::errors::Error::Release(
            "No asset found for target: `example-target`".to_string(),
        ));

        assert_eq!(
            (
                should_retry_release_asset_error(&error, "example-target", &["example-target"]),
                should_retry_release_asset_error(&error, "unsupported-target", &["example-target"]),
            ),
            (true, false),
        );
    }

    #[derive(Debug, PartialEq)]
    struct RetryOutcome {
        result: Result<String, String>,
        attempts: usize,
        waits: Vec<Duration>,
    }

    #[rstest]
    #[case::succeeds_after_retries(
        "No asset found for target: `example-target`",
        Some(2),
        Duration::ZERO,
        RetryOutcome {
            result: Ok("updated".to_string()),
            attempts: 3,
            waits: vec![Duration::from_secs(10); 2],
        },
    )]
    #[case::non_asset_error_returns_immediately(
        "No releases found",
        None,
        Duration::ZERO,
        RetryOutcome {
            result: Err("ReleaseError: No releases found".to_string()),
            attempts: 1,
            waits: vec![],
        },
    )]
    #[case::deadline_caps_final_wait(
        "No asset found for target: `example-target`",
        None,
        Duration::from_secs(1795),
        RetryOutcome {
            result: Err("ReleaseError: No asset found for target: `example-target`".to_string()),
            attempts: 1,
            waits: vec![Duration::from_secs(5)],
        },
    )]
    fn update_with_retry_cases(
        #[case] error_message: &str,
        #[case] failures_before_success: Option<usize>,
        #[case] attempt_duration: Duration,
        #[case] expected: RetryOutcome,
    ) {
        let attempts = Cell::new(0);
        let elapsed = Cell::new(Duration::ZERO);
        let waits = RefCell::new(Vec::new());

        let result = update_with_retry(
            || {
                let attempt = attempts.get();
                attempts.set(attempt + 1);
                elapsed.set(elapsed.get() + attempt_duration);
                if failures_before_success.is_some_and(|failures| attempt >= failures) {
                    return Ok("updated");
                }

                Err(anyhow::Error::new(self_update::errors::Error::Release(
                    error_message.to_string(),
                )))
            },
            |duration| {
                waits.borrow_mut().push(duration);
                elapsed.set(elapsed.get() + duration);
            },
            || elapsed.get(),
            |error| should_retry_release_asset_error(error, "example-target", &["example-target"]),
        );

        assert_eq!(
            RetryOutcome {
                result: result
                    .map(|value| value.to_string())
                    .map_err(|error| error.to_string()),
                attempts: attempts.get(),
                waits: waits.into_inner(),
            },
            expected,
        );
    }
}
