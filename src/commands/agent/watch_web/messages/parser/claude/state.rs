use std::collections::{BTreeMap, HashSet};

use chrono::{DateTime, Utc};
use serde_json::{Map, Value};

use crate::commands::agent::store::PermissionRequestData;

use super::super::super::SkillSuggestion;
use super::super::queue::QueueReplayEvent;
use super::super::record_timestamp;
use super::background_tasks::{BackgroundTaskTracker, task_notification_text};
use super::events::{ClaudeEffect, TimedClaudeEvent};
use super::questions::{append_questions, parse_question_answers, tool_use_result_answers};
use super::transcript_branch::{BranchMembership, TranscriptNode, is_inactive_branch};
use super::user_input::{content_blocks, is_tool_result_only_user_record, user_content, user_text};

pub(super) fn is_attachment_record(record: &Value) -> bool {
    record.get("type").and_then(Value::as_str) == Some("attachment")
}

#[derive(Clone)]
pub(super) struct ClaudeParserState {
    pub(super) transcript_nodes: BTreeMap<String, TranscriptNode>,
    pub(super) latest_uuid: Option<String>,
    pub(super) tool_result_parent: Option<String>,
    pub(super) events: Vec<TimedClaudeEvent>,
    pub(super) queue_events: Vec<QueueReplayEvent>,
    pub(super) tool_uses: Vec<ToolUse>,
    pub(super) question_uses: HashSet<String>,
    pub(super) background_tasks: BackgroundTaskTracker,
    pub(super) skills: Vec<SkillSuggestion>,
}

impl ClaudeParserState {
    pub(super) fn new() -> Self {
        Self {
            transcript_nodes: BTreeMap::new(),
            latest_uuid: None,
            tool_result_parent: None,
            events: Vec::new(),
            queue_events: Vec::new(),
            tool_uses: Vec::new(),
            question_uses: HashSet::new(),
            background_tasks: BackgroundTaskTracker::default(),
            skills: Vec::new(),
        }
    }

    pub(super) fn process_record(&mut self, record: Value, now: DateTime<Utc>) {
        if let Some(attachment) = record.get("attachment")
            && attachment.get("type").and_then(Value::as_str) == Some("skill_listing")
        {
            self.skills =
                super::super::skill_suggestions::parse_claude_listing(attachment.get("content"));
        }

        let task_notification = task_notification_text(&record);
        let is_tool_result_only = is_tool_result_only_user_record(&record);
        let uuid = record.get("uuid").and_then(Value::as_str);
        let mut active_record = uuid.is_none();
        if let Some(uuid) = uuid {
            let membership = self.transcript_nodes.get(uuid).map(|node| node.membership);
            let bypass_branch_check = is_tool_result_only || task_notification.is_some();
            active_record = match membership {
                None => bypass_branch_check,
                Some(BranchMembership::Unknown) if !bypass_branch_check => {
                    !is_inactive_branch(uuid, &mut self.transcript_nodes)
                }
                Some(_) => true,
            };
        }

        if let Some(mut queue_event) = QueueReplayEvent::from_record(&record) {
            queue_event.rendered_in_transcript = active_record;
            self.queue_events.push(queue_event);
        }
        if !active_record {
            return;
        }

        if is_tool_result_only {
            self.tool_result_parent = record_parent_uuid(&record);
        } else if uuid.is_some() && !is_attachment_record(&record) {
            self.tool_result_parent = None;
        }

        let effects = self.record_effects(&record, task_notification.as_deref(), now);
        self.events.push(TimedClaudeEvent {
            timestamp: record_timestamp(&record),
            effects,
        });
    }

    fn record_effects(
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
                                .and_then(super::background_tasks::task_notification_envelope)
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
                        if self.question_uses.remove(id)
                            && let Some(answers) = match tool_use_result {
                                Some(result) => tool_use_result_answers(result),
                                None => block
                                    .get("content")
                                    .and_then(tool_result_text)
                                    .and_then(|text| parse_question_answers(&text)),
                            }
                        {
                            effects.push(ClaudeEffect::QuestionAnswers {
                                id: id.to_owned(),
                                answers,
                            });
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
                                .and_then(super::background_tasks::task_notification_envelope)
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
                                        self.question_uses.insert(id.clone());
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

pub(super) fn record_parent_uuid(record: &Value) -> Option<String> {
    match record.get("parentUuid") {
        Some(Value::String(parent)) => Some(parent.clone()),
        Some(Value::Null) => match record.get("logicalParentUuid") {
            None | Some(Value::Null) => None,
            Some(Value::String(parent)) => Some(parent.clone()),
            _ => None,
        },
        _ => None,
    }
}

#[derive(Clone)]
pub(super) struct ToolUse {
    pub(super) id: String,
    pub(super) name: String,
    pub(super) input: Option<Value>,
    pub(super) started_at: Option<DateTime<Utc>>,
}

impl ToolUse {
    pub(super) fn matches_permission_request(&self, permission: &PermissionRequestData) -> bool {
        permission.tool_name.as_deref() == Some(self.name.as_str())
            && self.input.as_ref() == Some(&permission.tool_input)
    }
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

fn tool_result_blocks(message: &Value) -> impl Iterator<Item = &Value> {
    message
        .get("content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|block| block.get("type").and_then(Value::as_str) == Some("tool_result"))
}

pub(super) fn tool_result_text(content: &Value) -> Option<String> {
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
