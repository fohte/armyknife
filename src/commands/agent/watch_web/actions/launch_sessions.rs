use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use axum::extract::{DefaultBodyLimit, State};
use axum::routing::post;
use axum::{Json, Router};
use chrono::Utc;
use clap::ValueEnum;
use serde::{Deserialize, Serialize};

use crate::commands::agent::new::{CommonNewArgs, NewArgs};
use crate::commands::agent::store;
use crate::commands::agent::types::{Engine, ReasoningEffort};
use crate::commands::agent::watch_web::attachments;
use crate::commands::agent::watch_web::messages::queued_prompt;
use crate::commands::agent::watch_web::paths::normalize_directory;

use super::ApiError;

pub(super) fn router() -> Router {
    router_with_host(Arc::new(LiveLaunchHost))
}

fn router_with_host(host: Arc<dyn LaunchHost>) -> Router {
    router_with_state(LaunchState {
        host,
        attachment_dir: None,
        sessions_dir: None,
        branch_name_for_message: Arc::new(|message| {
            let backend = crate::commands::name_branch::detect_backend();
            crate::commands::name_branch::generate_branch_name(message, backend.as_ref())
        }),
    })
}

#[cfg(test)]
fn router_with_host_and_attachment_dir(
    host: Arc<dyn LaunchHost>,
    attachment_dir: PathBuf,
    sessions_dir: PathBuf,
) -> Router {
    router_with_state(LaunchState {
        host,
        attachment_dir: Some(attachment_dir),
        sessions_dir: Some(sessions_dir),
        branch_name_for_message: Arc::new(|message| Ok(format!("branch-from-{message}"))),
    })
}

fn router_with_state(state: LaunchState) -> Router {
    Router::new()
        .route("/sessions", post(launch_session))
        .layer(DefaultBodyLimit::max(
            super::messages::MAX_MESSAGE_BODY_BYTES,
        ))
        .with_state(state)
}

#[derive(Clone)]
struct LaunchState {
    host: Arc<dyn LaunchHost>,
    attachment_dir: Option<PathBuf>,
    sessions_dir: Option<PathBuf>,
    branch_name_for_message: Arc<BranchNameGenerator>,
}

type BranchNameGenerator = dyn Fn(&str) -> Result<String> + Send + Sync;

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
    #[serde(default)]
    attachments: Vec<attachments::Request>,
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
    let validated_attachments = attachments::validate(request.attachments, vec![])
        .map_err(|error| ApiError::bad_request(error.to_string()))?;
    let launch_id = uuid::Uuid::new_v4().to_string();
    let initial_message = request
        .initial_message
        .filter(|message| !message.trim().is_empty());
    let saved_attachments = tokio::task::spawn_blocking({
        let attachment_dir = state.attachment_dir.clone();
        move || attachments::save(validated_attachments, attachment_dir)
    })
    .await
    .map_err(ApiError::internal)?
    .map_err(ApiError::operation)?;
    let attachment_paths = saved_attachments
        .iter()
        .map(|attachment| attachment.path.clone())
        .collect::<Vec<_>>();
    let has_attachments = !attachment_paths.is_empty();
    let sessions_dir = has_attachments
        .then(|| super::support::resolve_sessions_dir(state.sessions_dir.clone()))
        .transpose()
        .map_err(ApiError::internal)?;
    let launch_session_id = format!("watch-web-launch:{launch_id}");
    let queued_attachments = saved_attachments
        .iter()
        .map(|attachment| match &attachment.image_source {
            Some(source) => store::WatchWebQueuedAttachment::Image {
                source: source.clone(),
            },
            None => store::WatchWebQueuedAttachment::File {
                name: attachment.name.clone(),
            },
        })
        .collect::<Vec<_>>();
    let queued_prompt_id = if let Some(sessions_dir) = sessions_dir.as_ref() {
        let sessions_dir = sessions_dir.clone();
        let launch_session_id = launch_session_id.clone();
        let content = initial_message.clone().unwrap_or_default();
        let attachments = queued_attachments.clone();
        match super::run_blocking(move || {
            store::insert_watch_web_queued_prompt_from(
                &sessions_dir,
                &launch_session_id,
                &launch_session_id,
                &content,
                &attachments,
                None,
                Utc::now(),
            )
        })
        .await
        {
            Ok(id) => Some(id),
            Err(error) => {
                super::messages::remove_attachments(attachment_paths).await;
                return Err(error);
            }
        }
    } else {
        None
    };
    let mut prompt = attachments::append_paths_to_message(
        initial_message.clone().unwrap_or_default(),
        &attachment_paths,
    );
    if queued_prompt_id.is_some() {
        prompt = queued_prompt::add_id_marker(&prompt, &launch_session_id);
    }
    let prompt = (!prompt.trim().is_empty()).then_some(prompt);
    let args = NewArgs {
        worktree: request.worktree.then_some(None),
        from: None,
        force: false,
        vibe: false,
        watch_web_prompt_cache: has_attachments.then(|| initial_message.clone()),
        common: CommonNewArgs {
            watch_web_launch_id: Some(launch_id.clone()),
            watch_web_directory: (!request.worktree).then_some(directory.clone()),
            prompt,
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
    let branch_message = initial_message;
    let branch_name_for_message = state.branch_name_for_message;
    let launch_time = Utc::now();
    let branch_launch_id = launch_id.clone();
    let launch_result = tokio::task::spawn_blocking(move || {
        let mut args = args;
        args.worktree = resolve_worktree(
            args.worktree.is_some(),
            branch_message.as_deref(),
            has_attachments,
            &branch_launch_id,
            launch_time,
            |message| branch_name_for_message(message),
        )?;
        host.launch(args)
    })
    .await;
    match launch_result {
        Ok(Ok(())) => {}
        Ok(Err(error)) => {
            if let (Some(sessions_dir), Some(prompt_id)) = (&sessions_dir, queued_prompt_id) {
                remove_initial_prompt_row(sessions_dir, &launch_session_id, Some(prompt_id)).await;
            }
            super::messages::remove_attachments(attachment_paths).await;
            return Err(ApiError::operation(error));
        }
        Err(error) => {
            if let (Some(sessions_dir), Some(prompt_id)) = (&sessions_dir, queued_prompt_id) {
                remove_initial_prompt_row(sessions_dir, &launch_session_id, Some(prompt_id)).await;
            }
            super::messages::remove_attachments(attachment_paths).await;
            return Err(ApiError::internal(error));
        }
    }

    Ok(Json(LaunchResponse { launch_id }))
}

async fn remove_initial_prompt_row(
    sessions_dir: &std::path::Path,
    launch_session_id: &str,
    queued_prompt_id: Option<i64>,
) {
    let Some(id) = queued_prompt_id else {
        return;
    };
    let sessions_dir = sessions_dir.to_path_buf();
    let session_id = launch_session_id.to_owned();
    if let Err(error) = super::run_blocking(move || {
        store::delete_watch_web_queued_prompts_from(&sessions_dir, &session_id, &[id])
    })
    .await
    {
        tracing::warn!(
            session_id = %launch_session_id,
            error = ?error,
            "failed to remove unsent watch-web initial prompt"
        );
    }
}

fn resolve_worktree(
    requested: bool,
    initial_message: Option<&str>,
    has_attachments: bool,
    launch_id: &str,
    now: chrono::DateTime<Utc>,
    branch_name_for_message: impl FnOnce(&str) -> Result<String>,
) -> Result<Option<Option<String>>> {
    if !requested {
        return Ok(None);
    }
    let Some(message) = initial_message else {
        return Ok(Some(Some(branch_name_for_time(now, launch_id))));
    };
    if has_attachments {
        return Ok(Some(Some(branch_name_for_message(message)?)));
    }
    Ok(Some(None))
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
    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD;
    use chrono::TimeZone;
    use serde_json::{Value, json};
    use std::sync::Mutex;
    use tower::ServiceExt;

    #[derive(Default)]
    struct FakeLaunchHost {
        calls: Mutex<Vec<NewArgs>>,
        fail: bool,
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
            if self.fail {
                anyhow::bail!("launch failed");
            }
            Ok(())
        }
    }

    const PNG_SIGNATURE: &[u8] = &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];

    fn image_request(name: &str, bytes: &[u8]) -> Value {
        json!({
            "name": name,
            "mimeType": "image/png",
            "size": bytes.len(),
            "data": STANDARD.encode(bytes),
        })
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
    async fn saves_initial_images_and_appends_their_paths_to_the_first_message() {
        let directory = tempfile::tempdir().unwrap();
        let storage = tempfile::tempdir().unwrap();
        let sessions = tempfile::tempdir().unwrap();
        let host = Arc::new(FakeLaunchHost::default());
        let router = router_with_host_and_attachment_dir(
            host.clone(),
            storage.path().to_path_buf(),
            sessions.path().to_path_buf(),
        );
        let mut image = PNG_SIGNATURE.to_vec();
        image.resize(2 * 1024 * 1024 + 1, 0);
        let (status, response) = request(
            &router,
            json!({
                "directory": directory.path(),
                "engine": "codex",
                "worktree": false,
                "initialMessage": "Inspect this image.",
                "attachments": [image_request("fixture.png", &image)],
            }),
        )
        .await;
        let args = host.calls.lock().unwrap().pop().unwrap();
        let saved_path = std::fs::read_dir(storage.path())
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path()
            .canonicalize()
            .unwrap();
        let launch_session_id = format!(
            "watch-web-launch:{}",
            response["launchId"].as_str().unwrap()
        );
        let mut queued_prompts =
            store::watch_web_queued_prompts_from(sessions.path(), &launch_session_id).unwrap();
        for prompt in &mut queued_prompts {
            prompt.sent_at = Utc.with_ymd_and_hms(2026, 1, 2, 3, 4, 5).single().unwrap();
        }
        let image_name = saved_path.file_name().unwrap().to_str().unwrap();

        assert_eq!(
            (
                status,
                args.common.prompt,
                std::fs::read(&saved_path).unwrap(),
                queued_prompts,
            ),
            (
                StatusCode::OK,
                Some(
                    indoc::formatdoc! {"
                        Inspect this image.

                        Attached file(s):
                        - {}
                        <!-- armyknife-queued-prompt-id:{} -->
                    ", saved_path.display(), launch_session_id}
                    .trim_end()
                    .to_string(),
                ),
                image,
                vec![store::WatchWebQueuedPrompt {
                    id: 1,
                    message_id: Some(launch_session_id.clone()),
                    content: "Inspect this image.".to_string(),
                    action_event_kind: None,
                    attachments: vec![store::WatchWebQueuedAttachment::Image {
                        source: format!("/api/images/{image_name}"),
                    }],
                    resolved: false,
                    sent_at: Utc.with_ymd_and_hms(2026, 1, 2, 3, 4, 5).single().unwrap(),
                }],
            ),
        );
    }

    #[tokio::test]
    async fn removes_saved_images_when_launch_fails() {
        let directory = tempfile::tempdir().unwrap();
        let storage = tempfile::tempdir().unwrap();
        let sessions = tempfile::tempdir().unwrap();
        let host = Arc::new(FakeLaunchHost {
            fail: true,
            ..FakeLaunchHost::default()
        });
        let router = router_with_host_and_attachment_dir(
            host.clone(),
            storage.path().to_path_buf(),
            sessions.path().to_path_buf(),
        );
        let (status, _) = request(
            &router,
            json!({
                "directory": directory.path(),
                "engine": "claude",
                "worktree": false,
                "initialMessage": "Recover this request.",
                "attachments": [image_request("fixture.png", PNG_SIGNATURE)],
            }),
        )
        .await;
        let args = host.calls.lock().unwrap().pop().unwrap();
        let launch_session_id = format!(
            "watch-web-launch:{}",
            args.common.watch_web_launch_id.as_deref().unwrap()
        );

        assert_eq!(
            (
                status,
                args.watch_web_prompt_cache,
                std::fs::read_dir(storage.path())
                    .unwrap()
                    .map(|entry| entry.unwrap().path())
                    .collect::<Vec<_>>(),
                store::watch_web_queued_prompts_from(sessions.path(), &launch_session_id).unwrap(),
            ),
            (
                StatusCode::BAD_GATEWAY,
                Some(Some("Recover this request.".to_string())),
                vec![],
                vec![],
            ),
        );
    }

    #[tokio::test]
    async fn rejects_invalid_initial_image_data_without_launching() {
        let directory = tempfile::tempdir().unwrap();
        let host = Arc::new(FakeLaunchHost::default());
        let router = router_with_host(host.clone());
        let (status, _) = request(
            &router,
            json!({
                "directory": directory.path(),
                "engine": "claude",
                "worktree": false,
                "attachments": [{
                    "name": "fixture.png",
                    "mimeType": "image/png",
                    "size": 1,
                    "data": "?",
                }],
            }),
        )
        .await;

        assert_eq!(
            (status, host.calls.lock().unwrap().len()),
            (StatusCode::BAD_REQUEST, 0),
        );
    }

    #[tokio::test]
    async fn worktree_launch_generates_its_branch_from_the_user_message() {
        let directory = tempfile::tempdir().unwrap();
        let storage = tempfile::tempdir().unwrap();
        let sessions = tempfile::tempdir().unwrap();
        let host = Arc::new(FakeLaunchHost::default());
        let router = router_with_host_and_attachment_dir(
            host.clone(),
            storage.path().to_path_buf(),
            sessions.path().to_path_buf(),
        );
        let (status, _) = request(
            &router,
            json!({
                "directory": directory.path(),
                "engine": "codex",
                "worktree": true,
                "initialMessage": "Update the dashboard layout",
                "attachments": [image_request("fixture.png", PNG_SIGNATURE)],
            }),
        )
        .await;
        let args = host.calls.lock().unwrap().pop().unwrap();
        let saved_path = std::fs::read_dir(storage.path())
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path()
            .canonicalize()
            .unwrap();

        assert_eq!(
            (
                status,
                args.worktree,
                args.common.prompt,
                args.watch_web_prompt_cache,
            ),
            (
                StatusCode::OK,
                Some(Some("branch-from-Update the dashboard layout".to_string())),
                Some(
                    indoc::formatdoc! {"
                        Update the dashboard layout

                        Attached file(s):
                        - {}
                    ", saved_path.display()}
                    .trim_end()
                    .to_string(),
                ),
                Some(Some("Update the dashboard layout".to_string())),
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
