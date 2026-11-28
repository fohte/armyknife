use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use axum::response::sse::Event;
use tokio::sync::{broadcast, watch};
use tokio::time::{self, Instant};

use crate::commands::agent::store;
use crate::commands::agent::types::{Engine, Session};
use crate::commands::agent::usage::codex;
use crate::commands::agent::usage::model::{EngineQuota, EngineQuotas, UsageResponse};

pub(super) trait UsageSource: Clone + Send + Sync + 'static {
    fn load(&self) -> impl Future<Output = anyhow::Result<UsageResponse>> + Send;
}

#[derive(Clone, Copy)]
pub(super) struct LiveUsageSource;

impl UsageSource for LiveUsageSource {
    async fn load(&self) -> anyhow::Result<UsageResponse> {
        tokio::task::spawn_blocking(load_usage_response).await?
    }
}

const USAGE_REFRESH_DEBOUNCE: Duration = Duration::from_millis(150);
const USAGE_MIN_REFRESH_INTERVAL: Duration = Duration::from_secs(1);

pub(super) type UsageSnapshot = Result<UsageResponse, String>;

#[derive(Clone)]
pub(super) struct UsageMonitor {
    inner: Arc<UsageMonitorInner>,
}

struct UsageMonitorInner {
    updates: watch::Sender<Option<UsageSnapshot>>,
    subscribers: watch::Sender<usize>,
}

pub(super) struct UsageSubscription {
    pub(super) updates: watch::Receiver<Option<UsageSnapshot>>,
    pub(super) _guard: UsageSubscriberGuard,
}

pub(super) fn usage_event(snapshot: UsageSnapshot) -> Event {
    match snapshot {
        Ok(usage) => Event::default()
            .event("usage")
            .json_data(usage)
            .unwrap_or_else(|_| {
                Event::default()
                    .event("usage-error")
                    .data("serialization failed")
            }),
        Err(error) => Event::default().event("usage-error").data(error),
    }
}

pub(super) struct UsageSubscriberGuard {
    subscribers: watch::Sender<usize>,
}

impl Drop for UsageSubscriberGuard {
    fn drop(&mut self) {
        self.subscribers
            .send_modify(|count| *count = count.saturating_sub(1));
    }
}

impl UsageMonitor {
    pub(super) fn new<S: UsageSource>(source: S, store_events: broadcast::Receiver<()>) -> Self {
        let (updates, _) = watch::channel(None);
        let (subscribers, subscriber_receiver) = watch::channel(0);
        tokio::spawn(run_usage_monitor(
            source,
            subscriber_receiver,
            updates.clone(),
            store_events,
        ));
        Self {
            inner: Arc::new(UsageMonitorInner {
                updates,
                subscribers,
            }),
        }
    }

    pub(super) fn subscribe(&self) -> UsageSubscription {
        self.inner.subscribers.send_modify(|count| {
            if *count == 0 {
                // Force a fresh read when a client reconnects after all subscribers leave.
                self.inner.updates.send_replace(None);
            }
            *count = count.saturating_add(1);
        });
        UsageSubscription {
            updates: self.inner.updates.subscribe(),
            _guard: UsageSubscriberGuard {
                subscribers: self.inner.subscribers.clone(),
            },
        }
    }
}

async fn run_usage_monitor<S: UsageSource>(
    source: S,
    mut subscriber_receiver: watch::Receiver<usize>,
    updates: watch::Sender<Option<UsageSnapshot>>,
    mut store_events: broadcast::Receiver<()>,
) {
    let mut event_channel_open = true;
    loop {
        while *subscriber_receiver.borrow_and_update() == 0 {
            if event_channel_open {
                tokio::select! {
                    changed = subscriber_receiver.changed() => {
                        if changed.is_err() {
                            return;
                        }
                    }
                    signal = store_events.recv() => {
                        if matches!(signal, Err(broadcast::error::RecvError::Closed)) {
                            event_channel_open = false;
                        }
                    }
                }
            } else if subscriber_receiver.changed().await.is_err() {
                return;
            }
        }

        refresh_usage(&source, &subscriber_receiver, &updates).await;
        let mut last_refresh_at = Instant::now();

        loop {
            if *subscriber_receiver.borrow() == 0 {
                break;
            }
            let has_signal = tokio::select! {
                changed = subscriber_receiver.changed() => {
                    if changed.is_err() {
                        return;
                    }
                    *subscriber_receiver.borrow() > 0
                }
                signal = store_events.recv(), if event_channel_open => {
                    match signal {
                        Ok(()) | Err(broadcast::error::RecvError::Lagged(_)) => true,
                        Err(broadcast::error::RecvError::Closed) => {
                            event_channel_open = false;
                            false
                        }
                    }
                }
            };
            if !has_signal {
                continue;
            }
            if !debounce_store_events(
                &mut subscriber_receiver,
                &mut store_events,
                &mut event_channel_open,
            )
            .await
            {
                break;
            }
            if !wait_for_refresh_interval(
                &mut subscriber_receiver,
                &mut store_events,
                &mut event_channel_open,
                last_refresh_at,
            )
            .await
            {
                break;
            }
            if *subscriber_receiver.borrow() > 0 {
                refresh_usage(&source, &subscriber_receiver, &updates).await;
                last_refresh_at = Instant::now();
            }
        }
    }
}

async fn debounce_store_events(
    subscribers: &mut watch::Receiver<usize>,
    store_events: &mut broadcast::Receiver<()>,
    event_channel_open: &mut bool,
) -> bool {
    loop {
        tokio::select! {
            () = time::sleep(USAGE_REFRESH_DEBOUNCE) => return *subscribers.borrow() > 0,
            changed = subscribers.changed() => {
                if changed.is_err() || *subscribers.borrow() == 0 {
                    return false;
                }
            }
            signal = store_events.recv(), if *event_channel_open => {
                match signal {
                    Ok(()) | Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(broadcast::error::RecvError::Closed) => *event_channel_open = false,
                }
            }
        }
    }
}

async fn wait_for_refresh_interval(
    subscribers: &mut watch::Receiver<usize>,
    store_events: &mut broadcast::Receiver<()>,
    event_channel_open: &mut bool,
    last_refresh_at: Instant,
) -> bool {
    let deadline = last_refresh_at + USAGE_MIN_REFRESH_INTERVAL;
    loop {
        tokio::select! {
            () = time::sleep_until(deadline) => return *subscribers.borrow() > 0,
            changed = subscribers.changed() => {
                if changed.is_err() || *subscribers.borrow() == 0 {
                    return false;
                }
            }
            signal = store_events.recv(), if *event_channel_open => {
                match signal {
                    Ok(()) | Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(broadcast::error::RecvError::Closed) => *event_channel_open = false,
                }
            }
        }
    }
}

async fn refresh_usage<S: UsageSource>(
    source: &S,
    subscribers: &watch::Receiver<usize>,
    updates: &watch::Sender<Option<UsageSnapshot>>,
) {
    let snapshot = source.load().await.map_err(|error| {
        tracing::warn!(%error, "failed to load agent usage");
        format!("{error:#}")
    });
    if *subscribers.borrow() == 0 || updates.borrow().as_ref() == Some(&snapshot) {
        return;
    }
    updates.send_replace(Some(snapshot));
}

#[cfg(test)]
pub(super) fn no_updates_subscription() -> UsageSubscription {
    let (updates, _) = watch::channel(None);
    let (subscribers, _) = watch::channel(1);
    UsageSubscription {
        updates: updates.subscribe(),
        _guard: UsageSubscriberGuard { subscribers },
    }
}

fn load_usage_response() -> anyhow::Result<UsageResponse> {
    let sessions_dir = store::sessions_dir()?;
    let sessions = store::list_all_sessions_from(&sessions_dir)?;
    let stored_usage = store::usage_snapshots_from(&sessions_dir)?;
    let stored_quotas = store::engine_quotas_from(&sessions_dir)?;
    let codex_quota = crate::shared::dirs::codex_home_dir()
        .as_deref()
        .map(codex::read_latest_quota_from)
        .transpose()?
        .flatten();
    Ok(usage_response(
        sessions,
        stored_usage,
        stored_quotas,
        codex_quota,
    ))
}

fn usage_response(
    sessions: Vec<Session>,
    stored_usage: Vec<crate::commands::agent::usage::model::SessionUsage>,
    stored_quotas: Vec<EngineQuota>,
    codex_quota: Option<EngineQuota>,
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
    if let Some(quota) = codex_quota
        && quotas
            .codex
            .as_ref()
            .is_none_or(|current| quota.observed_at > current.observed_at)
    {
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

    use indoc::indoc;

    use super::usage_response;
    use crate::commands::agent::types::Engine;
    use crate::commands::agent::types::{Session, SessionStatus};
    use crate::commands::agent::usage::codex;
    use crate::commands::agent::usage::model::{
        ContextUsage, EngineQuota, EngineQuotas, RateLimitWindow, RateLimits, SessionUsage,
        UsageResponse,
    };

    fn codex_session(session_id: &str, transcript_path: PathBuf) -> Session {
        Session {
            launch_id: None,
            session_id: session_id.to_owned(),
            archived_at: None,
            pr_context: None,
            created_pr_urls: Vec::new(),
            crit_urls: Vec::new(),
            crit_not_waiting_urls: Default::default(),
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
        let transcript_path = directory
            .path()
            .join("sessions/2026/11/16/rollout-fixture.jsonl");
        std::fs::create_dir_all(
            transcript_path
                .parent()
                .expect("fixture rollout should have a parent directory"),
        )
        .expect("fixture rollout directory should be created");
        std::fs::write(
            &transcript_path,
            indoc! {r#"
                {"type":"turn_context","payload":{"model":"fixture-model-cobalt","effort":"high"}}
                {"type":"session_meta","payload":{"cli_version":"0.99.0"}}
                {"type":"event_msg","timestamp":"2026-11-16T12:00:00Z","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":500,"total_tokens":520},"model_context_window":1000},"rate_limits":{"primary":{"used_percent":31,"window_minutes":300,"resets_at":1794841200},"secondary":{"used_percent":16,"window_minutes":10080,"resets_at":1795435200}}}}
            "#},
        )
        .expect("fixture rollout should be written");
        let codex_quota =
            codex::read_latest_quota_from(directory.path()).expect("Codex quota should load");

        assert_eq!(
            usage_response(
                vec![codex_session("fixture-session", transcript_path)],
                Vec::new(),
                Vec::new(),
                codex_quota,
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

    #[test]
    fn returns_codex_quota_without_a_visible_session() {
        let directory = tempfile::tempdir().expect("temporary Codex home should exist");
        let earlier_transcript_path = directory
            .path()
            .join("sessions/2026/11/16/rollout-earlier.jsonl");
        std::fs::create_dir_all(
            earlier_transcript_path
                .parent()
                .expect("fixture rollout should have a parent directory"),
        )
        .expect("fixture sessions directory should be created");
        std::fs::write(
            &earlier_transcript_path,
            indoc! {r#"
                {"type":"event_msg","timestamp":"2026-11-16T11:00:00Z","payload":{"type":"token_count","info":null,"rate_limits":{"primary":{"used_percent":8,"window_minutes":300,"resets_at":1794837600},"secondary":{"used_percent":5,"window_minutes":10080,"resets_at":1795431600}}}}
            "#},
        )
        .expect("earlier fixture rollout should be written");
        let transcript_path = directory
            .path()
            .join("archived_sessions/rollout-fixture.jsonl");
        std::fs::create_dir_all(
            transcript_path
                .parent()
                .expect("fixture rollout should have a parent directory"),
        )
        .expect("fixture archive directory should be created");
        std::fs::write(
            &transcript_path,
            indoc! {r#"
                {"type":"event_msg","timestamp":"2026-11-16T12:00:00Z","payload":{"type":"token_count","info":null,"rate_limits":{"primary":{"used_percent":42,"window_minutes":300,"resets_at":1794841200},"secondary":{"used_percent":21,"window_minutes":10080,"resets_at":1795435200}}}}
            "#},
        )
        .expect("archived fixture rollout should be written");
        let codex_quota = codex::read_latest_quota_from(directory.path())
            .expect("archived Codex quota should load");

        assert_eq!(
            usage_response(Vec::new(), Vec::new(), Vec::new(), codex_quota),
            UsageResponse {
                sessions: Vec::new(),
                quotas: EngineQuotas {
                    claude: None,
                    codex: Some(EngineQuota {
                        engine: Engine::Codex,
                        rate_limits: RateLimits {
                            five_hour: Some(RateLimitWindow {
                                used_percentage: 42.0,
                                resets_at: Some(1794841200),
                                window_seconds: 18_000,
                            }),
                            seven_day: Some(RateLimitWindow {
                                used_percentage: 21.0,
                                resets_at: Some(1795435200),
                                window_seconds: 604_800,
                            }),
                        },
                        observed_at: chrono::DateTime::parse_from_rfc3339("2026-11-16T12:00:00Z")
                            .unwrap()
                            .to_utc(),
                    }),
                },
            },
        );
    }
}
