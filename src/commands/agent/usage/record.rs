use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use clap::{Args, Subcommand};
use serde::de::DeserializeOwned;
use serde_json::{Map, Number, Value};

use super::model::{ContextUsage, RateLimitWindow, RateLimits, SessionUsage};
use super::transcript::read_total_tokens;
use crate::commands::agent::store;
use crate::commands::agent::types::Engine;

#[derive(Subcommand, Clone, PartialEq, Eq)]
pub enum UsageCommands {
    /// Record the Claude Code status-line JSON from stdin
    Record(RecordArgs),
}

#[derive(Args, Clone, PartialEq, Eq)]
pub struct RecordArgs {}

pub fn run(command: &UsageCommands) -> Result<()> {
    match command {
        UsageCommands::Record(_) => {
            let stdin = std::io::stdin();
            record_status_line_best_effort(store::sessions_dir(), stdin.lock());
            Ok(())
        }
    }
}

fn record_status_line_best_effort(sessions_dir: Result<PathBuf>, input: impl Read) {
    let result = sessions_dir.and_then(|sessions_dir| {
        record_status_line_from(&sessions_dir, input, Utc::now).map(|_| ())
    });
    if let Err(error) = result {
        tracing::warn!(session_id = "<unavailable>", error = %format!("{error:#}"), "could not record Claude Code usage");
    }
}

fn record_status_line_from(
    sessions_dir: &Path,
    input: impl Read,
    observed_at: impl FnOnce() -> DateTime<Utc>,
) -> Result<Option<SessionUsage>> {
    let mut raw = String::new();
    let mut input = input;
    input
        .read_to_string(&mut raw)
        .context("reading Claude Code status-line JSON from stdin")?;
    let observed_at = observed_at();
    let Some(usage) = parse_claude_status_line(&raw, observed_at)? else {
        return Ok(None);
    };
    store::record_usage_from(sessions_dir, &usage)?;
    Ok(Some(usage))
}

pub(crate) fn parse_claude_status_line(
    raw: &str,
    observed_at: DateTime<Utc>,
) -> Result<Option<SessionUsage>> {
    let data: Value = serde_json::from_str(raw).context("parsing Claude Code status-line JSON")?;
    let Some(data) = data.as_object() else {
        if let Err(error) = serde_json::from_value::<Map<String, Value>>(data.clone()) {
            tracing::warn!(session_id = "<unavailable>", field = "<root>", serde_error = %error, "could not parse Claude Code status-line JSON");
        }
        return Ok(None);
    };
    let session_id: Option<String> =
        deserialize_field(data.get("session_id"), "session_id", "<unavailable>");
    let Some(session_id) = session_id
        .map(|session_id| session_id.trim().to_owned())
        .filter(|session_id| !session_id.is_empty())
    else {
        return Ok(None);
    };

    let window_tokens = object_field(data.get("context_window"), "context_window", &session_id)
        .and_then(|window| {
            integer_field(
                window.get("context_window_size"),
                "context_window.context_window_size",
                &session_id,
            )
        })
        .unwrap_or(200_000);
    let transcript_path: Option<PathBuf> =
        deserialize_field(data.get("transcript_path"), "transcript_path", &session_id);
    let context = transcript_path.as_deref().and_then(|path| {
        let used_tokens = match read_total_tokens(path) {
            Ok(used_tokens) => used_tokens,
            Err(error) => {
                tracing::warn!(path = %path.display(), %error, "could not read Claude transcript usage");
                return None;
            }
        };
        let auto_compact_limit = window_tokens as f64 * 0.8;
        let used_percentage = if auto_compact_limit <= 0.0 {
            0
        } else {
            ((used_tokens as f64 / auto_compact_limit) * 100.0).round() as u64
        };
        Some(ContextUsage {
            used_tokens,
            window_tokens,
            used_percentage,
        })
    });

    let model = object_field(data.get("model"), "model", &session_id).and_then(|model| {
        deserialize_field(model.get("display_name"), "model.display_name", &session_id)
    });
    let effort = object_field(data.get("effort"), "effort", &session_id)
        .and_then(|effort| deserialize_field(effort.get("level"), "effort.level", &session_id));
    let rate_limits = claude_rate_limits(data.get("rate_limits"), &session_id);
    let version = deserialize_field(data.get("version"), "version", &session_id);

    Ok(Some(SessionUsage {
        session_id,
        engine: Engine::Claude,
        model,
        effort,
        context,
        rate_limits,
        version,
        observed_at,
    }))
}

fn deserialize_field<T: DeserializeOwned>(
    value: Option<&Value>,
    field: &str,
    session_id: &str,
) -> Option<T> {
    let value = value?;
    if value.is_null() {
        return None;
    }
    match serde_json::from_value(value.clone()) {
        Ok(value) => Some(value),
        Err(error) => {
            tracing::warn!(session_id, field, serde_error = %error, "ignoring invalid Claude status-line field");
            None
        }
    }
}

fn object_field<'a>(
    value: Option<&'a Value>,
    field: &str,
    session_id: &str,
) -> Option<&'a Map<String, Value>> {
    let value = value?;
    if value.is_null() {
        return None;
    }
    match value.as_object() {
        Some(object) => Some(object),
        None => {
            if let Err(error) = serde_json::from_value::<Map<String, Value>>(value.clone()) {
                tracing::warn!(session_id, field, serde_error = %error, "ignoring invalid Claude status-line field");
            }
            None
        }
    }
}

trait IntegerValue: Sized {
    fn from_signed(value: i64) -> Option<Self>;
    fn from_unsigned(value: u64) -> Option<Self>;
    fn from_float(value: f64) -> Option<Self>;
}

impl IntegerValue for u64 {
    fn from_signed(value: i64) -> Option<Self> {
        u64::try_from(value).ok()
    }

    fn from_unsigned(value: u64) -> Option<Self> {
        Some(value)
    }

    fn from_float(value: f64) -> Option<Self> {
        (value >= 0.0 && value < u64::MAX as f64).then(|| value.trunc() as u64)
    }
}

impl IntegerValue for i64 {
    fn from_signed(value: i64) -> Option<Self> {
        Some(value)
    }

    fn from_unsigned(value: u64) -> Option<Self> {
        i64::try_from(value).ok()
    }

    fn from_float(value: f64) -> Option<Self> {
        (value >= i64::MIN as f64 && value < i64::MAX as f64).then(|| value.trunc() as i64)
    }
}

fn integer_field<T: IntegerValue>(
    value: Option<&Value>,
    field: &str,
    session_id: &str,
) -> Option<T> {
    let value = value?;
    if value.is_null() {
        return None;
    }
    let Value::Number(number) = value else {
        if let Err(error) = serde_json::from_value::<i64>(value.clone()) {
            tracing::warn!(session_id, field, serde_error = %error, "ignoring invalid Claude status-line field");
        }
        return None;
    };
    let parsed = integer_from_number(number);
    if parsed.is_none() {
        tracing::warn!(
            session_id,
            field,
            serde_error = "number is outside the supported integer range",
            "ignoring invalid Claude status-line field"
        );
    }
    parsed
}

fn integer_from_number<T: IntegerValue>(number: &Number) -> Option<T> {
    number
        .as_i64()
        .and_then(T::from_signed)
        .or_else(|| number.as_u64().and_then(T::from_unsigned))
        .or_else(|| number.as_f64().and_then(T::from_float))
}

fn claude_rate_limits(limits: Option<&Value>, session_id: &str) -> RateLimits {
    let Some(limits) = object_field(limits, "rate_limits", session_id) else {
        return RateLimits::default();
    };
    RateLimits {
        five_hour: rate_limit_window(
            limits.get("five_hour"),
            "rate_limits.five_hour",
            session_id,
            5 * 60 * 60,
        ),
        seven_day: rate_limit_window(
            limits.get("seven_day"),
            "rate_limits.seven_day",
            session_id,
            7 * 24 * 60 * 60,
        ),
    }
}

fn rate_limit_window(
    value: Option<&Value>,
    field: &str,
    session_id: &str,
    window_seconds: u64,
) -> Option<RateLimitWindow> {
    object_field(value, field, session_id).and_then(|window| {
        let used_percentage = deserialize_field(
            window.get("used_percentage"),
            &format!("{field}.used_percentage"),
            session_id,
        )?;
        Some(RateLimitWindow {
            used_percentage,
            resets_at: integer_field(
                window.get("resets_at"),
                &format!("{field}.resets_at"),
                session_id,
            ),
            window_seconds,
        })
    })
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io::Cursor;

    use chrono::{TimeZone, Utc};
    use indoc::indoc;
    use tempfile::tempdir;

    use super::{
        parse_claude_status_line, record_status_line_best_effort, record_status_line_from,
    };
    use crate::commands::agent::store::{engine_quotas_from, usage_snapshots_from};
    use crate::commands::agent::types::Engine;
    use crate::commands::agent::usage::model::{
        ContextUsage, RateLimitWindow, RateLimits, SessionUsage,
    };

    #[test]
    fn records_usage_from_status_line_json_and_transcript() {
        let sessions_dir = tempdir().expect("temporary sessions directory should exist");
        let transcript_dir = tempdir().expect("temporary transcript directory should exist");
        let transcript = transcript_dir.path().join("fixture-transcript.jsonl");
        fs::write(
            &transcript,
            indoc! {r#"
                {"type":"assistant","message":{"usage":{"input_tokens":180,"output_tokens":20,"cache_creation_input_tokens":15,"cache_read_input_tokens":5}}}
            "#},
        )
        .expect("fixture transcript should be written");
        let input = format!(
            r#"{{"session_id":"fixture-session","model":{{"display_name":"fixture-model-amber"}},"effort":{{"level":"high"}},"context_window":{{"context_window_size":1000}},"rate_limits":{{"five_hour":{{"used_percentage":32.5,"resets_at":1794841200}},"seven_day":{{"used_percentage":18,"resets_at":1795435200}}}},"version":"0.99.0","transcript_path":"{}"}}"#,
            transcript.display()
        );
        let observed_at = Utc.with_ymd_and_hms(2026, 11, 16, 12, 0, 0).unwrap();

        record_status_line_from(sessions_dir.path(), Cursor::new(input), || observed_at)
            .expect("status-line usage should be recorded");

        assert_eq!(
            (
                usage_snapshots_from(sessions_dir.path()).expect("session usage should load"),
                engine_quotas_from(sessions_dir.path()).expect("engine quota should load"),
            ),
            (
                vec![SessionUsage {
                    session_id: "fixture-session".to_owned(),
                    engine: Engine::Claude,
                    model: Some("fixture-model-amber".to_owned()),
                    effort: Some("high".to_owned()),
                    context: Some(ContextUsage {
                        used_tokens: 220,
                        window_tokens: 1000,
                        used_percentage: 28,
                    }),
                    rate_limits: RateLimits {
                        five_hour: Some(RateLimitWindow {
                            used_percentage: 32.5,
                            resets_at: Some(1794841200),
                            window_seconds: 18_000,
                        }),
                        seven_day: Some(RateLimitWindow {
                            used_percentage: 18.0,
                            resets_at: Some(1795435200),
                            window_seconds: 604_800,
                        }),
                    },
                    version: Some("0.99.0".to_owned()),
                    observed_at,
                }],
                vec![crate::commands::agent::usage::model::EngineQuota {
                    engine: Engine::Claude,
                    rate_limits: RateLimits {
                        five_hour: Some(RateLimitWindow {
                            used_percentage: 32.5,
                            resets_at: Some(1794841200),
                            window_seconds: 18_000,
                        }),
                        seven_day: Some(RateLimitWindow {
                            used_percentage: 18.0,
                            resets_at: Some(1795435200),
                            window_seconds: 604_800,
                        }),
                    },
                    observed_at,
                }],
            ),
        );
    }

    #[test]
    fn records_metadata_without_context_when_transcript_is_unreadable() {
        let directory = tempdir().expect("temporary directory should exist");
        let missing_transcript = directory.path().join("missing-fixture-transcript.jsonl");
        let observed_at = Utc.with_ymd_and_hms(2026, 11, 16, 12, 0, 0).unwrap();
        let raw = format!(
            r#"{{"session_id":"fixture-session","model":{{"display_name":"fixture-model-amber"}},"transcript_path":"{}"}}"#,
            missing_transcript.display()
        );

        assert_eq!(
            parse_claude_status_line(&raw, observed_at)
                .expect("status-line metadata should be parsed"),
            Some(SessionUsage {
                session_id: "fixture-session".to_owned(),
                engine: Engine::Claude,
                model: Some("fixture-model-amber".to_owned()),
                effort: None,
                context: None,
                rate_limits: RateLimits::default(),
                version: None,
                observed_at,
            }),
        );
    }

    #[test]
    fn records_valid_status_line_fields_when_another_field_has_an_unexpected_type() {
        use crate::shared::testing::capture_json_events;

        let sessions_dir = tempdir().expect("temporary sessions directory should exist");
        let transcript_dir = tempdir().expect("temporary transcript directory should exist");
        let transcript = transcript_dir.path().join("sample-transcript.jsonl");
        fs::write(
            &transcript,
            indoc! {r#"
                {"type":"assistant","message":{"usage":{"input_tokens":160,"output_tokens":40}}}
            "#},
        )
        .expect("fixture transcript should be written");
        let raw = serde_json::json!({
            "session_id": "sample-session-copper",
            "model": {"display_name": "sample-model-copper"},
            "effort": {"level": 3},
            "context_window": {"context_window_size": 1000.75},
            "rate_limits": {
                "five_hour": {"used_percentage": 32, "resets_at": 1794841200.75},
                "seven_day": {"used_percentage": 18.5, "resets_at": 1795435200}
            },
            "version": "0.99.0",
            "transcript_path": transcript,
        })
        .to_string();
        let observed_at = Utc.with_ymd_and_hms(2026, 11, 16, 12, 0, 0).unwrap();

        let (recorded, warnings) = capture_json_events(tracing::Level::WARN, || {
            record_status_line_from(sessions_dir.path(), Cursor::new(raw), || observed_at)
        });
        let recorded = recorded
            .expect("valid status-line fields should be recorded")
            .expect("session id should produce a usage record");
        let serde_error = serde_json::from_value::<String>(serde_json::json!(3))
            .expect_err("numeric effort levels should fail string deserialization")
            .to_string();

        assert_eq!(
            (recorded, warnings),
            (
                SessionUsage {
                    session_id: "sample-session-copper".to_owned(),
                    engine: Engine::Claude,
                    model: Some("sample-model-copper".to_owned()),
                    effort: None,
                    context: Some(ContextUsage {
                        used_tokens: 200,
                        window_tokens: 1000,
                        used_percentage: 25,
                    }),
                    rate_limits: RateLimits {
                        five_hour: Some(RateLimitWindow {
                            used_percentage: 32.0,
                            resets_at: Some(1794841200),
                            window_seconds: 18_000,
                        }),
                        seven_day: Some(RateLimitWindow {
                            used_percentage: 18.5,
                            resets_at: Some(1795435200),
                            window_seconds: 604_800,
                        }),
                    },
                    version: Some("0.99.0".to_owned()),
                    observed_at,
                },
                vec![serde_json::json!({
                    "session_id": "sample-session-copper",
                    "field": "effort.level",
                    "serde_error": serde_error,
                    "message": "ignoring invalid Claude status-line field",
                })],
            ),
        );
    }

    #[test]
    fn best_effort_recording_succeeds_when_status_line_json_is_invalid() {
        let sessions_dir = tempdir().expect("temporary sessions directory should exist");

        record_status_line_best_effort(
            Ok(sessions_dir.path().to_path_buf()),
            Cursor::new("not json"),
        );

        assert_eq!(
            usage_snapshots_from(sessions_dir.path()).expect("session usage should load"),
            Vec::<SessionUsage>::new(),
        );
    }
}
