use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use axum::Router;
use axum::body::Body;
use axum::extract::Path as AxumPath;
use axum::extract::State;
use axum::http::StatusCode;
use axum::http::header::CONTENT_TYPE;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use serde::Serialize;

use crate::commands::agent::claude_sessions;
use crate::commands::agent::types::{Engine, Session};

mod parser;

type SessionLoader = Arc<dyn Fn(&str) -> Result<Option<Session>> + Send + Sync>;

pub(super) fn routes_with(
    load_session: impl Fn(&str) -> Result<Option<Session>> + Send + Sync + 'static,
) -> Router {
    Router::new()
        .route("/sessions/{id}/messages", get(get_messages))
        .with_state(Arc::new(load_session) as SessionLoader)
}

async fn get_messages(
    State(load_session): State<SessionLoader>,
    AxumPath(session_id): AxumPath<String>,
) -> Response {
    match tokio::task::spawn_blocking(move || {
        load_messages_with(&session_id, |id| load_session(id))
    })
    .await
    {
        Ok(Ok(Some(messages))) => match serde_json::to_vec(&messages) {
            Ok(body) => (
                StatusCode::OK,
                [(CONTENT_TYPE, "application/json")],
                Body::from(body),
            )
                .into_response(),
            Err(error) => {
                tracing::warn!(%error, "failed to serialize agent session messages");
                StatusCode::INTERNAL_SERVER_ERROR.into_response()
            }
        },
        Ok(Ok(None)) => StatusCode::NOT_FOUND.into_response(),
        Ok(Err(error)) => {
            tracing::warn!(%error, "failed to read agent session messages");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
        Err(error) => {
            tracing::error!(%error, "agent session message worker failed");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

fn load_messages_with(
    session_id: &str,
    mut load_session: impl FnMut(&str) -> Result<Option<Session>>,
) -> Result<Option<Vec<ChatItem>>> {
    let Some(session) = load_session(session_id)? else {
        return Ok(None);
    };
    let from = parent_title(&session, &mut load_session)?;
    let pending_permission = session.has_pending_permission_requests();
    let pending_tool = pending_permission
        .then_some(session.current_tool.as_deref())
        .flatten();
    let mut messages = session
        .transcript_path
        .as_deref()
        .map(|path| parse_session_transcript(path, session.engine, from.as_deref(), pending_tool))
        .transpose()?
        .unwrap_or_default();

    if pending_permission
        && !messages
            .iter()
            .any(|message| matches!(message, ChatItem::Permission { .. }))
    {
        messages.push(ChatItem::Permission {
            tool: session
                .current_tool
                .clone()
                .unwrap_or_else(|| "Permission required".to_string()),
            description: session
                .last_message
                .clone()
                .unwrap_or_else(|| "Waiting for permission".to_string()),
        });
    }

    Ok(Some(messages))
}

fn parent_title(
    session: &Session,
    load_session: &mut impl FnMut(&str) -> Result<Option<Session>>,
) -> Result<Option<String>> {
    let Some(parent_id) = session.ancestor_session_ids.last() else {
        return Ok(None);
    };
    let Some(parent) = load_session(parent_id)? else {
        return Ok(None);
    };
    Ok(Some(
        parent
            .label
            .or_else(|| claude_sessions::get_session_title(&parent.cwd, &parent.session_id))
            .or_else(|| {
                parent
                    .cwd
                    .file_name()
                    .and_then(|name| name.to_str())
                    .map(str::to_owned)
            })
            .unwrap_or_else(|| "parent session".to_string()),
    ))
}

fn parse_session_transcript(
    path: &Path,
    engine: Engine,
    from: Option<&str>,
    pending_tool: Option<&str>,
) -> Result<Vec<ChatItem>> {
    let Some(reader) = open_transcript(path, engine == Engine::Codex)? else {
        return Ok(Vec::new());
    };
    match engine {
        Engine::Claude => Ok(parser::claude::parse(reader, from, pending_tool)?),
        Engine::Codex => Ok(parser::codex::parse(reader, from)?),
    }
}

fn open_transcript(path: &Path, allow_compressed: bool) -> Result<Option<Box<dyn BufRead + Send>>> {
    match File::open(path) {
        Ok(file) => return reader_for(file, path),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(error).with_context(|| format!("open transcript {}", path.display()));
        }
    }

    if !allow_compressed {
        return Ok(None);
    }
    let compressed_path = compressed_path(path);
    let file = match File::open(&compressed_path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("open transcript {}", compressed_path.display()));
        }
    };
    let decoder = zstd::stream::read::Decoder::new(file)
        .with_context(|| format!("decompress transcript {}", compressed_path.display()))?;
    Ok(Some(Box::new(BufReader::new(decoder))))
}

fn reader_for(file: File, path: &Path) -> Result<Option<Box<dyn BufRead + Send>>> {
    if path.extension().and_then(|extension| extension.to_str()) == Some("zst") {
        let decoder = zstd::stream::read::Decoder::new(file)
            .with_context(|| format!("decompress transcript {}", path.display()))?;
        return Ok(Some(Box::new(BufReader::new(decoder))));
    }
    Ok(Some(Box::new(BufReader::new(file))))
}

fn compressed_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(".zst");
    PathBuf::from(name)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ChatItem {
    User {
        text: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        from: Option<String>,
    },
    Assistant {
        text: String,
    },
    ToolSummary {
        summary: String,
    },
    Question {
        question: String,
        choices: Vec<String>,
    },
    Permission {
        tool: String,
        description: String,
    },
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use serde_json::json;

    use super::{ChatItem, load_messages_with};
    use crate::commands::agent::types::Session;

    fn stored_session() -> Session {
        serde_json::from_value(json!({
            "session_id": "fixture-session",
            "cwd": "/fixture/project",
            "status": "ended",
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z",
            "last_message": null,
        }))
        .unwrap()
    }

    #[test]
    fn missing_store_entry_has_no_messages_result() {
        assert_eq!(
            load_messages_with("fixture-unregistered-session", |_| Ok(None)).unwrap(),
            None
        );
    }

    #[test]
    fn pending_permission_without_transcript_uses_the_stored_request_details() {
        let mut session = stored_session();
        session.pending_permission_agent_ids = BTreeSet::from(["main".to_string()]);
        session.current_tool = Some("Bash(cargo test -p sample)".to_string());
        session.last_message = Some("Run the package tests".to_string());

        assert_eq!(
            load_messages_with("fixture-session", |id| {
                Ok((id == "fixture-session").then(|| session.clone()))
            })
            .unwrap(),
            Some(vec![ChatItem::Permission {
                tool: "Bash(cargo test -p sample)".to_string(),
                description: "Run the package tests".to_string(),
            }])
        );
    }
}
