use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::{OptionalExtension, TransactionBehavior, params};

use super::database;
use crate::commands::agent::types::Engine;
use crate::commands::agent::usage::model::{
    EngineQuota, RateLimitWindow, RateLimits, SessionUsage,
};

pub(crate) fn record_usage_from(sessions_dir: &Path, usage: &SessionUsage) -> Result<()> {
    let mut connection = database::open_usage_connection(sessions_dir)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let incoming_rate_limits = usage.rate_limits.clone();
    let has_rate_limit_update = incoming_rate_limits.has_any();
    let previous_usage = transaction
        .query_row(
            "SELECT data FROM agent_session_usage WHERE session_id = ?1",
            params![usage.session_id],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .map(|data| serde_json::from_str::<SessionUsage>(&data))
        .transpose()
        .context("deserializing previous session usage")?;
    let mut usage = usage.clone();
    if let Some(previous_usage) = previous_usage {
        merge_usage(&mut usage, previous_usage);
    }
    let usage_json = serde_json::to_string(&usage).context("serializing session usage")?;
    let observed_at = usage.observed_at.timestamp_micros();
    transaction.execute(
        "INSERT INTO agent_session_usage (session_id, engine, data, observed_at)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(session_id) DO UPDATE SET
             engine = excluded.engine,
             data = excluded.data,
             observed_at = excluded.observed_at
         WHERE excluded.observed_at > agent_session_usage.observed_at",
        params![
            usage.session_id,
            engine_name(usage.engine),
            usage_json,
            observed_at
        ],
    )?;

    if has_rate_limit_update {
        let previous_quota = transaction
            .query_row(
                "SELECT data FROM agent_engine_quota WHERE engine = ?1",
                params![engine_name(usage.engine)],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .map(|data| serde_json::from_str::<EngineQuota>(&data))
            .transpose()
            .context("deserializing previous engine quota")?;
        let rate_limits = previous_quota
            .map(|quota| merge_rate_limits(incoming_rate_limits.clone(), quota.rate_limits))
            .unwrap_or(incoming_rate_limits);
        let quota = EngineQuota {
            engine: usage.engine,
            rate_limits,
            observed_at: usage.observed_at,
        };
        let quota_json = serde_json::to_string(&quota).context("serializing engine quota")?;
        transaction.execute(
            "INSERT INTO agent_engine_quota (engine, data, observed_at)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(engine) DO UPDATE SET
                 data = excluded.data,
                 observed_at = excluded.observed_at
             WHERE excluded.observed_at > agent_engine_quota.observed_at",
            params![engine_name(usage.engine), quota_json, observed_at],
        )?;
    }

    transaction.commit()?;
    Ok(())
}

fn merge_usage(current: &mut SessionUsage, previous: SessionUsage) {
    current.model = current.model.take().or(previous.model);
    current.effort = current.effort.take().or(previous.effort);
    current.context = current.context.take().or(previous.context);
    current.version = current.version.take().or(previous.version);
    current.rate_limits = merge_rate_limits(current.rate_limits.clone(), previous.rate_limits);
}

fn merge_rate_limits(current: RateLimits, previous: RateLimits) -> RateLimits {
    fn merge_window(
        current: Option<RateLimitWindow>,
        previous: Option<RateLimitWindow>,
    ) -> Option<RateLimitWindow> {
        match (current, previous) {
            (Some(mut current), Some(previous)) => {
                current.resets_at = current.resets_at.or(previous.resets_at);
                Some(current)
            }
            (Some(current), None) => Some(current),
            (None, Some(previous)) => Some(previous),
            (None, None) => None,
        }
    }

    RateLimits {
        five_hour: merge_window(current.five_hour, previous.five_hour),
        seven_day: merge_window(current.seven_day, previous.seven_day),
    }
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
    use rstest::{fixture, rstest};
    use tempfile::TempDir;

    #[fixture]
    fn sessions_dir() -> TempDir {
        tempfile::tempdir().expect("temporary sessions directory should exist")
    }

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

    #[rstest]
    fn records_and_reads_session_usage_and_latest_engine_quota(sessions_dir: TempDir) {
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

    #[rstest]
    fn keeps_the_previous_engine_quota_when_an_update_has_no_quota_data(sessions_dir: TempDir) {
        record_usage_from(sessions_dir.path(), &usage()).expect("initial usage should be recorded");
        let mut update = usage();
        update.model = Some("fixture-model-cobalt".to_owned());
        update.rate_limits = RateLimits::default();
        update.observed_at += chrono::Duration::seconds(1);
        record_usage_from(sessions_dir.path(), &update).expect("latest usage should be recorded");
        let mut expected_update = update;
        expected_update.rate_limits = usage().rate_limits;

        assert_eq!(
            (
                usage_snapshots_from(sessions_dir.path()).expect("session usage should load"),
                engine_quotas_from(sessions_dir.path()).expect("engine quota should load"),
            ),
            (
                vec![expected_update],
                vec![crate::commands::agent::usage::model::EngineQuota {
                    engine: Engine::Claude,
                    rate_limits: usage().rate_limits,
                    observed_at: usage().observed_at,
                }],
            ),
        );
    }

    #[rstest]
    fn ignores_an_older_session_usage_and_quota_update(sessions_dir: TempDir) {
        record_usage_from(sessions_dir.path(), &usage()).expect("initial usage should be recorded");
        let mut stale_update = usage();
        stale_update.model = Some("fixture-model-cobalt".to_owned());
        stale_update.rate_limits.five_hour = Some(RateLimitWindow {
            used_percentage: 84.0,
            resets_at: Some(1_800_000_000),
            window_seconds: 18_000,
        });
        stale_update.observed_at -= chrono::Duration::seconds(1);
        record_usage_from(sessions_dir.path(), &stale_update)
            .expect("stale usage should not fail the record command");

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
}
