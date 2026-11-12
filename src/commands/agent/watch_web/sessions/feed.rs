use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;

use axum::http::StatusCode;
use serde::Serialize;

use super::{SessionView, SessionsResponse};

#[derive(Debug, Serialize)]
#[serde(
    tag = "mode",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub(super) enum SessionsFeedResponse {
    Full {
        token: String,
        revision: i64,
        work_type_nav_labels: HashMap<String, String>,
        sessions: Vec<SessionView>,
    },
    Delta {
        token: String,
        revision: i64,
        updated_sessions: Vec<SessionView>,
        removed_session_ids: Vec<String>,
        session_order: Vec<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        work_type_nav_labels: Option<HashMap<String, String>>,
    },
}

const SESSION_SNAPSHOT_HISTORY: usize = 5;

#[derive(Default)]
pub(super) struct SessionSnapshotCache {
    snapshots: VecDeque<(String, SessionsResponse)>,
}

#[derive(Debug, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct SessionsQuery {
    pub(super) since: Option<String>,
}

pub(super) fn session_feed_response(
    cache: &Mutex<SessionSnapshotCache>,
    since: Option<&str>,
    response: SessionsResponse,
) -> Result<SessionsFeedResponse, StatusCode> {
    let mut cache = cache
        .lock()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let token = uuid::Uuid::new_v4().to_string();
    let previous = since.and_then(|since| {
        cache
            .snapshots
            .iter()
            .find(|(saved_token, _)| saved_token == since)
            .map(|(_, snapshot)| snapshot.clone())
    });
    cache.snapshots.push_back((token.clone(), response.clone()));
    while cache.snapshots.len() > SESSION_SNAPSHOT_HISTORY {
        cache.snapshots.pop_front();
    }

    let Some(previous) = previous else {
        return Ok(SessionsFeedResponse::Full {
            token,
            revision: response.revision,
            work_type_nav_labels: response.work_type_nav_labels,
            sessions: response.sessions,
        });
    };

    let previous_by_id = previous
        .sessions
        .iter()
        .map(|session| (session.id.as_str(), session))
        .collect::<HashMap<_, _>>();
    let current_by_id = response
        .sessions
        .iter()
        .map(|session| (session.id.as_str(), session))
        .collect::<HashMap<_, _>>();
    let updated_sessions = response
        .sessions
        .iter()
        .filter(|session| {
            previous_by_id
                .get(session.id.as_str())
                .is_none_or(|previous| !same_session_except_elapsed(previous, session))
        })
        .cloned()
        .collect();
    let removed_session_ids = previous
        .sessions
        .iter()
        .filter(|session| !current_by_id.contains_key(session.id.as_str()))
        .map(|session| session.id.clone())
        .collect();
    let work_type_nav_labels = (previous.work_type_nav_labels != response.work_type_nav_labels)
        .then_some(response.work_type_nav_labels);

    Ok(SessionsFeedResponse::Delta {
        token,
        revision: response.revision,
        updated_sessions,
        removed_session_ids,
        session_order: response
            .sessions
            .iter()
            .map(|session| session.id.clone())
            .collect(),
        work_type_nav_labels,
    })
}

fn same_session_except_elapsed(left: &SessionView, right: &SessionView) -> bool {
    let mut left = left.clone();
    left.elapsed_seconds = 0;
    let mut right = right.clone();
    right.elapsed_seconds = 0;
    left == right
}
