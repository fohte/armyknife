use chrono::{DateTime, Utc};
use serde_json::{Map, Value};

use crate::commands::agent::peer_message::task_notification_envelope;

use super::super::super::record_timestamp;
use super::super::events::ClaudeEffect;
use super::super::questions::{
    append_questions, is_question_cancelled, parse_question_answers, tool_use_result_answers,
};
use super::super::thinking;
use super::super::user_input::{content_blocks, user_content, user_text};
use super::tool_results::tool_result_blocks;
use super::{ClaudeParserState, ToolUse};

impl ClaudeParserState {
    pub(super) fn record_effects(
        &mut self,
        record: &Value,
        task_notification: Option<&str>,
        now: DateTime<Utc>,
    ) -> Vec<ClaudeEffect> {
        if let Some(text) = task_notification {
            self.background_tasks.finish_notification(text);
        }

        let mut effects = Vec::new();
        let timestamp = record_timestamp(record);
        match record.get("type").and_then(Value::as_str) {
            _ if record
                .get("attachment")
                .and_then(|attachment| attachment.get("type"))
                .and_then(Value::as_str)
                == Some("queued_command") =>
            {
                if let Some(prompt) = record
                    .get("attachment")
                    .and_then(|attachment| attachment.get("prompt"))
                {
                    let (text, images) = user_content(Some(prompt));
                    if !text.is_empty() || !images.is_empty() {
                        effects.push(ClaudeEffect::FlushTools);
                        effects.push(ClaudeEffect::Inbound {
                            record: classifier_record(record),
                            text: task_notification
                                .and_then(task_notification_display_text)
                                .unwrap_or(&text)
                                .to_owned(),
                            images,
                            use_user_marker: true,
                            simplify_slash_command: false,
                            rewind_id: record
                                .get("uuid")
                                .and_then(Value::as_str)
                                .map(str::to_owned),
                        });
                    }
                }
            }
            Some("user") => {
                if let Some(message) = record.get("message") {
                    let tool_use_result = record
                        .get("toolUseResult")
                        .filter(|result| !result.is_null());
                    for block in tool_result_blocks(message) {
                        let Some(id) = block.get("tool_use_id").and_then(Value::as_str) else {
                            continue;
                        };
                        if self.question_uses.remove(id).is_some() {
                            let answers = match tool_use_result {
                                Some(result) => tool_use_result_answers(result),
                                None => block
                                    .get("content")
                                    .and_then(tool_result_text)
                                    .and_then(|text| parse_question_answers(&text)),
                            };
                            if let Some(answers) = answers {
                                effects.push(ClaudeEffect::QuestionAnswers {
                                    id: id.to_owned(),
                                    answers,
                                });
                            } else if tool_result_failed(record, block)
                                || block
                                    .get("content")
                                    .and_then(tool_result_text)
                                    .is_some_and(|text| is_question_cancelled(&text))
                            {
                                effects.push(ClaudeEffect::QuestionCancelled { id: id.to_owned() });
                            }
                        }
                        if let Some(index) =
                            self.tool_uses.iter().position(|tool_use| tool_use.id == id)
                        {
                            let tool_use = self.tool_uses.remove(index);
                            let failed = tool_result_failed(record, block);
                            if tool_use.name == "SendUserFile" {
                                effects.push(ClaudeEffect::SendUserFile { tool_use, failed });
                            } else {
                                effects.push(ClaudeEffect::ToolAction {
                                    name: tool_use.name.clone(),
                                    input: tool_use.input.clone(),
                                    failed,
                                });
                                self.background_tasks
                                    .complete_tool_use(&tool_use, record, block, now);
                            }
                        }
                    }
                    let (text, images) = user_content(message.get("content"));
                    if !text.is_empty() || !images.is_empty() {
                        effects.push(ClaudeEffect::FlushTools);
                        effects.push(ClaudeEffect::Inbound {
                            record: classifier_record(record),
                            text: task_notification
                                .and_then(task_notification_display_text)
                                .unwrap_or(&text)
                                .to_owned(),
                            images,
                            use_user_marker: true,
                            simplify_slash_command: true,
                            rewind_id: record
                                .get("uuid")
                                .and_then(Value::as_str)
                                .map(str::to_owned),
                        });
                    }
                }
            }
            Some("system") => {
                effects.push(ClaudeEffect::FlushTools);
                effects.push(ClaudeEffect::Inbound {
                    record: classifier_record(record),
                    text: record.get("message").map(user_text).unwrap_or_default(),
                    images: Vec::new(),
                    use_user_marker: false,
                    simplify_slash_command: false,
                    rewind_id: None,
                });
            }
            Some("assistant") => {
                if let Some(content) = record
                    .get("message")
                    .and_then(|message| message.get("content"))
                {
                    for block in content_blocks(content) {
                        match block.get("type").and_then(Value::as_str) {
                            Some("thinking") => {
                                let Some(text) = block.get("thinking").and_then(Value::as_str)
                                else {
                                    continue;
                                };
                                if text.trim().is_empty() {
                                    continue;
                                }
                                let duration_ms =
                                    record.get("thinkingDurationMs").and_then(Value::as_u64);
                                let signature = block.get("signature").and_then(Value::as_str);
                                if signature.is_some_and(thinking::is_narration_signature) {
                                    effects.push(ClaudeEffect::Narration {
                                        text: text.to_owned(),
                                        duration_ms,
                                    });
                                } else {
                                    effects.push(ClaudeEffect::Thinking {
                                        text: text.to_owned(),
                                        duration_ms,
                                    });
                                }
                            }
                            Some("text") => {
                                if let Some(text) = block.get("text").and_then(Value::as_str) {
                                    effects.push(ClaudeEffect::FlushTools);
                                    effects.push(ClaudeEffect::AssistantText {
                                        text: text.to_owned(),
                                    });
                                }
                            }
                            Some("tool_use") => {
                                let name = block
                                    .get("name")
                                    .and_then(Value::as_str)
                                    .unwrap_or_default();
                                let input = block.get("input");
                                if name == "AskUserQuestion" {
                                    effects.push(ClaudeEffect::FlushTools);
                                    let mut questions = Vec::new();
                                    let id =
                                        block.get("id").and_then(Value::as_str).map(str::to_owned);
                                    append_questions(&mut questions, input);
                                    if let Some(id) = &id {
                                        self.question_uses.insert(
                                            id.clone(),
                                            record
                                                .get("uuid")
                                                .and_then(Value::as_str)
                                                .map(str::to_owned),
                                        );
                                    }
                                    effects.extend(questions.into_iter().map(|item| {
                                        ClaudeEffect::Question {
                                            id: id.clone(),
                                            item,
                                        }
                                    }));
                                } else if let Some(id) = block.get("id").and_then(Value::as_str) {
                                    self.background_tasks.stop(input, name);
                                    self.tool_uses.push(ToolUse {
                                        id: id.to_owned(),
                                        assistant_uuid: record
                                            .get("uuid")
                                            .and_then(Value::as_str)
                                            .map(str::to_owned),
                                        name: name.to_owned(),
                                        input: input.cloned(),
                                        started_at: timestamp,
                                    });
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
            _ => {}
        }
        effects
    }
}

fn task_notification_display_text(text: &str) -> Option<&str> {
    let envelope = task_notification_envelope(text)?;
    Some(
        if envelope
            .trailing_text
            .trim_start()
            .starts_with("<system-reminder>")
        {
            text
        } else {
            envelope.notification
        },
    )
}

fn classifier_record(record: &Value) -> Value {
    const FIELDS: &[&str] = &[
        "type",
        "origin",
        "sourceToolAssistantUUID",
        "toolUseResult",
        "isMeta",
        "isSidechain",
        "isCompactSummary",
        "subtype",
        "hookErrors",
        "preventedContinuation",
        "durationMs",
        "content",
        "is_error",
        "isError",
        "failed",
        "blocked",
        "exit_code",
    ];
    let mut projected = Map::new();
    for field in FIELDS {
        if let Some(value) = record.get(*field) {
            projected.insert((*field).to_owned(), value.clone());
        }
    }
    if let Some(attachment) = record.get("attachment") {
        let mut projected_attachment = Map::new();
        for field in ["origin", "isMeta", "isSidechain", "isCompactSummary"] {
            if let Some(value) = attachment.get(field) {
                projected_attachment.insert(field.to_owned(), value.clone());
            }
        }
        if !projected_attachment.is_empty() {
            projected.insert("attachment".to_owned(), Value::Object(projected_attachment));
        }
    }
    Value::Object(projected)
}

pub(in crate::commands::agent::watch_web::messages::parser::claude) fn tool_result_text(
    content: &Value,
) -> Option<String> {
    match content {
        Value::String(text) => Some(text.clone()),
        Value::Array(blocks) => Some(
            blocks
                .iter()
                .filter_map(|block| block.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n"),
        ),
        _ => None,
    }
}

fn tool_result_failed(record: &Value, block: &Value) -> bool {
    [Some(block), record.get("toolUseResult")]
        .into_iter()
        .flatten()
        .any(|result| {
            ["is_error", "isError"]
                .into_iter()
                .any(|key| result.get(key).and_then(Value::as_bool) == Some(true))
        })
}
