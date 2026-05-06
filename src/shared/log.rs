//! JSONL file logging for armyknife.
//!
//! Initialised once from `main()`; every subsequent `tracing::info!`
//! / `tracing::warn!` call lands as a single JSON line in
//! `~/.cache/armyknife/logs/armyknife.log.YYYY-MM-DD` (daily rotation,
//! 7 files retained).
//!
//! Logger init failures are swallowed: a CLI binary dying because the cache
//! directory was unwritable would be far worse than missing diagnostics.
//! `ARMYKNIFE_LOG` (`off` / `error` / `info` / `debug`, default `info`)
//! controls the level so a debugging session can be turned up without
//! editing config files.
//!
//! See `docs/logging.md` for caller-side conventions and debugging
//! recipes.

use std::path::PathBuf;

use tracing_appender::rolling::{Builder, Rotation};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::fmt;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

const LOG_ENV: &str = "ARMYKNIFE_LOG";
const FILENAME_PREFIX: &str = "armyknife.log";
const RETENTION: usize = 7;

/// Returns the directory where rotated log files live.
/// `~/.cache/armyknife/logs/` on every platform; `None` when no home dir.
pub fn logs_dir() -> Option<PathBuf> {
    super::cache::base_dir().map(|d| d.join("logs"))
}

/// Generates a short 8-character hex id that callers attach to a span as
/// `run_id` so every event emitted within a single `sweep run` / `schedule`
/// worker / hook invocation can be grouped with `jq 'select(.span.run_id ==
/// "abc12345")'`. Full UUIDs are overkill for human-driven log inspection.
pub fn short_run_id() -> String {
    let id = uuid::Uuid::new_v4().simple().to_string();
    id[..8].to_string()
}

/// Initialise the global tracing subscriber.
///
/// Idempotent in the sense that a second call is a no-op
/// (`try_init` returns Err and we ignore it).
pub fn init() {
    let level = std::env::var(LOG_ENV)
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "info".to_string());

    if level.eq_ignore_ascii_case("off") {
        return;
    }

    let Some(dir) = logs_dir() else {
        return;
    };
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }

    let appender = Builder::new()
        .rotation(Rotation::DAILY)
        .filename_prefix(FILENAME_PREFIX)
        .max_log_files(RETENTION)
        .build(&dir);
    let appender = match appender {
        Ok(a) => a,
        Err(_) => return,
    };

    // EnvFilter accepts `info`, `armyknife=debug`, etc. Map our short levels
    // to filter directives so `ARMYKNIFE_LOG=debug` does what users expect.
    let filter = EnvFilter::try_new(&level).unwrap_or_else(|_| EnvFilter::new("info"));

    let layer = fmt::layer()
        .with_writer(appender)
        .with_ansi(false)
        .with_target(true)
        .json()
        .flatten_event(true)
        .with_current_span(true)
        .with_span_list(false);

    let _ = tracing_subscriber::registry()
        .with(filter)
        .with(layer)
        .try_init();
}
