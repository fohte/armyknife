use std::fs::File;
use std::io::{BufRead, BufReader, Cursor, Read, Seek};
use std::path::Path;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde_json::Value;

use super::parser::cards::{CardKind, TimedCard};
use super::parser::claude;
use super::questions::question_item_from_input;
use super::subagent_reports;
use super::{ChatItem, RouteLoaders};
use crate::commands::agent::store::{PermissionDeniedOperation, PermissionRequestData};
use crate::commands::agent::types::{Engine, MAIN_THREAD_AGENT_KEY, Session};

pub(in crate::commands::agent::watch_web) const MAX_PERMISSION_COMMAND_BYTES: usize = 16 * 1024;

pub(super) struct PendingPermissionCards {
    pub(super) cards: Vec<TimedCard>,
    pub(super) questions: Vec<ChatItem>,
}

pub(super) fn permission_denied_cards(operations: &[PermissionDeniedOperation]) -> Vec<TimedCard> {
    operations
        .iter()
        .map(|operation| {
            TimedCard::new(
                CardKind::PermissionDenied,
                Some(operation.created_at),
                operation.seq,
                operation.resolved_at.is_none(),
                ChatItem::PermissionDenied {
                    seq: operation.seq,
                    tool_use_id: operation.tool_use_id.clone(),
                    tool: operation.tool_name.clone(),
                    input_summary: permission_denied_input_summary(
                        &operation.tool_name,
                        &operation.tool_input,
                    ),
                    command: permission_request_command(
                        Some(&operation.tool_name),
                        Some(&operation.tool_input),
                    ),
                    reason: operation.reason.clone(),
                    retryable: operation.has_classifier_verdict(),
                    resolved: operation.resolved_at.is_some(),
                },
            )
        })
        .collect()
}

pub(super) fn collect_pending_permission_cards(
    session_id: &str,
    session: &Session,
    main_permission_is_question: bool,
    main_permission_request_id: Option<&str>,
    main_permission_data: Option<&PermissionRequestData>,
    loaders: &RouteLoaders,
) -> Result<PendingPermissionCards> {
    let mut cards = Vec::new();
    let mut next_seq = 0;
    if session
        .pending_permission_agent_ids
        .contains(MAIN_THREAD_AGENT_KEY)
        && !main_permission_is_question
    {
        let command = main_permission_data.and_then(|data| {
            permission_request_command(data.tool_name.as_deref(), Some(&data.tool_input))
        });
        let tool = main_permission_data
            .and_then(|data| {
                if data.tool_name.as_deref() == Some("Bash") {
                    data.tool_name.clone()
                } else {
                    permission_request_pending_tool(data)
                }
            })
            .or_else(|| session.current_tool.clone())
            .unwrap_or_else(|| "Permission required".to_string());
        let description = main_permission_data
            .map(|data| {
                permission_request_description(
                    data.tool_name.as_deref(),
                    Some(&data.tool_input),
                    command.as_deref(),
                    "Waiting for permission",
                )
            })
            .unwrap_or_else(|| "Waiting for permission".to_string());
        cards.push(TimedCard::new(
            CardKind::Permission,
            None,
            next_seq,
            true,
            ChatItem::Permission {
                request_id: main_permission_request_id.map(str::to_owned),
                tool,
                description,
                command,
            },
        ));
        next_seq = next_seq.saturating_add(1);
    }

    let mut questions = Vec::new();
    for agent_id in session
        .pending_permission_agent_ids
        .iter()
        .filter(|agent_id| agent_id.as_str() != MAIN_THREAD_AGENT_KEY)
    {
        let Some(request_id) = session
            .pending_permission_request_ids
            .get(agent_id)
            .cloned()
        else {
            continue;
        };
        let permission_data = (loaders.permission_request_data)(session_id, &request_id)?;
        if permission_data
            .as_ref()
            .is_some_and(|data| data.tool_name.as_deref() == Some("AskUserQuestion"))
        {
            let Some(data) = permission_data.as_ref() else {
                continue;
            };
            if let Some(question) = question_item_from_input(Some(request_id), &data.tool_input) {
                questions.push(question);
            }
            continue;
        }
        let (matched_tool, created_at) = if session.engine == Engine::Claude
            && let Some(data) = permission_data.as_ref()
            && data.tool_name.is_some()
            && let Some(parent_transcript_path) = session.transcript_path.as_deref()
        {
            matching_subagent_permission_tool_use(parent_transcript_path, agent_id, data)
        } else {
            (false, None)
        };
        let tool = if matched_tool {
            permission_data
                .as_ref()
                .and_then(permission_request_pending_tool)
        } else {
            permission_data
                .as_ref()
                .and_then(|data| data.tool_name.clone())
        }
        .unwrap_or_else(|| "Permission required".to_string());
        let command = permission_data.as_ref().and_then(|data| {
            permission_request_command(data.tool_name.as_deref(), Some(&data.tool_input))
        });
        let description = permission_data
            .as_ref()
            .map(|data| {
                permission_request_description(
                    data.tool_name.as_deref(),
                    Some(&data.tool_input),
                    command.as_deref(),
                    "Waiting for permission approval.",
                )
            })
            .unwrap_or_else(|| "Waiting for permission approval.".to_string());
        cards.push(TimedCard::new(
            CardKind::Permission,
            created_at,
            next_seq,
            true,
            ChatItem::Permission {
                request_id: Some(request_id),
                tool: format!("Subagent {agent_id}: {tool}"),
                description,
                command,
            },
        ));
        next_seq = next_seq.saturating_add(1);
    }

    Ok(PendingPermissionCards { cards, questions })
}

fn permission_request_pending_tool(data: &PermissionRequestData) -> Option<String> {
    let tool_name = data.tool_name.as_deref()?;
    let detail = tool_input_detail(&data.tool_input);
    Some(match detail {
        Some(detail) => format!("{tool_name}({detail})"),
        None => tool_name.to_string(),
    })
}

fn matching_subagent_permission_tool_use(
    parent_transcript_path: &Path,
    agent_id: &str,
    permission: &PermissionRequestData,
) -> (bool, Option<DateTime<Utc>>) {
    let Some(path) = subagent_reports::transcript_path_for_agent(parent_transcript_path, agent_id)
    else {
        return (false, None);
    };
    let reader = match open_seekable_transcript(&path) {
        Ok(Some(reader)) => reader,
        Ok(None) => return (false, None),
        Err(error) => {
            tracing::warn!(
                path = %path.display(),
                %error,
                "failed to open subagent permission transcript"
            );
            return (false, None);
        }
    };
    match claude::matching_pending_tool_use(reader, permission) {
        Ok(Some(matched)) => (true, matched.started_at),
        Ok(None) => (false, None),
        Err(error) => {
            tracing::warn!(
                path = %path.display(),
                %error,
                "failed to match subagent permission transcript"
            );
            (false, None)
        }
    }
}

pub(in crate::commands::agent::watch_web) fn permission_request_command(
    tool_name: Option<&str>,
    tool_input: Option<&Value>,
) -> Option<String> {
    if tool_name != Some("Bash") {
        return None;
    }
    let command = tool_input
        .and_then(|input| input.get("command"))
        .and_then(Value::as_str)?;
    (command.len() <= MAX_PERMISSION_COMMAND_BYTES).then(|| command.to_string())
}

pub(in crate::commands::agent::watch_web) fn permission_request_command_summary(
    tool_input: Option<&Value>,
) -> Option<String> {
    let command = tool_input
        .and_then(|input| input.get("command"))
        .and_then(Value::as_str)?;
    let mut chars = command.chars();
    let preview = chars.by_ref().take(200).collect::<String>();
    Some(if chars.next().is_some() {
        format!("{preview}…")
    } else {
        preview
    })
}

pub(in crate::commands::agent::watch_web) fn permission_request_description(
    tool_name: Option<&str>,
    tool_input: Option<&Value>,
    command: Option<&str>,
    waiting_message: &str,
) -> String {
    if tool_name == Some("Bash")
        && command.is_none()
        && let Some(preview) = permission_request_command_summary(tool_input)
    {
        return format!("Command preview: {preview}");
    }
    waiting_message.to_string()
}

fn permission_denied_input_summary(tool_name: &str, input: &Value) -> String {
    let detail = match tool_name {
        "Bash" => input.get("command"),
        "Read" | "Write" | "Edit" | "NotebookEdit" => input.get("file_path"),
        "Grep" | "Glob" => input.get("pattern"),
        _ => None,
    }
    .and_then(Value::as_str)
    .map(str::to_string)
    .unwrap_or_else(|| serde_json::to_string(input).unwrap_or_else(|_| "{}".to_string()));
    detail.chars().take(200).collect()
}

fn tool_input_detail(tool_input: &Value) -> Option<&str> {
    ["command", "file_path", "pattern"]
        .into_iter()
        .find_map(|key| tool_input.get(key).and_then(Value::as_str))
}

trait SeekableBufRead: BufRead + Seek {}

impl<T: BufRead + Seek> SeekableBufRead for T {}

fn open_seekable_transcript(path: &Path) -> Result<Option<Box<dyn SeekableBufRead + Send>>> {
    match File::open(path) {
        Ok(file) if path.extension().and_then(|extension| extension.to_str()) == Some("zst") => {
            // Matching rewinds after scanning unresolved tool calls, so compressed input must be buffered.
            let mut decoder = zstd::stream::read::Decoder::new(file)
                .with_context(|| format!("decompress transcript {}", path.display()))?;
            let mut contents = Vec::new();
            decoder
                .read_to_end(&mut contents)
                .with_context(|| format!("decompress transcript {}", path.display()))?;
            let reader: Box<dyn SeekableBufRead + Send> = Box::new(Cursor::new(contents));
            Ok(Some(reader))
        }
        Ok(file) => Ok(Some(Box::new(BufReader::new(file)))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).with_context(|| format!("open transcript {}", path.display())),
    }
}
