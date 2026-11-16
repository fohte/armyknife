use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::{TransactionBehavior, params};

use super::database;
use crate::commands::agent::types::Engine;
use crate::commands::agent::usage::model::{EngineQuota, SessionUsage};

pub(crate) fn record_usage_from(sessions_dir: &Path, usage: &SessionUsage) -> Result<()> {
    let mut connection = database::open_usage_connection(sessions_dir)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let usage_json = serde_json::to_string(usage).context("serializing session usage")?;
    let observed_at = usage.observed_at.to_rfc3339();
    transaction.execute(
        "INSERT INTO agent_session_usage (session_id, engine, data, observed_at)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(session_id) DO UPDATE SET
             engine = excluded.engine,
             data = excluded.data,
             observed_at = excluded.observed_at",
        params![
            usage.session_id,
            engine_name(usage.engine),
            usage_json,
            observed_at
        ],
    )?;

    if has_rate_limits(&usage.rate_limits) {
        let quota = EngineQuota {
            engine: usage.engine,
            rate_limits: usage.rate_limits.clone(),
            observed_at: usage.observed_at,
        };
        let quota_json = serde_json::to_string(&quota).context("serializing engine quota")?;
        transaction.execute(
            "INSERT INTO agent_engine_quota (engine, data, observed_at)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(engine) DO UPDATE SET
                 data = excluded.data,
                 observed_at = excluded.observed_at",
            params![engine_name(usage.engine), quota_json, observed_at],
        )?;
    }

    transaction.commit()?;
    Ok(())
}

pub(crate) fn usage_snapshots_from(sessions_dir: &Path) -> Result<Vec<SessionUsage>> {
    let connection = database::open_usage_connection(sessions_dir)?;
    let mut statement =
        connection.prepare("SELECT data FROM agent_session_usage ORDER BY session_id")?;
    let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
    rows.map(|row| serde_json::from_str(&row?).context("deserializing session usage"))
        .collect()
}

pub(crate) fn engine_quotas_from(sessions_dir: &Path) -> Result<Vec<EngineQuota>> {
    let connection = database::open_usage_connection(sessions_dir)?;
    let mut statement =
        connection.prepare("SELECT data FROM agent_engine_quota ORDER BY engine")?;
    let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
    rows.map(|row| serde_json::from_str(&row?).context("deserializing engine quota"))
        .collect()
}

fn has_rate_limits(rate_limits: &crate::commands::agent::usage::model::RateLimits) -> bool {
    rate_limits.five_hour.is_some() || rate_limits.seven_day.is_some()
}

fn engine_name(engine: Engine) -> &'static str {
    match engine {
        Engine::Claude => "claude",
        Engine::Codex => "codex",
    }
}

#[cfg(test)]
mod tests {
    use super::{engine_quotas_from, record_usage_from, usage_snapshots_from};
    use crate::commands::agent::types::Engine;
    use crate::commands::agent::usage::model::{
        ContextUsage, RateLimitWindow, RateLimits, SessionUsage,
    };
    use chrono::{TimeZone, Utc};

    fn usage() -> SessionUsage {
        SessionUsage {
            session_id: "fixture-session".to_owned(),
            engine: Engine::Claude,
            model: Some("fixture-model-amber".to_owned()),
            effort: Some("medium".to_owned()),
            context: Some(ContextUsage {
                used_tokens: 120,
                window_tokens: 200_000,
                used_percentage: 1,
            }),
            rate_limits: RateLimits {
                five_hour: Some(RateLimitWindow {
                    used_percentage: 27.0,
                    resets_at: Some(1_800_000_000),
                    window_seconds: 18_000,
                }),
                seven_day: None,
            },
            version: Some("0.99.0".to_owned()),
            observed_at: Utc.with_ymd_and_hms(2026, 11, 16, 12, 0, 0).unwrap(),
        }
    }

    #[test]
    fn records_and_reads_session_usage_and_latest_engine_quota() {
        let sessions_dir = tempfile::tempdir().expect("temporary sessions directory should exist");
        record_usage_from(sessions_dir.path(), &usage()).expect("usage should be recorded");

        assert_eq!(
            (
                usage_snapshots_from(sessions_dir.path()).expect("session usage should load"),
                engine_quotas_from(sessions_dir.path()).expect("engine quota should load"),
            ),
            (
                vec![usage()],
                vec![crate::commands::agent::usage::model::EngineQuota {
                    engine: Engine::Claude,
                    rate_limits: usage().rate_limits,
                    observed_at: usage().observed_at,
                }],
            ),
        );
    }

    #[test]
    fn keeps_the_previous_engine_quota_when_an_update_has_no_quota_data() {
        let sessions_dir = tempfile::tempdir().expect("temporary sessions directory should exist");
        record_usage_from(sessions_dir.path(), &usage()).expect("initial usage should be recorded");
        let mut update = usage();
        update.model = Some("fixture-model-cobalt".to_owned());
        update.rate_limits = RateLimits::default();
        record_usage_from(sessions_dir.path(), &update).expect("latest usage should be recorded");

        assert_eq!(
            (
                usage_snapshots_from(sessions_dir.path()).expect("session usage should load"),
                engine_quotas_from(sessions_dir.path()).expect("engine quota should load"),
            ),
            (
                vec![update],
                vec![crate::commands::agent::usage::model::EngineQuota {
                    engine: Engine::Claude,
                    rate_limits: usage().rate_limits,
                    observed_at: usage().observed_at,
                }],
            ),
        );
    }
}
