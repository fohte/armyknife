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

use std::fs::File;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use chrono::{NaiveDate, Utc};
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

/// Today's log file path, matching the daily rotation scheme used by
/// `tracing_appender`. Returns `None` when no cache dir is available.
pub fn current_log_path() -> Option<PathBuf> {
    log_path_for_date(Utc::now().date_naive())
}

/// Log file path for a specific UTC date. Useful for tail readers that
/// follow a single day's worth of events.
pub fn log_path_for_date(date: NaiveDate) -> Option<PathBuf> {
    logs_dir().map(|d| d.join(format!("{FILENAME_PREFIX}.{date}")))
}

/// Read JSON-parsed lines past `cursor` from a rotating log file.
/// Skips the trailing partial line so callers see only complete events.
/// Returns the new cursor; pass it back on the next call to resume.
///
/// Missing files are treated as empty so callers can `start_clean_tail`
/// before the producer's first write.
pub fn read_jsonl_lines_since(
    path: &Path,
    mut cursor: u64,
) -> std::io::Result<(Vec<serde_json::Value>, u64)> {
    let mut file = match File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok((Vec::new(), cursor));
        }
        Err(e) => return Err(e),
    };
    let len = file.metadata()?.len();
    if len < cursor {
        // Rotation / truncation: restart from the top.
        cursor = 0;
    }
    file.seek(SeekFrom::Start(cursor))?;
    let mut buf = String::new();
    file.read_to_string(&mut buf)?;
    let (consumable, _trailing) = match buf.rfind('\n') {
        Some(idx) => (&buf[..=idx], &buf[idx + 1..]),
        None => ("", buf.as_str()),
    };
    let mut events = Vec::new();
    for line in consumable.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        // Skip undecodable lines instead of aborting: one corrupt line
        // must not stop a live progress display or a summary scan.
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(trimmed) {
            events.push(v);
        }
    }
    let new_cursor = cursor + consumable.len() as u64;
    Ok((events, new_cursor))
}

/// Walk the rotating log files newest-first (up to `max_days_back`
/// days) and return the most recent event whose top-level JSON matches
/// `predicate`. Within each file, lines are scanned bottom-up so the
/// freshest event wins.
pub fn find_latest_event(
    predicate: impl Fn(&serde_json::Value) -> bool,
    max_days_back: usize,
) -> Option<serde_json::Value> {
    let today = Utc::now().date_naive();
    for day_offset in 0..=max_days_back {
        let date = today.checked_sub_signed(chrono::TimeDelta::days(day_offset as i64))?;
        let Some(path) = log_path_for_date(date) else {
            continue;
        };
        let Ok(file) = File::open(&path) else {
            continue;
        };
        // Collect the file's lines newest-first. Daily log files are
        // small enough that buffering them in memory is cheaper than
        // a real reverse line reader.
        let lines: Vec<String> = BufReader::new(file).lines().map_while(Result::ok).collect();
        for line in lines.into_iter().rev() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(trimmed)
                && predicate(&v)
            {
                return Some(v);
            }
        }
    }
    None
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

#[cfg(test)]
mod tests {
    use super::*;
    use indoc::indoc;
    use rstest::rstest;
    use serde_json::json;
    use std::fs;
    use tempfile::TempDir;

    #[rstest]
    fn read_jsonl_lines_since_returns_complete_lines() {
        let tmp = TempDir::new().expect("tempdir");
        let log = tmp.path().join("log");
        fs::write(
            &log,
            indoc! {r#"
                {"a":1}
                {"b":2}
            "#},
        )
        .expect("write");

        let (events, cursor) = read_jsonl_lines_since(&log, 0).expect("read");
        assert_eq!(events, vec![json!({"a":1}), json!({"b":2})]);
        let (events, _) = read_jsonl_lines_since(&log, cursor).expect("re-read");
        assert!(events.is_empty());
    }

    #[rstest]
    fn read_jsonl_lines_since_skips_partial_tail() {
        let tmp = TempDir::new().expect("tempdir");
        let log = tmp.path().join("log");
        let payload = concat!("{\"a\":1}\n", "{\"b\":");
        fs::write(&log, payload).expect("write");

        let (events, cursor) = read_jsonl_lines_since(&log, 0).expect("read");
        assert_eq!(events, vec![json!({"a":1})]);
        assert!(cursor < payload.len() as u64);
    }

    #[rstest]
    fn read_jsonl_lines_since_handles_missing_file() {
        let tmp = TempDir::new().expect("tempdir");
        let (events, cursor) =
            read_jsonl_lines_since(&tmp.path().join("missing"), 0).expect("read");
        assert!(events.is_empty());
        assert_eq!(cursor, 0);
    }
}
