use std::sync::Arc;

use anyhow::Context;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::routing::get;
use axum::{Json, Router};
use clap::ValueEnum;
use serde::{Deserialize, Serialize};

use crate::commands::agent::codex_steer::Client as CodexClient;
use crate::commands::agent::types::{Engine, ReasoningEffort};
use crate::shared::config::{ResolvedLaunchOptions, load_config};

pub(super) fn router() -> Router {
    router_with_source(Arc::new(LiveModelOptionsSource))
}

fn router_with_source(source: Arc<dyn ModelOptionsSource>) -> Router {
    Router::new()
        .route("/model-options", get(model_options))
        .with_state(ModelOptionsState { source })
}

#[derive(Clone)]
struct ModelOptionsState {
    source: Arc<dyn ModelOptionsSource>,
}

trait ModelOptionsSource: Send + Sync {
    fn load(&self, engine: Engine) -> anyhow::Result<ModelOptionsResponse>;
}

struct LiveModelOptionsSource;

impl ModelOptionsSource for LiveModelOptionsSource {
    fn load(&self, engine: Engine) -> anyhow::Result<ModelOptionsResponse> {
        let config = load_config()?;
        let resolved = config
            .agent
            .resolve_launch_options(None, Some(engine), None, None);
        match engine {
            Engine::Claude => Ok(claude_options(resolved)),
            Engine::Codex => codex_options(load_codex_model_list()?, resolved),
        }
    }
}

fn load_codex_model_list() -> anyhow::Result<serde_json::Value> {
    load_model_list_with_fallback(
        || {
            let mut client = CodexClient::connect()?;
            client.list_models()
        },
        crate::commands::agent::watch_web::codex_model_catalog::list_models,
    )
}

fn load_model_list_with_fallback(
    persistent: impl FnOnce() -> anyhow::Result<serde_json::Value>,
    temporary: impl FnOnce() -> anyhow::Result<serde_json::Value>,
) -> anyhow::Result<serde_json::Value> {
    match persistent() {
        Ok(response) => Ok(response),
        Err(error) => temporary()
            .with_context(|| format!("persistent Codex app-server request failed: {error:#}")),
    }
}

#[derive(Debug, Deserialize)]
struct ModelOptionsQuery {
    engine: Engine,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct ModelOptionsResponse {
    default_model: Option<String>,
    default_effort: Option<String>,
    models: Vec<ModelOption>,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct ModelOption {
    id: String,
    label: String,
    default_effort: Option<String>,
    efforts: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CodexModelList {
    data: Vec<CodexModel>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CodexModel {
    model: String,
    display_name: String,
    hidden: bool,
    supported_reasoning_efforts: Vec<CodexReasoningEffort>,
    default_reasoning_effort: String,
    is_default: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CodexReasoningEffort {
    reasoning_effort: String,
}

async fn model_options(
    State(state): State<ModelOptionsState>,
    Query(query): Query<ModelOptionsQuery>,
) -> Result<Json<ModelOptionsResponse>, StatusCode> {
    let source = state.source;
    tokio::task::spawn_blocking(move || source.load(query.engine))
        .await
        .map_err(|error| {
            tracing::warn!(%error, "model options task failed");
            StatusCode::INTERNAL_SERVER_ERROR
        })?
        .map(Json)
        .map_err(|error| {
            tracing::warn!(%error, "failed to load model options");
            StatusCode::SERVICE_UNAVAILABLE
        })
}

fn claude_options(resolved: ResolvedLaunchOptions) -> ModelOptionsResponse {
    let models = [
        ("opus", "Opus", "medium"),
        ("sonnet", "Sonnet", "medium"),
        ("haiku", "Haiku", "medium"),
        ("fable", "Fable", "medium"),
    ]
    .into_iter()
    .map(|(id, label, default_effort)| ModelOption {
        id: id.to_string(),
        label: label.to_string(),
        default_effort: Some(default_effort.to_string()),
        efforts: ReasoningEffort::value_variants()
            .iter()
            .map(|effort| effort.as_str().to_string())
            .collect(),
    })
    .collect();
    ModelOptionsResponse {
        default_model: resolved.model,
        default_effort: resolved
            .reasoning_effort
            .map(|effort| effort.as_str().to_string())
            .or_else(|| Some("medium".to_string())),
        models,
    }
}

fn codex_options(
    response: serde_json::Value,
    resolved: ResolvedLaunchOptions,
) -> anyhow::Result<ModelOptionsResponse> {
    let model_list: CodexModelList = serde_json::from_value(response)?;
    let catalog_default_model = model_list
        .data
        .iter()
        .find(|model| model.is_default && !model.hidden)
        .map(|model| model.model.clone());
    let models = model_list
        .data
        .into_iter()
        .filter(|model| !model.hidden)
        .map(|model| {
            let efforts = model
                .supported_reasoning_efforts
                .into_iter()
                .filter_map(|effort| known_effort(&effort.reasoning_effort))
                .map(str::to_string)
                .collect::<Vec<_>>();
            let default_effort = known_effort(&model.default_reasoning_effort)
                .map(str::to_string)
                .filter(|effort| efforts.contains(effort));
            ModelOption {
                id: model.model,
                label: model.display_name,
                default_effort,
                efforts,
            }
        })
        .collect::<Vec<_>>();
    let default_model = resolved.model.or(catalog_default_model);
    let default_effort = resolved
        .reasoning_effort
        .map(|effort| effort.as_str().to_string())
        .or_else(|| {
            default_model.as_ref().and_then(|default_model| {
                models
                    .iter()
                    .find(|model| &model.id == default_model)
                    .and_then(|model| model.default_effort.clone())
            })
        });
    Ok(ModelOptionsResponse {
        default_model,
        default_effort,
        models,
    })
}

fn known_effort(value: &str) -> Option<&'static str> {
    ReasoningEffort::value_variants()
        .iter()
        .find(|effort| effort.as_str() == value)
        .map(ReasoningEffort::as_str)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{Body, to_bytes};
    use axum::http::Request;
    use serde_json::{Value, json};
    use std::sync::Mutex;
    use tower::ServiceExt;

    #[derive(Default)]
    struct FakeModelOptionsSource {
        engines: Mutex<Vec<Engine>>,
    }

    impl ModelOptionsSource for FakeModelOptionsSource {
        fn load(&self, engine: Engine) -> anyhow::Result<ModelOptionsResponse> {
            self.engines.lock().unwrap().push(engine);
            Ok(ModelOptionsResponse {
                default_model: Some("fixture-model-default".to_string()),
                default_effort: Some("medium".to_string()),
                models: vec![ModelOption {
                    id: "fixture-model-default".to_string(),
                    label: "Fixture model".to_string(),
                    default_effort: Some("medium".to_string()),
                    efforts: vec!["low".to_string(), "medium".to_string()],
                }],
            })
        }
    }

    async fn get(router: &Router, uri: &str) -> (StatusCode, Value) {
        let response = router
            .clone()
            .oneshot(Request::get(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (status, serde_json::from_slice(&body).unwrap_or(Value::Null))
    }

    #[tokio::test]
    async fn returns_the_engine_model_options() {
        let source = Arc::new(FakeModelOptionsSource::default());
        let router = router_with_source(source.clone());

        let (status, body) = get(&router, "/model-options?engine=codex").await;

        assert_eq!(
            (status, body, source.engines.lock().unwrap().clone()),
            (
                StatusCode::OK,
                json!({
                    "defaultModel": "fixture-model-default",
                    "defaultEffort": "medium",
                    "models": [{
                        "id": "fixture-model-default",
                        "label": "Fixture model",
                        "defaultEffort": "medium",
                        "efforts": ["low", "medium"],
                    }],
                }),
                vec![Engine::Codex],
            ),
        );
    }

    #[test]
    fn maps_codex_model_catalog_defaults_and_supported_efforts() {
        let response = json!({
            "data": [
                {
                    "model": "fixture-model-one",
                    "displayName": "Fixture One",
                    "hidden": false,
                    "isDefault": true,
                    "defaultReasoningEffort": "high",
                    "supportedReasoningEfforts": [
                        { "reasoningEffort": "low", "description": "Low" },
                        { "reasoningEffort": "high", "description": "High" },
                        { "reasoningEffort": "focused", "description": "Provider specific" }
                    ]
                },
                {
                    "model": "fixture-hidden-model",
                    "displayName": "Hidden fixture",
                    "hidden": true,
                    "isDefault": false,
                    "defaultReasoningEffort": "medium",
                    "supportedReasoningEfforts": []
                }
            ],
            "nextCursor": null
        });
        let catalog = codex_options(
            response,
            ResolvedLaunchOptions {
                engine: Engine::Codex,
                model: None,
                reasoning_effort: None,
            },
        )
        .unwrap();

        assert_eq!(
            catalog,
            ModelOptionsResponse {
                default_model: Some("fixture-model-one".to_string()),
                default_effort: Some("high".to_string()),
                models: vec![ModelOption {
                    id: "fixture-model-one".to_string(),
                    label: "Fixture One".to_string(),
                    default_effort: Some("high".to_string()),
                    efforts: vec!["low".to_string(), "high".to_string()],
                }],
            },
        );
    }

    #[test]
    fn returns_claude_aliases_with_documented_effort_defaults() {
        let catalog = claude_options(ResolvedLaunchOptions {
            engine: Engine::Claude,
            model: None,
            reasoning_effort: None,
        });

        assert_eq!(
            catalog,
            ModelOptionsResponse {
                default_model: None,
                default_effort: Some("medium".to_string()),
                models: ["opus", "sonnet", "haiku", "fable"]
                    .into_iter()
                    .map(|id| ModelOption {
                        id: id.to_string(),
                        label: match id {
                            "opus" => "Opus",
                            "sonnet" => "Sonnet",
                            "haiku" => "Haiku",
                            "fable" => "Fable",
                            _ => unreachable!(),
                        }
                        .to_string(),
                        default_effort: Some("medium".to_string()),
                        efforts: ["low", "medium", "high", "xhigh", "max"]
                            .into_iter()
                            .map(str::to_string)
                            .collect(),
                    })
                    .collect(),
            },
        );
    }
}
