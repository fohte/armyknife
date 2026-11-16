use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde_json::Value;

use super::model::{ContextUsage, EngineQuota, RateLimitWindow, RateLimits, SessionUsage};
use crate::commands::agent::types::Engine;

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
                rate_limits = codex_rate_limits(
                    payload
                        .and_then(|payload| payload.get("rate_limits"))
                        .or_else(|| info.and_then(|info| info.get("rate_limits"))),
                );
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
    let mut latest_quota: Option<EngineQuota> = None;
    for root in [
        codex_home.join("sessions"),
        codex_home.join("archived_sessions"),
    ] {
        for path in rollout_paths_from(&root)? {
            let Some(usage) = read_transcript(&path, "") else {
                continue;
            };
            if !usage.rate_limits.has_any() {
                continue;
            }
            let quota = EngineQuota {
                engine: Engine::Codex,
                rate_limits: usage.rate_limits,
                observed_at: usage.observed_at,
            };
            if latest_quota
                .as_ref()
                .is_none_or(|latest| quota.observed_at > latest.observed_at)
            {
                latest_quota = Some(quota);
            }
        }
    }
    Ok(latest_quota)
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
    use std::fs;

    use chrono::{TimeZone, Utc};
    use indoc::indoc;
    use tempfile::tempdir;

    use super::read_transcript;
    use crate::commands::agent::types::Engine;
    use crate::commands::agent::usage::model::{
        ContextUsage, RateLimitWindow, RateLimits, SessionUsage,
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
}
