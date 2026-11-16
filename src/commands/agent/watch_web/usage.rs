use std::collections::HashMap;
use std::future::Future;

use axum::Json;
use axum::Router;
use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::get;

use crate::commands::agent::store;
use crate::commands::agent::types::{Engine, Session};
use crate::commands::agent::usage::codex;
use crate::commands::agent::usage::model::{EngineQuota, EngineQuotas, UsageResponse};

trait UsageSource: Clone + Send + Sync + 'static {
    fn load(&self) -> impl Future<Output = anyhow::Result<UsageResponse>> + Send;
}

#[derive(Clone, Copy)]
struct LiveUsageSource;

impl UsageSource for LiveUsageSource {
    async fn load(&self) -> anyhow::Result<UsageResponse> {
        tokio::task::spawn_blocking(load_usage_response).await?
    }
}

pub(super) fn router() -> Router {
    router_with_source(LiveUsageSource)
}

fn router_with_source<S: UsageSource>(source: S) -> Router {
    Router::new()
        .route("/usage", get(get_usage::<S>))
        .with_state(source)
}

async fn get_usage<S: UsageSource>(
    State(source): State<S>,
) -> Result<Json<UsageResponse>, StatusCode> {
    source.load().await.map(Json).map_err(|error| {
        tracing::warn!(%error, "failed to load agent usage");
        StatusCode::INTERNAL_SERVER_ERROR
    })
}

fn load_usage_response() -> anyhow::Result<UsageResponse> {
    let sessions_dir = store::sessions_dir()?;
    let sessions = store::list_all_sessions_from(&sessions_dir)?;
    let stored_usage = store::usage_snapshots_from(&sessions_dir)?;
    let stored_quotas = store::engine_quotas_from(&sessions_dir)?;
    Ok(usage_response(sessions, stored_usage, stored_quotas))
}

fn usage_response(
    sessions: Vec<Session>,
    stored_usage: Vec<crate::commands::agent::usage::model::SessionUsage>,
    stored_quotas: Vec<EngineQuota>,
) -> UsageResponse {
    let mut usage_by_session = stored_usage
        .into_iter()
        .map(|usage| (usage.session_id.clone(), usage))
        .collect::<HashMap<_, _>>();
    let mut response_sessions = Vec::new();
    let mut quotas = EngineQuotas::default();
    for quota in stored_quotas {
        quotas.set(quota);
    }

    for session in sessions {
        let usage = match session.engine {
            Engine::Claude => usage_by_session.remove(&session.session_id),
            Engine::Codex => session
                .transcript_path
                .as_deref()
                .and_then(|path| codex::read_transcript(path, &session.session_id)),
        };
        let Some(usage) = usage else {
            continue;
        };
        if session.engine == Engine::Codex && usage.rate_limits.has_any() {
            let should_replace = quotas
                .codex
                .as_ref()
                .is_none_or(|current| usage.observed_at > current.observed_at);
            if should_replace {
                quotas.set(EngineQuota {
                    engine: Engine::Codex,
                    rate_limits: usage.rate_limits.clone(),
                    observed_at: usage.observed_at,
                });
            }
        }
        response_sessions.push(usage);
    }

    response_sessions.sort_by(|left, right| left.session_id.cmp(&right.session_id));
    UsageResponse {
        sessions: response_sessions,
        quotas,
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Arc;

    use axum::body::{Body, to_bytes};
    use axum::http::{Request, StatusCode};
    use indoc::indoc;
    use serde_json::{Value, json};
    use tower::ServiceExt;

    use super::{UsageSource, router_with_source, usage_response};
    use crate::commands::agent::types::Engine;
    use crate::commands::agent::types::{Session, SessionStatus};
    use crate::commands::agent::usage::model::{
        ContextUsage, EngineQuota, EngineQuotas, RateLimitWindow, RateLimits, SessionUsage,
        UsageResponse,
    };

    #[derive(Clone)]
    struct StaticUsageSource(Arc<UsageResponse>);

    impl UsageSource for StaticUsageSource {
        async fn load(&self) -> anyhow::Result<UsageResponse> {
            Ok((*self.0).clone())
        }
    }

    fn codex_session(session_id: &str, transcript_path: PathBuf) -> Session {
        Session {
            launch_id: None,
            session_id: session_id.to_owned(),
            archived_at: None,
            pr_context: None,
            created_pr_urls: Vec::new(),
            crit_urls: Vec::new(),
            web_uis: Vec::new(),
            pending_human_review_ids: Default::default(),
            cwd: PathBuf::from("/workspace/fixture-project"),
            transcript_path: Some(transcript_path),
            tty: None,
            tmux_info: None,
            status: SessionStatus::Running,
            agent_status: None,
            agent_status_clear_pending: false,
            created_at: chrono::DateTime::parse_from_rfc3339("2026-11-16T11:00:00Z")
                .unwrap()
                .to_utc(),
            updated_at: chrono::DateTime::parse_from_rfc3339("2026-11-16T11:00:00Z")
                .unwrap()
                .to_utc(),
            last_prompted_at: None,
            last_message: None,
            current_tool: None,
            current_tool_name: None,
            label: None,
            work_type: None,
            vibe_enabled: false,
            launched_with_vibe: false,
            work_type_pinned: false,
            child_launch: Default::default(),
            ancestor_session_ids: Vec::new(),
            pending_bg_task_ids: Default::default(),
            pending_agent_task_ids: Default::default(),
            pending_permission_agent_ids: Default::default(),
            pending_permission_request_ids: Default::default(),
            read_at: None,
            sweep_signaled: false,
            stop_blocked_by_armyknife: false,
            engine: Engine::Codex,
            model: None,
            effort: None,
        }
    }

    #[test]
    fn includes_codex_transcript_usage_and_quota_in_response() {
        let directory = tempfile::tempdir().expect("temporary directory should be created");
        let transcript_path = directory.path().join("fixture-rollout.jsonl");
        std::fs::write(
            &transcript_path,
            indoc! {r#"
                {"type":"turn_context","payload":{"model":"fixture-model-cobalt","effort":"high"}}
                {"type":"session_meta","payload":{"cli_version":"0.99.0"}}
                {"type":"event_msg","timestamp":"2026-11-16T12:00:00Z","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":500,"total_tokens":520},"model_context_window":1000,"rate_limits":{"primary":{"used_percent":31,"window_duration_ms":18000000,"resets_at":1794841200},"secondary":{"used_percent":16,"window_duration_ms":604800000,"resets_at":1795435200}}}}}
            "#},
        )
        .expect("fixture rollout should be written");

        assert_eq!(
            usage_response(
                vec![codex_session("fixture-session", transcript_path)],
                Vec::new(),
                Vec::new(),
            ),
            UsageResponse {
                sessions: vec![SessionUsage {
                    session_id: "fixture-session".to_owned(),
                    engine: Engine::Codex,
                    model: Some("fixture-model-cobalt".to_owned()),
                    effort: Some("high".to_owned()),
                    context: Some(ContextUsage {
                        used_tokens: 520,
                        window_tokens: 1000,
                        used_percentage: 52,
                    }),
                    rate_limits: RateLimits {
                        five_hour: Some(RateLimitWindow {
                            used_percentage: 31.0,
                            resets_at: Some(1794841200),
                            window_seconds: 18_000,
                        }),
                        seven_day: Some(RateLimitWindow {
                            used_percentage: 16.0,
                            resets_at: Some(1795435200),
                            window_seconds: 604_800,
                        }),
                    },
                    version: Some("0.99.0".to_owned()),
                    observed_at: chrono::DateTime::parse_from_rfc3339("2026-11-16T12:00:00Z")
                        .unwrap()
                        .to_utc(),
                }],
                quotas: EngineQuotas {
                    claude: None,
                    codex: Some(EngineQuota {
                        engine: Engine::Codex,
                        rate_limits: RateLimits {
                            five_hour: Some(RateLimitWindow {
                                used_percentage: 31.0,
                                resets_at: Some(1794841200),
                                window_seconds: 18_000,
                            }),
                            seven_day: Some(RateLimitWindow {
                                used_percentage: 16.0,
                                resets_at: Some(1795435200),
                                window_seconds: 604_800,
                            }),
                        },
                        observed_at: chrono::DateTime::parse_from_rfc3339("2026-11-16T12:00:00Z",)
                            .unwrap()
                            .to_utc(),
                    }),
                },
            },
        );
    }

    #[tokio::test]
    async fn returns_sessions_and_engine_quotas() {
        let source = StaticUsageSource(Arc::new(UsageResponse {
            sessions: vec![SessionUsage {
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
                    seven_day: None,
                },
                version: Some("0.99.0".to_owned()),
                observed_at: chrono::DateTime::parse_from_rfc3339("2026-11-16T12:00:00Z")
                    .unwrap()
                    .to_utc(),
            }],
            quotas: EngineQuotas {
                claude: Some(EngineQuota {
                    engine: Engine::Claude,
                    rate_limits: RateLimits {
                        five_hour: Some(RateLimitWindow {
                            used_percentage: 32.5,
                            resets_at: Some(1794841200),
                            window_seconds: 18_000,
                        }),
                        seven_day: None,
                    },
                    observed_at: chrono::DateTime::parse_from_rfc3339("2026-11-16T12:00:00Z")
                        .unwrap()
                        .to_utc(),
                }),
                codex: None,
            },
        }));
        let response = router_with_source(source)
            .oneshot(Request::get("/usage").body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let actual: Value = serde_json::from_slice(&body).unwrap();

        assert_eq!(
            (status, actual),
            (
                StatusCode::OK,
                json!({
                    "sessions": [{
                        "sessionId": "fixture-session",
                        "engine": "claude",
                        "model": "fixture-model-amber",
                        "effort": "high",
                        "context": {
                            "usedTokens": 220,
                            "windowTokens": 1000,
                            "usedPercentage": 28,
                        },
                        "rateLimits": {
                            "fiveHour": {
                                "usedPercentage": 32.5,
                                "resetsAt": 1794841200,
                                "windowSeconds": 18000,
                            },
                            "sevenDay": null,
                        },
                        "version": "0.99.0",
                        "observedAt": "2026-11-16T12:00:00Z",
                    }],
                    "quotas": {
                        "claude": {
                            "engine": "claude",
                            "rateLimits": {
                                "fiveHour": {
                                    "usedPercentage": 32.5,
                                    "resetsAt": 1794841200,
                                    "windowSeconds": 18000,
                                },
                                "sevenDay": null,
                            },
                            "observedAt": "2026-11-16T12:00:00Z",
                        },
                        "codex": null,
                    }
                }),
            ),
        );
    }
}
