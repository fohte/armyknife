use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::SystemTime;

use anyhow::{Context, Result, anyhow};
use chrono::{DateTime, Utc};
use serde_json::Value;

use super::model::{ContextUsage, EngineQuota, RateLimitWindow, RateLimits, SessionUsage};
use crate::commands::agent::types::Engine;

const INITIAL_QUOTA_READ_SIZE: usize = 8 * 1024;
const MAX_QUOTA_READ_SIZE: usize = 64 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
struct FileSignature {
    modified: SystemTime,
    length: u64,
}

#[derive(Clone)]
struct CachedQuota {
    signature: FileSignature,
    quota: Option<EngineQuota>,
}

static ROLLOUT_QUOTA_CACHE: OnceLock<Mutex<HashMap<PathBuf, CachedQuota>>> = OnceLock::new();

pub(crate) fn read_transcript(path: &Path, session_id: &str) -> Option<SessionUsage> {
    let file = File::open(path).ok()?;
    let mut model = None;
    let mut effort = None;
    let mut version = None;
    let mut context = None;
    let mut rate_limits = RateLimits::default();
    let mut observed_at = None;

    for line in BufReader::new(file)
        .lines()
        .map_while(std::result::Result::ok)
    {
        let Ok(record) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        match record.get("type").and_then(Value::as_str) {
            Some("turn_context") => {
                if let Some(payload) = record.get("payload") {
                    if let Some(value) = payload.get("model").and_then(Value::as_str) {
                        model = Some(value.to_owned());
                    }
                    if let Some(value) = payload
                        .get("effort")
                        .or_else(|| payload.get("reasoning_effort"))
                        .and_then(Value::as_str)
                    {
                        effort = Some(value.to_owned());
                    }
                }
            }
            Some("session_meta") => {
                version = record
                    .pointer("/payload/cli_version")
                    .and_then(Value::as_str)
                    .or_else(|| record.get("cli_version").and_then(Value::as_str))
                    .map(str::to_owned)
                    .or(version);
            }
            Some("event_msg")
                if record.pointer("/payload/type").and_then(Value::as_str)
                    == Some("token_count") =>
            {
                let payload = record.get("payload");
                let info = payload.and_then(|payload| payload.get("info"));
                if let Some(info) = info
                    && let Some(usage) = info
                        .get("last_token_usage")
                        .or_else(|| info.get("total_token_usage"))
                {
                    context = codex_context(usage, info.get("model_context_window"));
                }
                let parsed_rate_limits = codex_rate_limits(
                    payload
                        .and_then(|payload| payload.get("rate_limits"))
                        .or_else(|| info.and_then(|info| info.get("rate_limits"))),
                );
                if parsed_rate_limits.has_any() {
                    rate_limits = parsed_rate_limits;
                }
                observed_at = record
                    .get("timestamp")
                    .and_then(Value::as_str)
                    .and_then(parse_timestamp);
            }
            _ => {}
        }
    }

    (model.is_some()
        || effort.is_some()
        || context.is_some()
        || rate_limits.has_any()
        || version.is_some())
    .then(|| SessionUsage {
        session_id: session_id.to_owned(),
        engine: Engine::Codex,
        model,
        effort,
        context,
        rate_limits,
        version,
        observed_at: observed_at.unwrap_or_else(Utc::now),
    })
}

pub(crate) fn read_latest_quota_from(codex_home: &Path) -> Result<Option<EngineQuota>> {
    let mut paths = Vec::new();
    for root in [
        codex_home.join("sessions"),
        codex_home.join("archived_sessions"),
    ] {
        paths.extend(rollout_paths_from(&root)?);
    }
    paths.sort();
    paths.dedup();

    let mut rollouts = Vec::new();
    for path in paths {
        if let Some(signature) = file_signature(&path)? {
            rollouts.push((path, signature));
        }
    }
    // Quotas follow the latest rollout update, so older files are only fallbacks.
    rollouts.sort_by(|(left_path, left), (right_path, right)| {
        right
            .modified
            .cmp(&left.modified)
            .then_with(|| left_path.cmp(right_path))
    });

    let current_paths = rollouts
        .iter()
        .map(|(path, _)| path.clone())
        .collect::<HashSet<_>>();
    let cache = ROLLOUT_QUOTA_CACHE.get_or_init(Default::default);
    cache
        .lock()
        .map_err(|_| anyhow!("Codex quota cache lock is poisoned"))?
        .retain(|path, _| !path.starts_with(codex_home) || current_paths.contains(path));

    for (path, signature) in rollouts {
        if let Some(quota) = cached_quota_from(&path, signature)? {
            return Ok(Some(quota));
        }
    }

    Ok(None)
}

fn cached_quota_from(path: &Path, signature: FileSignature) -> Result<Option<EngineQuota>> {
    let cache = ROLLOUT_QUOTA_CACHE.get_or_init(Default::default);
    if let Some(cached) = cache
        .lock()
        .map_err(|_| anyhow!("Codex quota cache lock is poisoned"))?
        .get(path)
        .filter(|cached| cached.signature == signature)
        .cloned()
    {
        return Ok(cached.quota);
    }

    let quota = read_latest_quota_from_transcript(path)?;
    if file_signature(path)?.as_ref() == Some(&signature) {
        cache
            .lock()
            .map_err(|_| anyhow!("Codex quota cache lock is poisoned"))?
            .insert(
                path.to_path_buf(),
                CachedQuota {
                    signature,
                    quota: quota.clone(),
                },
            );
    }
    Ok(quota)
}

fn file_signature(path: &Path) -> Result<Option<FileSignature>> {
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("inspecting Codex transcript {}", path.display()));
        }
    };
    let modified = metadata
        .modified()
        .with_context(|| format!("reading modification time for {}", path.display()))?;
    Ok(Some(FileSignature {
        modified,
        length: metadata.len(),
    }))
}

fn read_latest_quota_from_transcript(path: &Path) -> Result<Option<EngineQuota>> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("opening Codex transcript {}", path.display()));
        }
    };
    let file_size = file
        .metadata()
        .with_context(|| format!("inspecting Codex transcript {}", path.display()))?
        .len();
    let file_size_as_usize = usize::try_from(file_size).unwrap_or(usize::MAX);
    let mut read_size = INITIAL_QUOTA_READ_SIZE.min(file_size_as_usize);
    while read_size > 0 {
        if let Some(quota) = read_latest_quota_window(&file, read_size, file_size)? {
            return Ok(Some(quota));
        }
        if read_size == file_size_as_usize || read_size == MAX_QUOTA_READ_SIZE {
            break;
        }
        read_size = (read_size.saturating_mul(2)).min(MAX_QUOTA_READ_SIZE);
    }

    read_latest_quota_forward(&file, path)
}

fn read_latest_quota_window(
    file: &File,
    read_size: usize,
    file_size: u64,
) -> Result<Option<EngineQuota>> {
    let file = file
        .try_clone()
        .context("cloning Codex transcript before seeking")?;
    let mut reader = BufReader::new(file);
    let read_size_from_end = i64::try_from(read_size).context("converting transcript read size")?;
    reader
        .seek(SeekFrom::End(-read_size_from_end))
        .context("seeking to the end of Codex transcript")?;
    let mut buffer = vec![0_u8; read_size];
    reader
        .read_exact(&mut buffer)
        .context("reading the end of Codex transcript")?;
    let content = String::from_utf8_lossy(&buffer);
    let complete_content = if u64::try_from(read_size).is_ok_and(|size| size >= file_size) {
        &content[..]
    } else if let Some(newline) = content.find('\n') {
        &content[newline + 1..]
    } else {
        return Ok(None);
    };

    Ok(complete_content.lines().rev().find_map(quota_from_line))
}

fn read_latest_quota_forward(file: &File, path: &Path) -> Result<Option<EngineQuota>> {
    let mut latest_quota = None;
    let file = file
        .try_clone()
        .context("cloning Codex transcript before scanning")?;
    let mut reader = BufReader::new(file);
    reader
        .seek(SeekFrom::Start(0))
        .with_context(|| format!("rewinding Codex transcript {}", path.display()))?;
    for line in reader.lines() {
        let line = line.with_context(|| format!("reading Codex transcript {}", path.display()))?;
        if let Some(quota) = quota_from_line(&line) {
            latest_quota = Some(quota);
        }
    }
    Ok(latest_quota)
}

fn quota_from_line(line: &str) -> Option<EngineQuota> {
    let record = serde_json::from_str::<Value>(line).ok()?;
    if record.pointer("/payload/type").and_then(Value::as_str) != Some("token_count") {
        return None;
    }
    let payload = record.get("payload");
    let info = payload.and_then(|payload| payload.get("info"));
    let rate_limits = codex_rate_limits(
        payload
            .and_then(|payload| payload.get("rate_limits"))
            .or_else(|| info.and_then(|info| info.get("rate_limits"))),
    );
    if !rate_limits.has_any() {
        return None;
    }
    Some(EngineQuota {
        engine: Engine::Codex,
        rate_limits,
        observed_at: record
            .get("timestamp")
            .and_then(Value::as_str)
            .and_then(parse_timestamp)
            .unwrap_or_else(Utc::now),
    })
}

fn rollout_paths_from(root: &Path) -> Result<Vec<PathBuf>> {
    let mut directories = vec![root.to_path_buf()];
    let mut paths = Vec::new();
    while let Some(directory) = directories.pop() {
        let entries = match fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(error).with_context(|| {
                    format!("reading Codex transcript directory {}", directory.display())
                });
            }
        };
        for entry in entries {
            let entry = entry.with_context(|| {
                format!(
                    "reading Codex transcript directory entry {}",
                    directory.display()
                )
            })?;
            let path = entry.path();
            let file_type = entry
                .file_type()
                .with_context(|| format!("inspecting Codex transcript path {}", path.display()))?;
            if file_type.is_dir() {
                directories.push(path);
            } else if file_type.is_file()
                && path
                    .extension()
                    .is_some_and(|extension| extension == "jsonl")
                && path
                    .file_name()
                    .is_some_and(|name| name.to_string_lossy().starts_with("rollout-"))
            {
                paths.push(path);
            }
        }
    }
    paths.sort();
    Ok(paths)
}

fn codex_context(usage: &Value, context_window: Option<&Value>) -> Option<ContextUsage> {
    let window_tokens = context_window?.as_u64()?;
    if window_tokens == 0 {
        return None;
    }
    let used_tokens = usage
        .get("total_tokens")
        .and_then(Value::as_u64)
        .unwrap_or_else(|| {
            [
                "input_tokens",
                "cached_input_tokens",
                "output_tokens",
                "reasoning_output_tokens",
            ]
            .into_iter()
            .filter_map(|field| usage.get(field).and_then(Value::as_u64))
            .fold(0_u64, u64::saturating_add)
        });

    Some(ContextUsage {
        used_tokens,
        window_tokens,
        used_percentage: ((used_tokens as f64 / window_tokens as f64) * 100.0).round() as u64,
    })
}

fn codex_rate_limits(limits: Option<&Value>) -> RateLimits {
    let Some(limits) = limits else {
        return RateLimits::default();
    };
    RateLimits {
        five_hour: codex_window(limits.get("primary"), 5 * 60 * 60),
        seven_day: codex_window(limits.get("secondary"), 7 * 24 * 60 * 60),
    }
}

fn codex_window(value: Option<&Value>, fallback_window_seconds: u64) -> Option<RateLimitWindow> {
    let value = value?;
    let used_percentage = value
        .get("used_percent")
        .or_else(|| value.get("used_percentage"))
        .and_then(Value::as_f64)?;
    let window_seconds = value
        .get("window_minutes")
        .and_then(Value::as_u64)
        .map(|minutes| minutes.saturating_mul(60))
        .or_else(|| {
            value
                .get("window_duration_ms")
                .and_then(Value::as_u64)
                .map(|milliseconds| milliseconds / 1000)
        })
        .or_else(|| value.get("window_duration_seconds").and_then(Value::as_u64))
        .unwrap_or(fallback_window_seconds);

    Some(RateLimitWindow {
        used_percentage,
        resets_at: value.get("resets_at").and_then(timestamp_value),
        window_seconds,
    })
}

fn timestamp_value(value: &Value) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| value.as_f64().map(|number| number as i64))
        .or_else(|| {
            value
                .as_str()
                .and_then(parse_timestamp)
                .map(|value| value.timestamp())
        })
}

fn parse_timestamp(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|timestamp| timestamp.with_timezone(&Utc))
}

#[cfg(test)]
mod tests {
    use std::fs::{self, File};
    use std::time::{Duration, SystemTime};

    use chrono::{TimeZone, Utc};
    use indoc::indoc;
    use tempfile::tempdir;

    use super::{read_latest_quota_from, read_transcript};
    use crate::commands::agent::types::Engine;
    use crate::commands::agent::usage::model::{
        ContextUsage, EngineQuota, RateLimitWindow, RateLimits, SessionUsage,
    };

    #[test]
    fn reads_latest_codex_usage_and_quota_from_rollout_records() {
        let directory = tempdir().expect("temporary directory should be created");
        let transcript = directory.path().join("fixture-rollout.jsonl");
        fs::write(
            &transcript,
            indoc! {r#"
                {"type":"session_meta","payload":{"cli_version":"0.98.0"}}
                {"type":"turn_context","payload":{"model":"fixture-model-amber","effort":"low"}}
                {"type":"event_msg","timestamp":"2026-11-16T11:00:00Z","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":80,"output_tokens":4,"total_tokens":84},"model_context_window":500},"rate_limits":{"primary":{"used_percent":8,"window_minutes":300,"resets_at":1794837600},"secondary":{"used_percent":5,"window_minutes":10080,"resets_at":1795431600}}}}
                {"type":"session_meta","payload":{"cli_version":"0.99.0"}}
                {"type":"turn_context","payload":{"model":"fixture-model-cobalt","effort":"high"}}
                {"type":"event_msg","timestamp":"2026-11-16T12:00:00Z","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":210,"cached_input_tokens":34,"output_tokens":13,"reasoning_output_tokens":8,"total_tokens":243},"total_token_usage":{"input_tokens":800,"output_tokens":55,"total_tokens":900},"model_context_window":1000},"rate_limits":{"primary":{"used_percent":22.5,"window_minutes":300,"resets_at":1794841200},"secondary":{"used_percent":17,"window_minutes":10080,"resets_at":1795435200}}}}
            "#},
        )
        .expect("fixture rollout should be written");

        assert_eq!(
            read_transcript(&transcript, "fixture-session"),
            Some(SessionUsage {
                session_id: "fixture-session".to_owned(),
                engine: Engine::Codex,
                model: Some("fixture-model-cobalt".to_owned()),
                effort: Some("high".to_owned()),
                context: Some(ContextUsage {
                    used_tokens: 243,
                    window_tokens: 1000,
                    used_percentage: 24,
                }),
                rate_limits: RateLimits {
                    five_hour: Some(RateLimitWindow {
                        used_percentage: 22.5,
                        resets_at: Some(1794841200),
                        window_seconds: 18000,
                    }),
                    seven_day: Some(RateLimitWindow {
                        used_percentage: 17.0,
                        resets_at: Some(1795435200),
                        window_seconds: 604800,
                    }),
                },
                version: Some("0.99.0".to_owned()),
                observed_at: Utc.with_ymd_and_hms(2026, 11, 16, 12, 0, 0).unwrap(),
            }),
        );
    }

    #[test]
    fn keeps_previous_quota_when_a_longer_rollout_tail_has_no_rate_limits() {
        let directory = tempdir().expect("temporary directory should be created");
        let codex_home = directory.path();
        let session_dir = codex_home.join("sessions/2026/11/16");
        fs::create_dir_all(&session_dir).expect("session directory should be created");
        let transcript = session_dir.join("rollout-fixture.jsonl");
        let quota_event = indoc! {r#"
            {"type":"event_msg","timestamp":"2026-11-16T12:00:00Z","payload":{"type":"token_count","info":{"last_token_usage":{"total_tokens":210},"model_context_window":1000},"rate_limits":{"primary":{"used_percent":22.5,"window_minutes":300,"resets_at":1794841200},"secondary":{"used_percent":17,"window_minutes":10080,"resets_at":1795435200}}}}
        "#};
        let no_quota_event = indoc! {r#"
            {"type":"event_msg","timestamp":"2026-11-16T13:00:00Z","payload":{"type":"token_count","info":{"last_token_usage":{"total_tokens":243},"model_context_window":1000}}}
        "#};
        let long_tail = no_quota_event.repeat(2048);
        fs::write(&transcript, format!("{quota_event}{long_tail}"))
            .expect("fixture rollout should be written");

        assert_eq!(
            read_transcript(&transcript, "fixture-session"),
            Some(SessionUsage {
                session_id: "fixture-session".to_owned(),
                engine: Engine::Codex,
                model: None,
                effort: None,
                context: Some(ContextUsage {
                    used_tokens: 243,
                    window_tokens: 1000,
                    used_percentage: 24,
                }),
                rate_limits: RateLimits {
                    five_hour: Some(RateLimitWindow {
                        used_percentage: 22.5,
                        resets_at: Some(1794841200),
                        window_seconds: 18000,
                    }),
                    seven_day: Some(RateLimitWindow {
                        used_percentage: 17.0,
                        resets_at: Some(1795435200),
                        window_seconds: 604800,
                    }),
                },
                version: None,
                observed_at: Utc.with_ymd_and_hms(2026, 11, 16, 13, 0, 0).unwrap(),
            }),
        );
        assert_eq!(
            read_latest_quota_from(codex_home).unwrap(),
            Some(EngineQuota {
                engine: Engine::Codex,
                rate_limits: RateLimits {
                    five_hour: Some(RateLimitWindow {
                        used_percentage: 22.5,
                        resets_at: Some(1794841200),
                        window_seconds: 18000,
                    }),
                    seven_day: Some(RateLimitWindow {
                        used_percentage: 17.0,
                        resets_at: Some(1795435200),
                        window_seconds: 604800,
                    }),
                },
                observed_at: Utc.with_ymd_and_hms(2026, 11, 16, 12, 0, 0).unwrap(),
            }),
        );
    }

    #[test]
    fn refreshes_cached_quota_when_a_rollout_changes() {
        let directory = tempdir().expect("temporary directory should be created");
        let transcript_dir = directory.path().join("sessions/2026/11/16");
        fs::create_dir_all(&transcript_dir).expect("session directory should be created");
        let transcript = transcript_dir.join("rollout-fixture.jsonl");
        fs::write(
            &transcript,
            indoc! {r#"
                {"type":"event_msg","timestamp":"2026-11-16T12:00:00Z","payload":{"type":"token_count","rate_limits":{"primary":{"used_percent":22,"window_minutes":300,"resets_at":1794841200}}}}
            "#},
        )
        .expect("initial rollout should be written");
        let initial = Some(EngineQuota {
            engine: Engine::Codex,
            rate_limits: RateLimits {
                five_hour: Some(RateLimitWindow {
                    used_percentage: 22.0,
                    resets_at: Some(1794841200),
                    window_seconds: 18000,
                }),
                seven_day: None,
            },
            observed_at: Utc.with_ymd_and_hms(2026, 11, 16, 12, 0, 0).unwrap(),
        });
        let updated = Some(EngineQuota {
            engine: Engine::Codex,
            rate_limits: RateLimits {
                five_hour: Some(RateLimitWindow {
                    used_percentage: 100.0,
                    resets_at: Some(1794844800),
                    window_seconds: 18000,
                }),
                seven_day: None,
            },
            observed_at: Utc.with_ymd_and_hms(2026, 11, 16, 13, 0, 0).unwrap(),
        });
        let first_read = read_latest_quota_from(directory.path()).unwrap();
        fs::write(
            &transcript,
            indoc! {r#"
                {"type":"event_msg","timestamp":"2026-11-16T13:00:00Z","payload":{"type":"token_count","rate_limits":{"primary":{"used_percent":100,"window_minutes":300,"resets_at":1794844800}}}}
            "#},
        )
        .expect("updated rollout should be written");

        assert_eq!(
            [
                first_read,
                read_latest_quota_from(directory.path()).unwrap()
            ],
            [initial, updated],
        );
    }

    #[test]
    fn stops_at_the_first_quota_in_modification_time_order() {
        let directory = tempdir().expect("temporary directory should be created");
        let archived_path = directory
            .path()
            .join("archived_sessions/rollout-older.jsonl");
        let session_path = directory
            .path()
            .join("sessions/2026/11/17/rollout-quota.jsonl");
        let newest_path = directory
            .path()
            .join("sessions/2026/11/17/rollout-newest.jsonl");
        for path in [&archived_path, &session_path, &newest_path] {
            fs::create_dir_all(
                path.parent()
                    .expect("fixture rollout should have a parent directory"),
            )
            .expect("fixture rollout directory should be created");
        }
        fs::write(
            &archived_path,
            indoc! {r#"
                {"type":"event_msg","timestamp":"2026-11-17T13:00:00Z","payload":{"type":"token_count","rate_limits":{"primary":{"used_percent":12,"window_minutes":300,"resets_at":1794841200}}}}
            "#},
        )
        .expect("older archived rollout should be written");
        fs::write(
            &session_path,
            indoc! {r#"
                {"type":"event_msg","timestamp":"2026-11-17T12:00:00Z","payload":{"type":"token_count","rate_limits":{"primary":{"used_percent":72,"window_minutes":300,"resets_at":1794841200}}}}
            "#},
        )
        .expect("quota rollout should be written");
        fs::write(
            &newest_path,
            indoc! {r#"
                {"type":"event_msg","timestamp":"2026-11-17T14:00:00Z","payload":{"type":"token_count","info":{"last_token_usage":{"total_tokens":243},"model_context_window":1000}}}
            "#},
        )
        .expect("newest rollout without quota should be written");
        let base_time = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        File::open(&archived_path)
            .expect("older archived rollout should open")
            .set_modified(base_time)
            .expect("older archived rollout mtime should be set");
        File::open(&session_path)
            .expect("quota rollout should open")
            .set_modified(base_time + Duration::from_secs(10))
            .expect("quota rollout mtime should be set");
        File::open(&newest_path)
            .expect("newest rollout should open")
            .set_modified(base_time + Duration::from_secs(20))
            .expect("newest rollout mtime should be set");

        assert_eq!(
            read_latest_quota_from(directory.path()).unwrap(),
            Some(EngineQuota {
                engine: Engine::Codex,
                rate_limits: RateLimits {
                    five_hour: Some(RateLimitWindow {
                        used_percentage: 72.0,
                        resets_at: Some(1794841200),
                        window_seconds: 18000,
                    }),
                    seven_day: None,
                },
                observed_at: Utc.with_ymd_and_hms(2026, 11, 17, 12, 0, 0).unwrap(),
            }),
        );
    }
}
