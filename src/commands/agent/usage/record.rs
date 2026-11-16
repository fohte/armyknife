use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use clap::{Args, Subcommand};
use serde::Deserialize;

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

#[derive(Debug, Deserialize)]
struct ClaudeStatusLine {
    session_id: Option<String>,
    model: Option<ModelData>,
    effort: Option<EffortData>,
    context_window: Option<ContextWindowData>,
    rate_limits: Option<ClaudeRateLimits>,
    version: Option<String>,
    transcript_path: Option<PathBuf>,
}

#[derive(Debug, Deserialize)]
struct ModelData {
    display_name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct EffortData {
    level: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ContextWindowData {
    context_window_size: Option<u64>,
}

#[derive(Debug, Deserialize, Default)]
struct ClaudeRateLimits {
    five_hour: Option<ClaudeRateLimitWindow>,
    seven_day: Option<ClaudeRateLimitWindow>,
}

#[derive(Debug, Deserialize)]
struct ClaudeRateLimitWindow {
    used_percentage: Option<f64>,
    resets_at: Option<i64>,
}

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
        tracing::warn!(%error, "could not record Claude Code usage");
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
    let data: ClaudeStatusLine =
        serde_json::from_str(raw).context("parsing Claude Code status-line JSON")?;
    let Some(session_id) = data
        .session_id
        .map(|session_id| session_id.trim().to_owned())
        .filter(|session_id| !session_id.is_empty())
    else {
        return Ok(None);
    };

    let window_tokens = data
        .context_window
        .and_then(|window| window.context_window_size)
        .unwrap_or(200_000);
    let context = data.transcript_path.as_deref().and_then(|path| {
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

    Ok(Some(SessionUsage {
        session_id,
        engine: Engine::Claude,
        model: data.model.and_then(|model| model.display_name),
        effort: data.effort.and_then(|effort| effort.level),
        context,
        rate_limits: claude_rate_limits(data.rate_limits.unwrap_or_default()),
        version: data.version,
        observed_at,
    }))
}

fn claude_rate_limits(limits: ClaudeRateLimits) -> RateLimits {
    RateLimits {
        five_hour: limits.five_hour.and_then(|window| {
            Some(RateLimitWindow {
                used_percentage: window.used_percentage?,
                resets_at: window.resets_at,
                window_seconds: 5 * 60 * 60,
            })
        }),
        seven_day: limits.seven_day.and_then(|window| {
            Some(RateLimitWindow {
                used_percentage: window.used_percentage?,
                resets_at: window.resets_at,
                window_seconds: 7 * 24 * 60 * 60,
            })
        }),
    }
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
