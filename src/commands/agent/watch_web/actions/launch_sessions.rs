use std::sync::Arc;

use anyhow::Result;
use axum::extract::State;
use axum::routing::post;
use axum::{Json, Router};
use chrono::Utc;
use clap::ValueEnum;
use serde::{Deserialize, Serialize};

use crate::commands::agent::new::{CommonNewArgs, NewArgs};
use crate::commands::agent::types::{Engine, ReasoningEffort};
use crate::commands::agent::watch_web::paths::normalize_directory;

use super::ApiError;

pub(super) fn router() -> Router {
    router_with_host(Arc::new(LiveLaunchHost))
}

fn router_with_host(host: Arc<dyn LaunchHost>) -> Router {
    Router::new()
        .route("/sessions", post(launch_session))
        .with_state(LaunchState { host })
}

#[derive(Clone)]
struct LaunchState {
    host: Arc<dyn LaunchHost>,
}

trait LaunchHost: Send + Sync {
    fn launch(&self, args: NewArgs) -> Result<()>;
}

struct LiveLaunchHost;

impl LaunchHost for LiveLaunchHost {
    fn launch(&self, args: NewArgs) -> Result<()> {
        crate::commands::agent::new::run(&args)
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LaunchRequest {
    directory: String,
    engine: String,
    worktree: bool,
    model: Option<String>,
    reasoning_effort: Option<String>,
    initial_message: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LaunchResponse {
    launch_id: String,
}

async fn launch_session(
    State(state): State<LaunchState>,
    Json(request): Json<LaunchRequest>,
) -> Result<Json<LaunchResponse>, ApiError> {
    let home = crate::shared::dirs::home_dir();
    let directory = normalize_directory(&request.directory, home.as_deref())
        .map_err(|error| ApiError::bad_request(error.to_string()))?;
    let engine = match request.engine.as_str() {
        "claude" => Engine::Claude,
        "codex" => Engine::Codex,
        _ => return Err(ApiError::bad_request("engine must be claude or codex")),
    };
    let reasoning_effort = request
        .reasoning_effort
        .as_deref()
        .map(parse_reasoning_effort)
        .transpose()
        .map_err(ApiError::bad_request)?;
    let launch_id = uuid::Uuid::new_v4().to_string();
    let initial_message = request
        .initial_message
        .filter(|message| !message.trim().is_empty());
    let worktree = request.worktree.then(|| {
        initial_message
            .is_none()
            .then(|| branch_name_for_time(Utc::now(), &launch_id))
    });
    let args = NewArgs {
        worktree,
        from: None,
        force: false,
        vibe: false,
        common: CommonNewArgs {
            watch_web_launch_id: Some(launch_id.clone()),
            watch_web_directory: (!request.worktree).then_some(directory.clone()),
            prompt: initial_message,
            child_launch: None,
            label: None,
            kind: None,
            type_name: None,
            model: request.model,
            parent_session_id: None,
            repo: request.worktree.then_some(directory),
            engine: Some(engine),
            reasoning_effort,
        },
        skip_hooks: false,
        dry_run: false,
    };

    let host = state.host;
    tokio::task::spawn_blocking(move || host.launch(args))
        .await
        .map_err(ApiError::internal)?
        .map_err(ApiError::operation)?;

    Ok(Json(LaunchResponse { launch_id }))
}

fn parse_reasoning_effort(value: &str) -> Result<ReasoningEffort, String> {
    ReasoningEffort::value_variants()
        .iter()
        .copied()
        .find(|effort| effort.as_str() == value)
        .ok_or_else(|| "reasoningEffort must be low, medium, high, xhigh, or max".to_string())
}

fn branch_name_for_time(now: chrono::DateTime<Utc>, launch_id: &str) -> String {
    format!("watch-web-{}-{launch_id}", now.format("%Y%m%d-%H%M%S"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{Body, to_bytes};
    use axum::http::{Request, StatusCode};
    use chrono::TimeZone;
    use serde_json::{Value, json};
    use std::path::PathBuf;
    use std::sync::Mutex;
    use tower::ServiceExt;

    #[derive(Default)]
    struct FakeLaunchHost {
        calls: Mutex<Vec<NewArgs>>,
    }

    #[derive(Debug, PartialEq, Eq)]
    struct LaunchCapture {
        launch_id: Option<String>,
        directory: Option<PathBuf>,
        exact_directory: Option<PathBuf>,
        engine: Option<Engine>,
        prompt: Option<String>,
        model: Option<String>,
        reasoning_effort: Option<ReasoningEffort>,
        worktree: Option<Option<String>>,
    }

    impl From<NewArgs> for LaunchCapture {
        fn from(args: NewArgs) -> Self {
            Self {
                launch_id: args.common.watch_web_launch_id,
                directory: args.common.repo,
                exact_directory: args.common.watch_web_directory,
                engine: args.common.engine,
                prompt: args.common.prompt,
                model: args.common.model,
                reasoning_effort: args.common.reasoning_effort,
                worktree: args.worktree,
            }
        }
    }

    impl LaunchHost for FakeLaunchHost {
        fn launch(&self, args: NewArgs) -> Result<()> {
            self.calls.lock().unwrap().push(args);
            Ok(())
        }
    }

    async fn request(router: &Router, body: Value) -> (StatusCode, Value) {
        let response = router
            .clone()
            .oneshot(
                Request::post("/sessions")
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (status, serde_json::from_slice(&body).unwrap_or(Value::Null))
    }

    #[tokio::test]
    async fn launches_the_requested_engine_in_the_canonical_directory() {
        let directory = tempfile::tempdir().unwrap();
        let host = Arc::new(FakeLaunchHost::default());
        let router = router_with_host(host.clone());
        let (status, response) = request(
            &router,
            json!({
                "directory": directory.path(),
                "engine": "codex",
                "worktree": false,
                "initialMessage": "Inspect the fixture.",
            }),
        )
        .await;
        let args = LaunchCapture::from(host.calls.lock().unwrap().pop().unwrap());
        let launch_id = response["launchId"].as_str().unwrap().to_string();

        assert_eq!(
            (status, response, args),
            (
                StatusCode::OK,
                json!({"launchId": launch_id.clone()}),
                LaunchCapture {
                    launch_id: Some(launch_id),
                    directory: None,
                    exact_directory: Some(directory.path().canonicalize().unwrap()),
                    engine: Some(Engine::Codex),
                    prompt: Some("Inspect the fixture.".to_string()),
                    model: None,
                    reasoning_effort: None,
                    worktree: None,
                },
            ),
        );
    }

    #[tokio::test]
    async fn forwards_selected_model_and_reasoning_effort_to_launch() {
        let directory = tempfile::tempdir().unwrap();
        let host = Arc::new(FakeLaunchHost::default());
        let router = router_with_host(host.clone());
        let (status, _) = request(
            &router,
            json!({
                "directory": directory.path(),
                "engine": "codex",
                "worktree": false,
                "model": "fixture-model",
                "reasoningEffort": "xhigh",
            }),
        )
        .await;
        let args = LaunchCapture::from(host.calls.lock().unwrap().pop().unwrap());

        assert_eq!(
            (status, args.model, args.reasoning_effort),
            (
                StatusCode::OK,
                Some("fixture-model".to_string()),
                Some(ReasoningEffort::XHigh),
            ),
        );
    }

    #[tokio::test]
    async fn launches_a_promptless_worktree_with_a_unique_timestamp_branch() {
        let directory = tempfile::tempdir().unwrap();
        let host = Arc::new(FakeLaunchHost::default());
        let router = router_with_host(host.clone());
        let (status, response) = request(
            &router,
            json!({
                "directory": directory.path(),
                "engine": "claude",
                "worktree": true,
            }),
        )
        .await;
        let args = LaunchCapture::from(host.calls.lock().unwrap().pop().unwrap());
        let launch_id = response["launchId"].as_str().unwrap().to_string();
        let worktree = args.worktree.as_ref().map(|branch| {
            branch
                .as_ref()
                .map(|branch| normalize_branch_timestamp(branch, &launch_id))
        });

        assert_eq!(
            (
                status,
                response,
                args.directory,
                args.exact_directory,
                args.engine,
                args.prompt,
                args.model,
                args.reasoning_effort,
                worktree,
            ),
            (
                StatusCode::OK,
                json!({"launchId": launch_id.clone()}),
                Some(directory.path().canonicalize().unwrap()),
                None,
                Some(Engine::Claude),
                None,
                None,
                None,
                Some(Some(format!("watch-web-<timestamp>-{launch_id}"))),
            ),
        );
    }

    #[rstest::rstest]
    #[case::unsupported_engine(json!({"directory":"/","engine":"shell","worktree":false}), StatusCode::BAD_REQUEST)]
    #[case::missing_directory(json!({"directory":"/missing-fixture-directory","engine":"claude","worktree":false}), StatusCode::BAD_REQUEST)]
    #[case::unsupported_effort(json!({"directory":"/","engine":"claude","worktree":false,"reasoningEffort":"focused"}), StatusCode::BAD_REQUEST)]
    #[tokio::test]
    async fn rejects_invalid_engine_or_directory(
        #[case] body: Value,
        #[case] expected_status: StatusCode,
    ) {
        let host = Arc::new(FakeLaunchHost::default());
        let router = router_with_host(host.clone());

        let (status, _) = request(&router, body).await;

        assert_eq!(
            (status, host.calls.lock().unwrap().len()),
            (expected_status, 0)
        );
    }

    #[test]
    fn worktree_without_message_uses_a_timestamp_branch() {
        let now = Utc.with_ymd_and_hms(2026, 11, 16, 3, 4, 5).unwrap();

        assert_eq!(
            branch_name_for_time(now, "launch-fixture"),
            "watch-web-20261116-030405-launch-fixture"
        );
    }

    fn normalize_branch_timestamp(branch: &str, launch_id: &str) -> String {
        let suffix = format!("-{launch_id}");
        let timestamp = branch
            .strip_prefix("watch-web-")
            .and_then(|branch| branch.strip_suffix(&suffix));
        if timestamp.is_some_and(|timestamp| {
            timestamp.len() == 15
                && timestamp.bytes().enumerate().all(|(index, byte)| {
                    if index == 8 {
                        byte == b'-'
                    } else {
                        byte.is_ascii_digit()
                    }
                })
        }) {
            format!("watch-web-<timestamp>-{launch_id}")
        } else {
            branch.to_string()
        }
    }
}
