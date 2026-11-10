use std::path::PathBuf;

use axum::extract::{Path as RoutePath, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::{DateTime, Datelike, Days, Local, LocalResult, NaiveTime, TimeZone, Timelike, Utc};
use serde::{Deserialize, Serialize};

use crate::commands::agent::store;

#[derive(Clone)]
struct SessionStateStore {
    sessions_dir: Option<PathBuf>,
    clock: SessionClock,
}

#[derive(Clone)]
enum SessionClock {
    System,
    #[cfg(test)]
    Fixed(DateTime<Utc>),
}

impl SessionClock {
    fn now(&self) -> DateTime<Utc> {
        match self {
            Self::System => Utc::now(),
            #[cfg(test)]
            Self::Fixed(now) => *now,
        }
    }
}

pub(super) fn router() -> Router {
    router_with_store(SessionStateStore {
        sessions_dir: None,
        clock: SessionClock::System,
    })
}

fn router_with_store(store: SessionStateStore) -> Router {
    Router::new()
        .route("/sessions/later-options", get(later_options))
        .route("/sessions/{id}/later", post(set_later))
        .route("/sessions/{id}/today", post(set_today))
        .with_state(store)
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum LaterPreset {
    BackNow,
    OneHour,
    ThisEvening,
    Tomorrow,
    NextWeek,
    UntilItChanges,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LaterRequest {
    preset: LaterPreset,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TodayRequest {
    is_today: bool,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
struct LaterPresetTimes {
    one_hour: String,
    this_evening: String,
    tomorrow: String,
    next_week: String,
    until_it_changes: String,
}

async fn later_options(State(state): State<SessionStateStore>) -> Json<LaterPresetTimes> {
    Json(later_preset_times(state.clock.now()))
}

async fn set_later(
    State(state): State<SessionStateStore>,
    RoutePath(id): RoutePath<String>,
    Json(request): Json<LaterRequest>,
) -> Result<StatusCode, StatusCode> {
    let session = load_session(&state, &id)?;
    if matches!(request.preset, LaterPreset::BackNow) {
        clear_later(&state, &id)?;
        return Ok(StatusCode::NO_CONTENT);
    }

    let now = state.clock.now();
    let until_activity = matches!(request.preset, LaterPreset::UntilItChanges);
    let later_until = (!until_activity)
        .then(|| later_deadline(request.preset, now))
        .flatten();
    save_later(&state, &id, later_until, session.updated_at, until_activity)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn set_today(
    State(state): State<SessionStateStore>,
    RoutePath(id): RoutePath<String>,
    Json(request): Json<TodayRequest>,
) -> Result<StatusCode, StatusCode> {
    load_session(&state, &id)?;
    save_today(&state, &id, request.is_today)?;
    Ok(StatusCode::NO_CONTENT)
}

fn load_session(
    state: &SessionStateStore,
    id: &str,
) -> Result<crate::commands::agent::types::Session, StatusCode> {
    let session = match &state.sessions_dir {
        Some(dir) => store::load_session_from(dir, id),
        None => store::load_session(id),
    }
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    session
        .filter(|session| session.archived_at.is_none())
        .ok_or(StatusCode::NOT_FOUND)
}

fn save_later(
    state: &SessionStateStore,
    id: &str,
    later_until: Option<DateTime<Utc>>,
    activity_snapshot: DateTime<Utc>,
    until_activity: bool,
) -> Result<(), StatusCode> {
    let updated = if let Some(dir) = &state.sessions_dir {
        store::set_watch_web_later_in(dir, id, later_until, activity_snapshot, until_activity)
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    } else {
        store::set_watch_web_later(id, later_until, activity_snapshot, until_activity)
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    };
    updated.then_some(()).ok_or(StatusCode::NOT_FOUND)
}

fn clear_later(state: &SessionStateStore, id: &str) -> Result<(), StatusCode> {
    let updated = if let Some(dir) = &state.sessions_dir {
        store::clear_watch_web_later_in(dir, id).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    } else {
        store::clear_watch_web_later(id).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    };
    updated.then_some(()).ok_or(StatusCode::NOT_FOUND)
}

fn save_today(state: &SessionStateStore, id: &str, is_today: bool) -> Result<(), StatusCode> {
    let updated = if let Some(dir) = &state.sessions_dir {
        store::set_watch_web_today_in(dir, id, is_today)
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    } else {
        store::set_watch_web_today(id, is_today).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
    };
    updated.then_some(()).ok_or(StatusCode::NOT_FOUND)
}

fn later_preset_times(now: DateTime<Utc>) -> LaterPresetTimes {
    LaterPresetTimes {
        one_hour: later_preset_when(LaterPreset::OneHour, now),
        this_evening: later_preset_when(LaterPreset::ThisEvening, now),
        tomorrow: later_preset_when(LaterPreset::Tomorrow, now),
        next_week: later_preset_when(LaterPreset::NextWeek, now),
        until_it_changes: later_preset_when(LaterPreset::UntilItChanges, now),
    }
}

fn later_preset_when(preset: LaterPreset, now: DateTime<Utc>) -> String {
    if matches!(preset, LaterPreset::UntilItChanges) {
        return "next activity".to_string();
    }
    if matches!(preset, LaterPreset::BackNow) {
        return "now".to_string();
    }
    let Some(deadline) = later_deadline(preset, now) else {
        return "Unavailable".to_string();
    };
    let deadline = deadline.with_timezone(&Local);
    let today = now.with_timezone(&Local).date_naive();

    match preset {
        LaterPreset::OneHour => format!("{:02}:{:02}", deadline.hour(), deadline.minute()),
        LaterPreset::ThisEvening if deadline.date_naive() == today => {
            format!("{:02}:{:02}", deadline.hour(), deadline.minute())
        }
        LaterPreset::ThisEvening => format!(
            "{} {:02}:{:02}",
            deadline.format("%a"),
            deadline.hour(),
            deadline.minute()
        ),
        LaterPreset::Tomorrow => format!(
            "{} {}:{:02}",
            deadline.format("%a"),
            deadline.hour(),
            deadline.minute()
        ),
        LaterPreset::NextWeek => format!(
            "{} {}/{}",
            deadline.format("%a"),
            deadline.month(),
            deadline.day()
        ),
        LaterPreset::UntilItChanges => "next activity".to_string(),
        LaterPreset::BackNow => "now".to_string(),
    }
}

fn later_deadline(preset: LaterPreset, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let local_now = now.with_timezone(&Local);
    let today = local_now.date_naive();
    let evening = NaiveTime::from_hms_opt(19, 0, 0)?;
    let (date, time) = match preset {
        LaterPreset::OneHour => return Some(now + chrono::Duration::hours(1)),
        LaterPreset::ThisEvening if local_now.time() < evening => (today, evening),
        LaterPreset::ThisEvening => (today + Days::new(1), evening),
        LaterPreset::Tomorrow => (today + Days::new(1), NaiveTime::from_hms_opt(9, 0, 0)?),
        LaterPreset::NextWeek => {
            let days_until_monday = 7 - local_now.weekday().num_days_from_monday();
            (
                today + Days::new(u64::from(days_until_monday)),
                NaiveTime::from_hms_opt(9, 0, 0)?,
            )
        }
        LaterPreset::UntilItChanges | LaterPreset::BackNow => return None,
    };
    match Local.from_local_datetime(&date.and_time(time)) {
        LocalResult::Single(deadline) => Some(deadline.with_timezone(&Utc)),
        LocalResult::Ambiguous(_, latest) => Some(latest.with_timezone(&Utc)),
        LocalResult::None => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::agent::store;
    use crate::commands::agent::types::{Engine, Session, SessionStatus};
    use axum::body::{Body, to_bytes};
    use axum::http::header::{HOST, ORIGIN};
    use axum::http::{Request, StatusCode};
    use axum::middleware::from_fn;
    use chrono::TimeZone;
    use rstest::{fixture, rstest};
    use std::path::PathBuf;
    use tempfile::TempDir;
    use tower::ServiceExt;

    #[fixture]
    fn temp_dir() -> TempDir {
        TempDir::new().expect("temporary directory should be created")
    }

    #[fixture]
    fn session() -> Session {
        Session {
            session_id: "test-session".to_string(),
            archived_at: None,
            pr_context: None,
            crit_urls: Vec::new(),
            pending_human_review_ids: Default::default(),
            cwd: PathBuf::from("/workspace/project"),
            transcript_path: None,
            tty: None,
            tmux_info: None,
            status: SessionStatus::Running,
            agent_status: None,
            agent_status_clear_pending: false,
            created_at: Utc.timestamp_opt(1_000, 0).unwrap(),
            updated_at: Utc.timestamp_opt(1_000, 0).unwrap(),
            last_message: None,
            current_tool: None,
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
            engine: Engine::Claude,
            model: None,
            effort: None,
        }
    }

    #[fixture]
    fn saved_session_dir(temp_dir: TempDir, session: Session) -> TempDir {
        store::save_session_to(temp_dir.path(), &session).expect("session should save");
        temp_dir
    }

    #[rstest]
    #[tokio::test]
    async fn state_updates_require_origin_and_missing_sessions_return_not_found(temp_dir: TempDir) {
        let app = axum::Router::new()
            .nest(
                "/api",
                router_with_store(SessionStateStore {
                    sessions_dir: Some(temp_dir.path().to_path_buf()),
                    clock: SessionClock::System,
                }),
            )
            .layer(from_fn(super::super::security::validate_request));
        let make_request = |origin: Option<&str>| {
            let mut builder = Request::post("/api/sessions/absent/today")
                .header(HOST, "localhost")
                .header("content-type", "application/json");
            if let Some(origin) = origin {
                builder = builder.header(ORIGIN, origin);
            }
            builder.body(Body::from(r#"{"isToday":true}"#)).unwrap()
        };

        let rejected = app.clone().oneshot(make_request(None)).await.unwrap();
        let missing = app
            .oneshot(make_request(Some("http://localhost")))
            .await
            .unwrap();

        assert_eq!(
            (rejected.status(), missing.status()),
            (StatusCode::FORBIDDEN, StatusCode::NOT_FOUND),
        );
    }

    #[rstest]
    #[tokio::test]
    async fn state_routes_persist_later_and_today_without_changing_the_session(
        saved_session_dir: TempDir,
        session: Session,
    ) {
        let before_revision = store::session_store_revision_from(saved_session_dir.path()).unwrap();
        let app = router_with_store(SessionStateStore {
            sessions_dir: Some(saved_session_dir.path().to_path_buf()),
            clock: SessionClock::System,
        });

        let later_response = app
            .clone()
            .oneshot(
                Request::post("/sessions/test-session/later")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"preset":"until_it_changes"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        let today_response = app
            .oneshot(
                Request::post("/sessions/test-session/today")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"isToday":true}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(
            (
                later_response.status(),
                today_response.status(),
                store::watch_web_session_states_from(saved_session_dir.path()).unwrap(),
                store::session_store_revision_from(saved_session_dir.path()).unwrap(),
                store::load_session_from(saved_session_dir.path(), "test-session")
                    .unwrap()
                    .unwrap()
                    .updated_at,
            ),
            (
                StatusCode::NO_CONTENT,
                StatusCode::NO_CONTENT,
                vec![store::WatchWebSessionState {
                    session_id: "test-session".to_string(),
                    later_until: None,
                    activity_snapshot: Some(session.updated_at),
                    until_activity: true,
                    is_today: true,
                }],
                before_revision + 2,
                session.updated_at,
            ),
        );
    }

    #[rstest]
    #[tokio::test]
    async fn back_now_clears_later_state_and_preserves_today(
        saved_session_dir: TempDir,
        session: Session,
    ) {
        let dir = saved_session_dir.path();
        store::set_watch_web_today_in(dir, &session.session_id, true).unwrap();
        store::set_watch_web_later_in(
            dir,
            &session.session_id,
            Some(session.updated_at + chrono::Duration::hours(1)),
            session.updated_at,
            false,
        )
        .unwrap();
        let before_revision = store::session_store_revision_from(dir).unwrap();
        let app = router_with_store(SessionStateStore {
            sessions_dir: Some(dir.to_path_buf()),
            clock: SessionClock::System,
        });
        let response = app
            .oneshot(
                Request::post("/sessions/test-session/later")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"preset":"back_now"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(
            (
                response.status(),
                store::watch_web_session_states_from(dir).unwrap(),
                store::session_store_revision_from(dir).unwrap(),
            ),
            (
                StatusCode::NO_CONTENT,
                vec![store::WatchWebSessionState {
                    session_id: session.session_id,
                    later_until: None,
                    activity_snapshot: None,
                    until_activity: false,
                    is_today: true,
                }],
                before_revision + 1,
            ),
        );
    }

    fn local_time_as_utc(
        (year, month, day, hour, minute): (i32, u32, u32, u32, u32),
    ) -> DateTime<Utc> {
        Local
            .with_ymd_and_hms(year, month, day, hour, minute, 0)
            .unwrap()
            .with_timezone(&Utc)
    }

    #[rstest]
    #[case::before_evening((2030, 3, 4, 15, 0), "16:00", "19:00", "Tue 9:00")]
    #[case::after_evening((2030, 3, 4, 20, 0), "21:00", "Tue 19:00", "Tue 9:00")]
    #[tokio::test]
    async fn later_options_show_server_calculated_return_times(
        temp_dir: TempDir,
        #[case] now_local: (i32, u32, u32, u32, u32),
        #[case] one_hour: &str,
        #[case] this_evening: &str,
        #[case] tomorrow: &str,
    ) {
        let app = router_with_store(SessionStateStore {
            sessions_dir: Some(temp_dir.path().to_path_buf()),
            clock: SessionClock::Fixed(local_time_as_utc(now_local)),
        });
        let response = app
            .oneshot(
                Request::get("/sessions/later-options")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let options = serde_json::from_slice::<serde_json::Value>(&body).unwrap();

        assert_eq!(
            (status, options),
            (
                StatusCode::OK,
                serde_json::json!({
                    "one_hour": one_hour,
                    "this_evening": this_evening,
                    "tomorrow": tomorrow,
                    "next_week": "Mon 3/11",
                    "until_it_changes": "next activity",
                }),
            ),
        );
    }

    #[rstest]
    #[case::one_hour("one_hour", (2030, 3, 4, 15, 0), Some((2030, 3, 4, 16, 0)), false)]
    #[case::this_evening("this_evening", (2030, 3, 4, 15, 0), Some((2030, 3, 4, 19, 0)), false)]
    #[case::evening_already_passed("this_evening", (2030, 3, 4, 20, 0), Some((2030, 3, 5, 19, 0)), false)]
    #[case::tomorrow("tomorrow", (2030, 3, 4, 15, 0), Some((2030, 3, 5, 9, 0)), false)]
    #[case::next_week("next_week", (2030, 3, 4, 15, 0), Some((2030, 3, 11, 9, 0)), false)]
    #[case::until_it_changes("until_it_changes", (2030, 3, 4, 15, 0), None, true)]
    #[tokio::test]
    async fn later_presets_store_expected_deadlines_through_the_route(
        saved_session_dir: TempDir,
        session: Session,
        #[case] preset: &str,
        #[case] now_local: (i32, u32, u32, u32, u32),
        #[case] expected_local_deadline: Option<(i32, u32, u32, u32, u32)>,
        #[case] until_activity: bool,
    ) {
        let now = local_time_as_utc(now_local);
        let expected_later_until = expected_local_deadline.map(local_time_as_utc);
        let app = router_with_store(SessionStateStore {
            sessions_dir: Some(saved_session_dir.path().to_path_buf()),
            clock: SessionClock::Fixed(now),
        });
        let response = app
            .oneshot(
                Request::post("/sessions/test-session/later")
                    .header("content-type", "application/json")
                    .body(Body::from(format!(r#"{{"preset":"{preset}"}}"#)))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(
            (
                response.status(),
                store::watch_web_session_states_from(saved_session_dir.path()).unwrap()
            ),
            (
                StatusCode::NO_CONTENT,
                vec![store::WatchWebSessionState {
                    session_id: session.session_id,
                    later_until: expected_later_until,
                    activity_snapshot: Some(session.updated_at),
                    until_activity,
                    is_today: false,
                }],
            ),
        );
    }

    #[rstest]
    fn later_updates_preserve_today_state(saved_session_dir: TempDir, session: Session) {
        let dir = saved_session_dir.path();
        store::set_watch_web_today_in(dir, "test-session", true).unwrap();
        store::set_watch_web_later_in(dir, "test-session", None, session.updated_at, true).unwrap();
        assert_eq!(
            store::watch_web_session_states_from(dir).unwrap(),
            vec![store::WatchWebSessionState {
                session_id: "test-session".to_string(),
                later_until: None,
                activity_snapshot: Some(session.updated_at),
                until_activity: true,
                is_today: true,
            }]
        );
    }
}
