use std::path::PathBuf;

use axum::extract::{Path as RoutePath, State};
use axum::http::StatusCode;
use axum::routing::post;
use axum::{Json, Router};
use chrono::{DateTime, Datelike, Days, Local, LocalResult, NaiveTime, TimeZone, Utc};
use serde::Deserialize;

use crate::commands::agent::store;

#[derive(Clone)]
struct SessionStateStore {
    sessions_dir: Option<PathBuf>,
}

pub(super) fn router() -> Router {
    router_with_store(SessionStateStore { sessions_dir: None })
}

fn router_with_store(store: SessionStateStore) -> Router {
    Router::new()
        .route("/sessions/{id}/later", post(set_later))
        .route("/sessions/{id}/today", post(set_today))
        .with_state(store)
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum LaterPreset {
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

async fn set_later(
    State(state): State<SessionStateStore>,
    RoutePath(id): RoutePath<String>,
    Json(request): Json<LaterRequest>,
) -> Result<StatusCode, StatusCode> {
    let session = load_session(&state, &id)?;
    let now = Utc::now();
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
    if let Some(dir) = &state.sessions_dir {
        store::set_watch_web_later_in(dir, id, later_until, activity_snapshot, until_activity)
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        store::touch_session_in(dir, id).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
    } else {
        store::set_watch_web_later(id, later_until, activity_snapshot, until_activity)
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        store::touch_session(id).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
    }
}

fn save_today(state: &SessionStateStore, id: &str, is_today: bool) -> Result<(), StatusCode> {
    if let Some(dir) = &state.sessions_dir {
        store::set_watch_web_today_in(dir, id, is_today)
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        store::touch_session_in(dir, id).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
    } else {
        store::set_watch_web_today(id, is_today).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        store::touch_session(id).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
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
                NaiveTime::from_hms_opt(11, 0, 0)?,
            )
        }
        LaterPreset::UntilItChanges => return None,
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
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use chrono::TimeZone;
    use std::path::PathBuf;
    use tower::ServiceExt;

    fn session(id: &str) -> Session {
        Session {
            session_id: id.to_string(),
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

    #[tokio::test]
    async fn missing_sessions_return_not_found_for_state_updates() {
        let temp = tempfile::tempdir().unwrap();
        let app = router_with_store(SessionStateStore {
            sessions_dir: Some(temp.path().to_path_buf()),
        });
        let response = app
            .oneshot(
                Request::post("/sessions/absent/today")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"isToday":true}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn state_routes_persist_later_and_today_without_changing_the_session() {
        let temp = tempfile::tempdir().unwrap();
        let existing = session("session");
        store::save_session_to(temp.path(), &existing).unwrap();
        let app = router_with_store(SessionStateStore {
            sessions_dir: Some(temp.path().to_path_buf()),
        });

        let later_response = app
            .clone()
            .oneshot(
                Request::post("/sessions/session/later")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"preset":"until_it_changes"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        let today_response = app
            .oneshot(
                Request::post("/sessions/session/today")
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
                store::watch_web_session_states_from(temp.path()).unwrap(),
                store::load_session_from(temp.path(), "session")
                    .unwrap()
                    .unwrap()
                    .updated_at,
            ),
            (
                StatusCode::NO_CONTENT,
                StatusCode::NO_CONTENT,
                vec![store::WatchWebSessionState {
                    session_id: "session".to_string(),
                    later_until: None,
                    activity_snapshot: Some(existing.updated_at),
                    until_activity: true,
                    is_today: true,
                }],
                existing.updated_at,
            ),
        );
    }

    #[test]
    fn later_presets_calculate_local_return_times() {
        let local_now = Local.with_ymd_and_hms(2030, 3, 4, 15, 0, 0).unwrap();
        let now = local_now.with_timezone(&Utc);
        let local_deadline =
            |preset| later_deadline(preset, now).map(|deadline| deadline.with_timezone(&Local));

        assert_eq!(
            (
                local_deadline(LaterPreset::OneHour),
                local_deadline(LaterPreset::ThisEvening),
                local_deadline(LaterPreset::Tomorrow),
                local_deadline(LaterPreset::NextWeek),
                later_deadline(LaterPreset::UntilItChanges, now),
            ),
            (
                Some(Local.with_ymd_and_hms(2030, 3, 4, 16, 0, 0).unwrap()),
                Some(Local.with_ymd_and_hms(2030, 3, 4, 19, 0, 0).unwrap()),
                Some(Local.with_ymd_and_hms(2030, 3, 5, 9, 0, 0).unwrap()),
                Some(Local.with_ymd_and_hms(2030, 3, 11, 11, 0, 0).unwrap()),
                None,
            ),
        );
    }

    #[test]
    fn later_updates_preserve_today_state() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path();
        let existing = session("session");
        store::save_session_to(dir, &existing).unwrap();
        store::set_watch_web_today_in(dir, "session", true).unwrap();
        store::set_watch_web_later_in(dir, "session", None, existing.updated_at, true).unwrap();
        assert_eq!(
            store::watch_web_session_states_from(dir).unwrap(),
            vec![store::WatchWebSessionState {
                session_id: "session".to_string(),
                later_until: None,
                activity_snapshot: Some(existing.updated_at),
                until_activity: true,
                is_today: true,
            }]
        );
    }
}
