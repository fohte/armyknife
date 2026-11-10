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
    expected_deadline: Option<DateTime<Utc>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TodayRequest {
    is_today: bool,
}

#[derive(Debug, PartialEq, Eq, Deserialize, Serialize)]
struct LaterPresetTime {
    when: String,
    deadline: Option<DateTime<Utc>>,
}

#[derive(Debug, PartialEq, Eq, Deserialize, Serialize)]
struct LaterPresetTimes {
    one_hour: LaterPresetTime,
    this_evening: LaterPresetTime,
    tomorrow: LaterPresetTime,
    next_week: LaterPresetTime,
    until_it_changes: LaterPresetTime,
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
    let later_until = if let Some(preset) = request.preset.timed() {
        let deadline = request.expected_deadline.ok_or(StatusCode::CONFLICT)?;
        if deadline <= now {
            clear_later(&state, &id)?;
            return Ok(StatusCode::NO_CONTENT);
        }
        if deadline > now + maximum_later_window(preset) {
            return Err(StatusCode::CONFLICT);
        }
        Some(deadline)
    } else {
        None
    };
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
    .map_err(|error| {
        if super::is_invalid_session_id_error(&error) {
            StatusCode::NOT_FOUND
        } else {
            StatusCode::INTERNAL_SERVER_ERROR
        }
    })?;
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
        one_hour: timed_later_preset_time(TimedLaterPreset::OneHour, now),
        this_evening: timed_later_preset_time(TimedLaterPreset::ThisEvening, now),
        tomorrow: timed_later_preset_time(TimedLaterPreset::Tomorrow, now),
        next_week: timed_later_preset_time(TimedLaterPreset::NextWeek, now),
        until_it_changes: LaterPresetTime {
            when: "next activity".to_string(),
            deadline: None,
        },
    }
}

fn timed_later_preset_time(preset: TimedLaterPreset, now: DateTime<Utc>) -> LaterPresetTime {
    let Some(deadline) = later_deadline(preset, now) else {
        return LaterPresetTime {
            when: "Unavailable".to_string(),
            deadline: None,
        };
    };
    let deadline = deadline.with_timezone(&Local);
    let today = now.with_timezone(&Local).date_naive();

    let when = match preset {
        TimedLaterPreset::OneHour => format!("{:02}:{:02}", deadline.hour(), deadline.minute()),
        TimedLaterPreset::ThisEvening if deadline.date_naive() == today => {
            format!("{:02}:{:02}", deadline.hour(), deadline.minute())
        }
        TimedLaterPreset::ThisEvening => format!(
            "{} {:02}:{:02}",
            deadline.format("%a"),
            deadline.hour(),
            deadline.minute()
        ),
        TimedLaterPreset::Tomorrow => format!(
            "{} {}:{:02}",
            deadline.format("%a"),
            deadline.hour(),
            deadline.minute()
        ),
        TimedLaterPreset::NextWeek => format!(
            "{} {}/{}",
            deadline.format("%a"),
            deadline.month(),
            deadline.day()
        ),
    };
    LaterPresetTime {
        when,
        deadline: Some(deadline.with_timezone(&Utc)),
    }
}

#[derive(Clone, Copy)]
enum TimedLaterPreset {
    OneHour,
    ThisEvening,
    Tomorrow,
    NextWeek,
}

impl LaterPreset {
    fn timed(self) -> Option<TimedLaterPreset> {
        match self {
            Self::OneHour => Some(TimedLaterPreset::OneHour),
            Self::ThisEvening => Some(TimedLaterPreset::ThisEvening),
            Self::Tomorrow => Some(TimedLaterPreset::Tomorrow),
            Self::NextWeek => Some(TimedLaterPreset::NextWeek),
            Self::BackNow | Self::UntilItChanges => None,
        }
    }
}

fn later_deadline(preset: TimedLaterPreset, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let local_now = now.with_timezone(&Local);
    let today = local_now.date_naive();
    let evening = NaiveTime::from_hms_opt(19, 0, 0)?;
    let (date, time) = match preset {
        TimedLaterPreset::OneHour => {
            let deadline = now + chrono::Duration::hours(1);
            return deadline.with_second(0)?.with_nanosecond(0);
        }
        TimedLaterPreset::ThisEvening if local_now.time() < evening => (today, evening),
        TimedLaterPreset::ThisEvening => (today + Days::new(1), evening),
        TimedLaterPreset::Tomorrow => (today + Days::new(1), NaiveTime::from_hms_opt(9, 0, 0)?),
        TimedLaterPreset::NextWeek => {
            let days_until_monday = 7 - local_now.weekday().num_days_from_monday();
            (
                today + Days::new(u64::from(days_until_monday)),
                NaiveTime::from_hms_opt(9, 0, 0)?,
            )
        }
    };
    match Local.from_local_datetime(&date.and_time(time)) {
        LocalResult::Single(deadline) => Some(deadline.with_timezone(&Utc)),
        LocalResult::Ambiguous(_, latest) => Some(latest.with_timezone(&Utc)),
        LocalResult::None => None,
    }
}

fn maximum_later_window(preset: TimedLaterPreset) -> chrono::Duration {
    match preset {
        TimedLaterPreset::OneHour => chrono::Duration::hours(1),
        TimedLaterPreset::ThisEvening => chrono::Duration::hours(25),
        TimedLaterPreset::Tomorrow => chrono::Duration::days(2),
        TimedLaterPreset::NextWeek => chrono::Duration::days(8),
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

    fn local_time_with_seconds_as_utc(
        (year, month, day, hour, minute, second): (i32, u32, u32, u32, u32, u32),
    ) -> DateTime<Utc> {
        Local
            .with_ymd_and_hms(year, month, day, hour, minute, second)
            .unwrap()
            .with_timezone(&Utc)
    }

    #[derive(Debug)]
    struct LaterOptionsCase {
        now_local: (i32, u32, u32, u32, u32),
        one_hour: &'static str,
        this_evening: &'static str,
        tomorrow: &'static str,
        one_hour_deadline: (i32, u32, u32, u32, u32),
        evening_deadline: (i32, u32, u32, u32, u32),
        tomorrow_deadline: (i32, u32, u32, u32, u32),
    }

    #[rstest]
    #[case::before_evening(LaterOptionsCase {
        now_local: (2030, 3, 4, 15, 0),
        one_hour: "16:00",
        this_evening: "19:00",
        tomorrow: "Tue 9:00",
        one_hour_deadline: (2030, 3, 4, 16, 0),
        evening_deadline: (2030, 3, 4, 19, 0),
        tomorrow_deadline: (2030, 3, 5, 9, 0),
    })]
    #[case::after_evening(LaterOptionsCase {
        now_local: (2030, 3, 4, 20, 0),
        one_hour: "21:00",
        this_evening: "Tue 19:00",
        tomorrow: "Tue 9:00",
        one_hour_deadline: (2030, 3, 4, 21, 0),
        evening_deadline: (2030, 3, 5, 19, 0),
        tomorrow_deadline: (2030, 3, 5, 9, 0),
    })]
    #[tokio::test]
    async fn later_options_show_server_calculated_return_times(
        temp_dir: TempDir,
        #[case] expected: LaterOptionsCase,
    ) {
        let now = local_time_as_utc(expected.now_local);
        let app = router_with_store(SessionStateStore {
            sessions_dir: Some(temp_dir.path().to_path_buf()),
            clock: SessionClock::Fixed(now),
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
        let options = serde_json::from_slice::<LaterPresetTimes>(&body).unwrap();

        assert_eq!(
            (status, options),
            (
                StatusCode::OK,
                LaterPresetTimes {
                    one_hour: LaterPresetTime {
                        when: expected.one_hour.to_string(),
                        deadline: Some(local_time_as_utc(expected.one_hour_deadline)),
                    },
                    this_evening: LaterPresetTime {
                        when: expected.this_evening.to_string(),
                        deadline: Some(local_time_as_utc(expected.evening_deadline)),
                    },
                    tomorrow: LaterPresetTime {
                        when: expected.tomorrow.to_string(),
                        deadline: Some(local_time_as_utc(expected.tomorrow_deadline)),
                    },
                    next_week: LaterPresetTime {
                        when: "Mon 3/11".to_string(),
                        deadline: Some(local_time_as_utc((2030, 3, 11, 9, 0))),
                    },
                    until_it_changes: LaterPresetTime {
                        when: "next activity".to_string(),
                        deadline: None,
                    },
                },
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
                    .body(Body::from(
                        serde_json::to_vec(&serde_json::json!({
                            "preset": preset,
                            "expectedDeadline": expected_later_until,
                        }))
                        .unwrap(),
                    ))
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
    #[case::one_hour_deadline_survives_minute_rollover(
        "one_hour",
        (2030, 3, 4, 15, 59, 0),
        (2030, 3, 4, 16, 0, 0),
        Some((2030, 3, 4, 16, 59, 0)),
    )]
    #[case::this_evening_deadline_expires_at_19(
        "this_evening",
        (2030, 3, 4, 18, 59, 0),
        (2030, 3, 4, 19, 0, 1),
        None,
    )]
    #[tokio::test]
    async fn later_requests_handle_preset_rollovers(
        saved_session_dir: TempDir,
        session: Session,
        #[case] preset: &str,
        #[case] menu_opened_at: (i32, u32, u32, u32, u32, u32),
        #[case] submitted_at: (i32, u32, u32, u32, u32, u32),
        #[case] expected_stored_deadline: Option<(i32, u32, u32, u32, u32, u32)>,
    ) {
        let menu_options = later_preset_times(local_time_with_seconds_as_utc(menu_opened_at));
        let menu_deadline = match preset {
            "one_hour" => menu_options.one_hour.deadline,
            "this_evening" => menu_options.this_evening.deadline,
            _ => unreachable!(),
        }
        .unwrap();
        let now = local_time_with_seconds_as_utc(submitted_at);
        let app = router_with_store(SessionStateStore {
            sessions_dir: Some(saved_session_dir.path().to_path_buf()),
            clock: SessionClock::Fixed(now),
        });
        let response = app
            .oneshot(
                Request::post("/sessions/test-session/later")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&serde_json::json!({
                            "preset": preset,
                            "expectedDeadline": menu_deadline,
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        let expected_later_until = expected_stored_deadline.map(local_time_with_seconds_as_utc);
        let expected_state = store::WatchWebSessionState {
            session_id: session.session_id,
            later_until: expected_later_until,
            activity_snapshot: expected_later_until.map(|_| session.updated_at),
            until_activity: false,
            is_today: false,
        };
        assert_eq!(
            (
                response.status(),
                store::watch_web_session_states_from(saved_session_dir.path()).unwrap(),
            ),
            (StatusCode::NO_CONTENT, vec![expected_state]),
        );
    }

    #[rstest]
    #[tokio::test]
    async fn rejects_a_later_deadline_that_is_too_far_in_the_future(saved_session_dir: TempDir) {
        let app = router_with_store(SessionStateStore {
            sessions_dir: Some(saved_session_dir.path().to_path_buf()),
            clock: SessionClock::Fixed(local_time_as_utc((2030, 3, 4, 15, 0))),
        });
        let response = app
            .oneshot(
                Request::post("/sessions/test-session/later")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&serde_json::json!({
                            "preset": "one_hour",
                            "expectedDeadline": local_time_as_utc((2030, 3, 4, 17, 0)),
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(
            (
                response.status(),
                store::watch_web_session_states_from(saved_session_dir.path()).unwrap(),
            ),
            (StatusCode::CONFLICT, vec![]),
        );
    }

    #[rstest]
    #[tokio::test]
    async fn malformed_session_ids_return_not_found(saved_session_dir: TempDir) {
        let app = router_with_store(SessionStateStore {
            sessions_dir: Some(saved_session_dir.path().to_path_buf()),
            clock: SessionClock::System,
        });
        let response = app
            .oneshot(
                Request::post("/sessions/a..b/today")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"isToday":true}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
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
