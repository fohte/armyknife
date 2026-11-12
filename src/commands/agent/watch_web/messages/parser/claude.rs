use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{BufRead, Cursor, Read, Seek, SeekFrom, Write};
use std::path::Path;

use chrono::{DateTime, Utc};
use serde_json::Value;

mod background_tasks;
mod proposals;
mod questions;
mod transcript_branch;
mod user_input;

use super::super::BackgroundTask;
use super::super::ChatItem;
use super::super::ParentSession;
use super::super::SkillSuggestion;
use super::super::parser::todos::TodoTimeline;
use super::super::parser::{
    ToolSummary, append_classification_with_image_directory,
    append_record_with_user_marker_and_image_directory, append_text_with_image_directory,
    classifier::{self, MessageKind},
    record_timestamp, tool_target,
};
use super::super::proposals::ProposalItem;
use super::decision_position::{DecisionTimeline, insert_time_placed_decisions};
use crate::commands::agent::store::{
    AgentDecision, AgentTodo, WatchWebPeerOutMessage, WatchWebQueuedPrompt,
};
use background_tasks::{BackgroundTaskTracker, task_notification_envelope, task_notification_text};
use proposals::ProposalTimeline;
use questions::{
    append_questions, apply_question_answers, parse_question_answers, tool_use_result_answers,
};
use transcript_branch::{BranchMembership, TranscriptNode, is_inactive_branch, mark_active_branch};
use user_input::{
    content_blocks, is_tool_result_only_user_record, slash_command_text, user_content,
};

pub(in crate::commands::agent::watch_web::messages) type ParsedTranscript = (
    Vec<ChatItem>,
    Vec<BackgroundTask>,
    Vec<i64>,
    Vec<SkillSuggestion>,
);

#[cfg(test)]
fn parse(
    reader: impl BufRead + Seek,
    parent: Option<&ParentSession>,
    pending_tool: Option<&str>,
    user_marker: Option<&str>,
) -> std::io::Result<Vec<ChatItem>> {
    let image_directory = super::super::super::image_attachments::storage_directory().ok();
    parse_with_image_directory(
        reader,
        parent,
        pending_tool,
        user_marker,
        image_directory.as_deref(),
    )
}

#[cfg(test)]
fn parse_with_image_directory(
    reader: impl BufRead + Seek,
    parent: Option<&ParentSession>,
    pending_tool: Option<&str>,
    user_marker: Option<&str>,
    image_directory: Option<&Path>,
) -> std::io::Result<Vec<ChatItem>> {
    parse_with_background_tasks_and_image_directory(
        reader,
        parent,
        pending_tool,
        user_marker,
        Utc::now(),
        ParseContext {
            image_directory,
            queued_prompts: &[],
            peer_out_messages: &[],
            decisions: &[],
            proposals: &[],
            todos: &[],
        },
    )
    .map(|(messages, _, _, _)| messages)
}

#[cfg(test)]
fn parse_with_decisions_and_queued_prompts(
    reader: impl BufRead + Seek,
    parent: Option<&ParentSession>,
    pending_tool: Option<&str>,
    user_marker: Option<&str>,
    now: DateTime<Utc>,
    decisions: &[AgentDecision],
    queued_prompts: &[WatchWebQueuedPrompt],
) -> std::io::Result<(Vec<ChatItem>, Vec<BackgroundTask>, Vec<i64>)> {
    parse_with_queued_prompts_and_peer_out_messages_and_proposals(
        reader,
        parent,
        pending_tool,
        user_marker,
        now,
        ParserAdditions {
            decisions,
            queued_prompts,
            peer_out_messages: &[],
            proposals: &[],
            todos: &[],
        },
    )
}

#[cfg(test)]
fn parse_with_background_tasks_and_peer_out_messages(
    reader: impl BufRead + Seek,
    parent: Option<&ParentSession>,
    pending_tool: Option<&str>,
    user_marker: Option<&str>,
    now: DateTime<Utc>,
    peer_out_messages: &[WatchWebPeerOutMessage],
) -> std::io::Result<(Vec<ChatItem>, Vec<BackgroundTask>)> {
    parse_with_queued_prompts_and_peer_out_messages_and_proposals(
        reader,
        parent,
        pending_tool,
        user_marker,
        now,
        ParserAdditions {
            decisions: &[],
            queued_prompts: &[],
            peer_out_messages,
            proposals: &[],
            todos: &[],
        },
    )
    .map(|(messages, background_tasks, _)| (messages, background_tasks))
}

#[cfg(test)]
fn parse_with_queued_prompts_and_proposals(
    reader: impl BufRead + Seek,
    parent: Option<&ParentSession>,
    pending_tool: Option<&str>,
    user_marker: Option<&str>,
    now: DateTime<Utc>,
    queued_prompts: &[WatchWebQueuedPrompt],
    proposals: &[ProposalItem],
) -> std::io::Result<(Vec<ChatItem>, Vec<BackgroundTask>, Vec<i64>)> {
    parse_with_queued_prompts_and_peer_out_messages_and_proposals(
        reader,
        parent,
        pending_tool,
        user_marker,
        now,
        ParserAdditions {
            decisions: &[],
            queued_prompts,
            peer_out_messages: &[],
            proposals,
            todos: &[],
        },
    )
}

pub(in crate::commands::agent::watch_web::messages) fn parse_with_queued_prompts_and_peer_out_messages_and_proposals(
    reader: impl BufRead + Seek,
    parent: Option<&ParentSession>,
    pending_tool: Option<&str>,
    user_marker: Option<&str>,
    now: DateTime<Utc>,
    additions: ParserAdditions<'_>,
) -> std::io::Result<(Vec<ChatItem>, Vec<BackgroundTask>, Vec<i64>)> {
    parse_with_queued_prompts_and_peer_out_messages_and_proposals_and_skills(
        reader,
        parent,
        pending_tool,
        user_marker,
        now,
        additions,
    )
    .map(|(messages, tasks, resolved_ids, _)| (messages, tasks, resolved_ids))
}

pub(in crate::commands::agent::watch_web::messages) fn parse_with_queued_prompts_and_peer_out_messages_and_proposals_and_skills(
    reader: impl BufRead + Seek,
    parent: Option<&ParentSession>,
    pending_tool: Option<&str>,
    user_marker: Option<&str>,
    now: DateTime<Utc>,
    additions: ParserAdditions<'_>,
) -> std::io::Result<ParsedTranscript> {
    let image_directory = super::super::super::image_attachments::storage_directory().ok();
    parse_with_background_tasks_and_image_directory(
        reader,
        parent,
        pending_tool,
        user_marker,
        now,
        ParseContext {
            image_directory: image_directory.as_deref(),
            queued_prompts: additions.queued_prompts,
            peer_out_messages: additions.peer_out_messages,
            decisions: additions.decisions,
            proposals: additions.proposals,
            todos: additions.todos,
        },
    )
}

#[derive(Clone, Copy)]
pub(in crate::commands::agent::watch_web::messages) struct ParserAdditions<'a> {
    pub(in crate::commands::agent::watch_web::messages) decisions: &'a [AgentDecision],
    pub(in crate::commands::agent::watch_web::messages) queued_prompts:
        &'a [WatchWebQueuedPrompt],
    pub(in crate::commands::agent::watch_web::messages) peer_out_messages:
        &'a [WatchWebPeerOutMessage],
    pub(in crate::commands::agent::watch_web::messages) proposals: &'a [ProposalItem],
    pub(in crate::commands::agent::watch_web::messages) todos: &'a [AgentTodo],
}

struct ParseContext<'a> {
    image_directory: Option<&'a Path>,
    queued_prompts: &'a [WatchWebQueuedPrompt],
    peer_out_messages: &'a [WatchWebPeerOutMessage],
    decisions: &'a [AgentDecision],
    proposals: &'a [ProposalItem],
    todos: &'a [AgentTodo],
}

fn parse_with_background_tasks_and_image_directory(
    reader: impl BufRead + Seek,
    parent: Option<&ParentSession>,
    pending_tool: Option<&str>,
    user_marker: Option<&str>,
    now: DateTime<Utc>,
    context: ParseContext<'_>,
) -> std::io::Result<ParsedTranscript> {
    let parser = IncrementalClaudeParser::from_reader(
        reader,
        parent,
        pending_tool,
        user_marker,
        now,
        context,
    )?;
    match parser.snapshot(now)? {
        IncrementalClaudeSnapshot::Parsed(parsed) => Ok(parsed),
        IncrementalClaudeSnapshot::RestartRequired => Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "Claude transcript tail forks from the cached active branch",
        )),
    }
}

#[derive(Clone, PartialEq, Eq)]
struct TranscriptParserContext {
    parent: Option<ParentSession>,
    pending_tool: Option<String>,
    user_marker: Option<String>,
    image_directory: Option<std::path::PathBuf>,
    decisions: Vec<AgentDecision>,
    queued_prompts: Vec<WatchWebQueuedPrompt>,
    peer_out_messages: Vec<WatchWebPeerOutMessage>,
    proposals: Vec<ProposalItem>,
    todos: Vec<AgentTodo>,
}

#[derive(Clone)]
struct ClaudeParserState {
    transcript_nodes: BTreeMap<String, TranscriptNode>,
    latest_uuid: Option<String>,
    items: Vec<ChatItem>,
    tools: ToolSummary,
    tool_uses: Vec<ToolUse>,
    question_uses: HashMap<String, Vec<usize>>,
    matched_peer_out_message_ids: HashSet<i64>,
    background_tasks: BackgroundTaskTracker,
    decision_timeline: DecisionTimeline,
    proposal_timeline: ProposalTimeline,
    todo_timeline: TodoTimeline,
    queue: super::queue::Queue,
    skills: Vec<SkillSuggestion>,
}

impl ClaudeParserState {
    fn new(context: &ParseContext<'_>) -> Self {
        Self {
            transcript_nodes: BTreeMap::new(),
            latest_uuid: None,
            items: Vec::new(),
            tools: ToolSummary::default(),
            tool_uses: Vec::new(),
            question_uses: HashMap::new(),
            matched_peer_out_message_ids: HashSet::new(),
            background_tasks: BackgroundTaskTracker::default(),
            decision_timeline: DecisionTimeline::new(context.decisions),
            proposal_timeline: ProposalTimeline::new(context.proposals),
            todo_timeline: TodoTimeline::new(context.todos),
            queue: super::queue::Queue::new(context.queued_prompts),
            skills: Vec::new(),
        }
    }

    fn process_record(
        &mut self,
        record: Value,
        context: &TranscriptParserContext,
        now: DateTime<Utc>,
    ) {
        let ClaudeParserState {
            transcript_nodes,
            items,
            tools,
            tool_uses,
            question_uses,
            matched_peer_out_message_ids,
            background_tasks,
            decision_timeline,
            proposal_timeline,
            todo_timeline,
            queue,
            skills,
            ..
        } = self;
        let parent = context.parent.as_ref();
        let user_marker = context.user_marker.as_deref();
        let image_directory = context.image_directory.as_deref();

        if let Some(attachment) = record.get("attachment")
            && attachment.get("type").and_then(Value::as_str) == Some("skill_listing")
        {
            *skills = super::skill_suggestions::parse_claude_listing(attachment.get("content"));
        }
        let task_notification = task_notification_text(&record);
        if let Some(uuid) = record.get("uuid").and_then(Value::as_str) {
            let membership = transcript_nodes.get(uuid).map(|node| node.membership);
            let is_tool_result_only = is_tool_result_only_user_record(&record);
            if membership.is_none() && !is_tool_result_only && task_notification.is_none() {
                return;
            }
            if membership == Some(BranchMembership::Unknown)
                && !is_tool_result_only
                && task_notification.is_none()
                && is_inactive_branch(uuid, transcript_nodes)
            {
                return;
            }
        }
        insert_time_placed_decisions(record_timestamp(&record), decision_timeline, tools, items);
        if queued_prompt_delivery_is_on_active_branch(&record, transcript_nodes) {
            queue.observe_queued_prompt_delivery(&record, user_marker);
        }
        if let Some(text) = task_notification.as_deref() {
            background_tasks.finish_notification(text);
        }
        proposal_timeline.insert_before_record(record_timestamp(&record), items, tools);
        todo_timeline.insert_before_record(record_timestamp(&record), items, tools);
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
                        tools.flush_into(items);
                        let text = task_notification
                            .as_deref()
                            .and_then(task_notification_envelope)
                            .unwrap_or(&text)
                            .to_owned();
                        append_record_with_user_marker_and_image_directory(
                            items,
                            &record,
                            text,
                            user_marker,
                            images,
                            image_directory,
                            parent,
                        );
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
                        if let Some(indices) = question_uses.remove(id) {
                            let answers = match tool_use_result {
                                Some(result) => tool_use_result_answers(result),
                                None => block
                                    .get("content")
                                    .and_then(tool_result_text)
                                    .and_then(|text| parse_question_answers(&text)),
                            };
                            if let Some(answers) = answers {
                                apply_question_answers(items, &indices, &answers);
                            }
                        }
                        if let Some(index) = tool_uses
                            .iter()
                            .position(|tool_use: &ToolUse| tool_use.id == id)
                        {
                            let tool_use = tool_uses.remove(index);
                            if classifier::is_peer_notify_tool(
                                &tool_use.name,
                                tool_use.input.as_ref(),
                            ) {
                                let text = block
                                    .get("content")
                                    .and_then(tool_result_text)
                                    .unwrap_or_default();
                                if tool_result_failed(&record, block)
                                    || classifier::peer_notification_failed(&record, &text)
                                {
                                    append_classification_with_image_directory(
                                        items,
                                        classifier::failed_peer_notification(&text),
                                        Vec::new(),
                                        image_directory,
                                        parent,
                                        record_timestamp(&record),
                                    );
                                } else if let Some(mut peer) = classifier::peer_out_from_tool(
                                    &tool_use.name,
                                    tool_use.input.as_ref(),
                                    parent.map(|parent| parent.session_id.as_str()),
                                    Some(&text),
                                ) {
                                    let completed_at = record_timestamp(&record);
                                    let delivered =
                                        context.peer_out_messages.iter().find(|message| {
                                            !matched_peer_out_message_ids.contains(&message.id)
                                                && message.to_session_id == peer.session_id
                                                && tool_use.started_at.is_some_and(|started_at| {
                                                    started_at <= message.sent_at
                                                })
                                                && completed_at.is_some_and(|completed_at| {
                                                    message.sent_at <= completed_at
                                                })
                                        });
                                    if let Some(delivered) = delivered {
                                        peer.text.clone_from(&delivered.content);
                                        matched_peer_out_message_ids.insert(delivered.id);
                                    }
                                    super::append_peer_out(
                                        items,
                                        peer,
                                        delivered
                                            .map(|message| message.sent_at)
                                            .or_else(|| record_timestamp(&record)),
                                    );
                                }
                            } else {
                                tools.record(
                                    &tool_use.name,
                                    tool_use.input.as_ref(),
                                    tool_result_failed(&record, block),
                                );
                                background_tasks.complete_tool_use(&tool_use, &record, block, now);
                            }
                        }
                    }
                    let (text, images) = user_content(message.get("content"));
                    if !text.is_empty() || !images.is_empty() {
                        tools.flush_into(items);
                        let classification_text = task_notification
                            .as_deref()
                            .and_then(task_notification_envelope)
                            .unwrap_or(&text);
                        let classification =
                            classifier::classify_inbound(&record, classification_text, user_marker);
                        let classification = match classification {
                            MessageKind::User { text } => MessageKind::User {
                                text: slash_command_text(&text).unwrap_or(text),
                            },
                            classification => classification,
                        };
                        append_classification_with_image_directory(
                            items,
                            classification,
                            images,
                            image_directory,
                            parent,
                            record_timestamp(&record),
                        );
                    }
                }
            }
            Some("system") => {
                tools.flush_into(items);
                let text = record
                    .get("message")
                    .map(user_input::user_text)
                    .unwrap_or_default();
                append_record_with_user_marker_and_image_directory(
                    items,
                    &record,
                    text,
                    None,
                    Vec::new(),
                    image_directory,
                    parent,
                );
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
                                    tools.flush_into(items);
                                    append_text_with_image_directory(
                                        items,
                                        text.to_owned(),
                                        false,
                                        None,
                                        image_directory,
                                        None,
                                    );
                                }
                            }
                            Some("tool_use") => {
                                let name = block
                                    .get("name")
                                    .and_then(Value::as_str)
                                    .unwrap_or_default();
                                let input = block.get("input");
                                if classifier::is_peer_notify_tool(name, input) {
                                    tools.flush_into(items);
                                    if let Some(id) = block.get("id").and_then(Value::as_str) {
                                        tool_uses.push(ToolUse {
                                            id: id.to_owned(),
                                            name: name.to_owned(),
                                            input: input.cloned(),
                                            started_at: record_timestamp(&record),
                                        });
                                    }
                                } else if name == "AskUserQuestion" {
                                    tools.flush_into(items);
                                    if let Some(id) = block.get("id").and_then(Value::as_str) {
                                        let indices = append_questions(items, input);
                                        question_uses.insert(id.to_owned(), indices);
                                    } else {
                                        append_questions(items, input);
                                    }
                                } else if let Some(id) = block.get("id").and_then(Value::as_str) {
                                    background_tasks.stop(input, name);
                                    tool_uses.push(ToolUse {
                                        id: id.to_owned(),
                                        name: name.to_owned(),
                                        input: input.cloned(),
                                        started_at: record_timestamp(&record),
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
    }

    fn into_parsed(
        self,
        context: &TranscriptParserContext,
        now: DateTime<Utc>,
    ) -> ParsedTranscript {
        let mut items = self.items;
        let mut tools = self.tools;
        let pending_tool = context.pending_tool.as_deref();
        let parent = context.parent.as_ref();
        let user_marker = context.user_marker.as_deref();
        let image_directory = context.image_directory.as_deref();
        tools.flush_into(&mut items);
        // Permission state must be tied to an unanswered call ID, not a reused tool name.
        if let Some(tool_use) = pending_tool.and_then(|pending| {
            self.tool_uses.iter().rev().find(|tool_use| {
                matches_pending_tool(tool_use, pending)
                    && !classifier::is_peer_notify_tool(&tool_use.name, tool_use.input.as_ref())
            })
        }) {
            items.push(ChatItem::Permission {
                request_id: None,
                tool: tool_use.name.clone(),
                description: tool_detail(&tool_use.name, tool_use.input.as_ref()),
            });
        } else if let Some(tool_use) = self.tool_uses.last().filter(|tool_use| {
            !classifier::is_peer_notify_tool(&tool_use.name, tool_use.input.as_ref())
        }) {
            items.push(ChatItem::ToolInProgress {
                tool: tool_use.name.clone(),
                target: tool_target(&tool_use.name, tool_use.input.as_ref()),
            });
        }
        let resolved_queued_prompt_ids = self.queue.resolved_queued_prompt_ids();
        items.extend(
            self.queue
                .pending_items(parent, user_marker, image_directory),
        );
        self.decision_timeline.append_remaining(&mut items);
        let mut proposal_timeline = self.proposal_timeline;
        proposal_timeline.append_remaining(&mut items);
        let mut todo_timeline = self.todo_timeline;
        todo_timeline.append_remaining(&mut items);
        let background_tasks = self.background_tasks.into_api_tasks(now);
        (
            items,
            background_tasks,
            resolved_queued_prompt_ids,
            self.skills,
        )
    }
}

#[derive(Clone)]
pub(in crate::commands::agent::watch_web::messages) struct IncrementalClaudeParser {
    state: ClaudeParserState,
    context: TranscriptParserContext,
    pending_line: Vec<u8>,
    tail_from_initial_scan: bool,
    compressed_chunks: Vec<Vec<u8>>,
}

pub(in crate::commands::agent::watch_web::messages) enum IncrementalClaudeSnapshot {
    Parsed(ParsedTranscript),
    RestartRequired,
}

impl IncrementalClaudeParser {
    pub(in crate::commands::agent::watch_web::messages) fn new(
        reader: impl BufRead + Seek,
        parent: Option<&ParentSession>,
        pending_tool: Option<&str>,
        user_marker: Option<&str>,
        now: DateTime<Utc>,
        additions: ParserAdditions<'_>,
    ) -> std::io::Result<Self> {
        let image_directory = super::super::super::image_attachments::storage_directory().ok();
        Self::from_reader(
            reader,
            parent,
            pending_tool,
            user_marker,
            now,
            ParseContext {
                image_directory: image_directory.as_deref(),
                queued_prompts: additions.queued_prompts,
                peer_out_messages: additions.peer_out_messages,
                decisions: additions.decisions,
                proposals: additions.proposals,
                todos: additions.todos,
            },
        )
    }

    fn from_reader(
        mut reader: impl BufRead + Seek,
        parent: Option<&ParentSession>,
        pending_tool: Option<&str>,
        user_marker: Option<&str>,
        now: DateTime<Utc>,
        context: ParseContext<'_>,
    ) -> std::io::Result<Self> {
        let parser_context = TranscriptParserContext {
            parent: parent.cloned(),
            pending_tool: pending_tool.map(str::to_owned),
            user_marker: user_marker.map(str::to_owned),
            image_directory: context.image_directory.map(Path::to_path_buf),
            decisions: context.decisions.to_vec(),
            queued_prompts: context.queued_prompts.to_vec(),
            peer_out_messages: context.peer_out_messages.to_vec(),
            proposals: context.proposals.to_vec(),
            todos: context.todos.to_vec(),
        };
        let mut state = ClaudeParserState::new(&context);
        let mut latest_complete_uuid = None;
        let mut latest_uuid = None;
        let mut pending_line = Vec::new();
        let mut line = Vec::new();
        let mut encoder = zstd::stream::Encoder::new(Vec::new(), 1)?;
        loop {
            line.clear();
            let read = reader.read_until(b'\n', &mut line)?;
            if read == 0 {
                break;
            }
            encoder.write_all(&line)?;
            let is_complete = line.ends_with(b"\n");
            if let Ok(line_text) = std::str::from_utf8(&line)
                && let Ok(record) = serde_json::from_str::<Value>(line_text)
            {
                if is_complete {
                    state.queue.observe(&record);
                    latest_complete_uuid = record
                        .get("uuid")
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                        .or(latest_complete_uuid);
                }
                if let Some(uuid) = record.get("uuid").and_then(Value::as_str) {
                    latest_uuid = Some(uuid.to_owned());
                    state.transcript_nodes.insert(
                        uuid.to_owned(),
                        TranscriptNode {
                            parent_uuid: record_parent_uuid(&record),
                            membership: BranchMembership::Unknown,
                        },
                    );
                }
            }
            if !is_complete {
                pending_line = line.clone();
                break;
            }
        }
        mark_active_branch(&mut state.transcript_nodes, latest_uuid.as_deref());
        state.latest_uuid = latest_complete_uuid;
        reader.seek(SeekFrom::Start(0))?;
        for_each_complete_jsonl_record(&mut reader, |record| {
            state.process_record(record, &parser_context, now);
        })?;
        let compressed_transcript = encoder.finish()?;
        Ok(Self {
            state,
            context: parser_context,
            pending_line,
            tail_from_initial_scan: true,
            compressed_chunks: vec![compressed_transcript],
        })
    }

    pub(in crate::commands::agent::watch_web::messages) fn snapshot(
        &self,
        now: DateTime<Utc>,
    ) -> std::io::Result<IncrementalClaudeSnapshot> {
        self.snapshot_from_context(now, &self.context)
    }

    pub(in crate::commands::agent::watch_web::messages) fn snapshot_with(
        &mut self,
        now: DateTime<Utc>,
        parent: Option<&ParentSession>,
        pending_tool: Option<&str>,
        user_marker: Option<&str>,
        additions: ParserAdditions<'_>,
    ) -> std::io::Result<IncrementalClaudeSnapshot> {
        let context = TranscriptParserContext {
            parent: parent.cloned(),
            pending_tool: pending_tool.map(str::to_owned),
            user_marker: user_marker.map(str::to_owned),
            image_directory: self.context.image_directory.clone(),
            decisions: additions.decisions.to_vec(),
            queued_prompts: additions.queued_prompts.to_vec(),
            peer_out_messages: additions.peer_out_messages.to_vec(),
            proposals: additions.proposals.to_vec(),
            todos: additions.todos.to_vec(),
        };
        if context == self.context {
            return self.snapshot_from_context(now, &context);
        }

        let mut transcript = Vec::new();
        for chunk in &self.compressed_chunks {
            let mut decoder = zstd::stream::read::Decoder::new(Cursor::new(chunk))?;
            decoder.read_to_end(&mut transcript)?;
        }
        let parse_context = ParseContext {
            image_directory: context.image_directory.as_deref(),
            queued_prompts: &context.queued_prompts,
            peer_out_messages: &context.peer_out_messages,
            decisions: &context.decisions,
            proposals: &context.proposals,
            todos: &context.todos,
        };
        let rebased = Self::from_reader(
            Cursor::new(transcript),
            context.parent.as_ref(),
            context.pending_tool.as_deref(),
            context.user_marker.as_deref(),
            now,
            parse_context,
        )?;
        let snapshot = rebased.snapshot_from_context(now, &context)?;
        if matches!(snapshot, IncrementalClaudeSnapshot::Parsed(_)) {
            *self = rebased;
        }
        Ok(snapshot)
    }

    fn snapshot_from_context(
        &self,
        now: DateTime<Utc>,
        context: &TranscriptParserContext,
    ) -> std::io::Result<IncrementalClaudeSnapshot> {
        let mut state = self.state.clone();
        if !self.pending_line.is_empty()
            && let Ok(line) = std::str::from_utf8(&self.pending_line)
            && let Ok(record) = serde_json::from_str::<Value>(line)
        {
            if !self.tail_from_initial_scan && !is_active_branch_continuation(&state, &record) {
                return Ok(IncrementalClaudeSnapshot::RestartRequired);
            }
            state.queue.observe(&record);
            if let Some(uuid) = record.get("uuid").and_then(Value::as_str) {
                state.transcript_nodes.insert(
                    uuid.to_owned(),
                    TranscriptNode {
                        parent_uuid: record_parent_uuid(&record),
                        membership: BranchMembership::Unknown,
                    },
                );
                mark_active_branch(&mut state.transcript_nodes, Some(uuid));
            }
            state.process_record(record, context, now);
        }
        Ok(IncrementalClaudeSnapshot::Parsed(
            state.into_parsed(context, now),
        ))
    }

    pub(in crate::commands::agent::watch_web::messages) fn append_bytes(
        &mut self,
        suffix: &[u8],
        now: DateTime<Utc>,
    ) -> std::io::Result<IncrementalAppend> {
        if !suffix.is_empty() {
            self.compressed_chunks
                .push(zstd::stream::encode_all(Cursor::new(suffix), 1)?);
        }
        let mut bytes = std::mem::take(&mut self.pending_line);
        bytes.extend_from_slice(suffix);
        self.tail_from_initial_scan = false;
        let mut start = 0;
        while let Some(relative_end) = bytes[start..].iter().position(|byte| *byte == b'\n') {
            let end = start + relative_end + 1;
            let line = &bytes[start..end];
            if let Ok(line_text) = std::str::from_utf8(line)
                && let Ok(record) = serde_json::from_str::<Value>(line_text)
            {
                if !is_active_branch_continuation(&self.state, &record) {
                    self.pending_line = bytes[start..].to_vec();
                    return Ok(IncrementalAppend::RestartRequired);
                }
                self.state.queue.observe(&record);
                if let Some(uuid) = record.get("uuid").and_then(Value::as_str) {
                    self.state.transcript_nodes.insert(
                        uuid.to_owned(),
                        TranscriptNode {
                            parent_uuid: record_parent_uuid(&record),
                            membership: BranchMembership::Unknown,
                        },
                    );
                    mark_active_branch(&mut self.state.transcript_nodes, Some(uuid));
                    self.state.latest_uuid = Some(uuid.to_owned());
                }
                self.state.process_record(record, &self.context, now);
            }
            start = end;
        }
        self.pending_line = bytes[start..].to_vec();
        Ok(IncrementalAppend::Applied)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(in crate::commands::agent::watch_web::messages) enum IncrementalAppend {
    Applied,
    RestartRequired,
}

fn for_each_complete_jsonl_record(
    reader: &mut impl BufRead,
    mut visit: impl FnMut(Value),
) -> std::io::Result<()> {
    let mut line = Vec::new();
    loop {
        line.clear();
        match reader.read_until(b'\n', &mut line) {
            Ok(0) => break,
            Err(error) => return Err(error),
            Ok(_) if !line.ends_with(b"\n") => break,
            Ok(_) => {}
        }
        let Ok(line) = std::str::from_utf8(&line) else {
            continue;
        };
        if let Ok(record) = serde_json::from_str(line) {
            visit(record);
        }
    }
    Ok(())
}

fn record_parent_uuid(record: &Value) -> Option<String> {
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

fn is_active_branch_continuation(state: &ClaudeParserState, record: &Value) -> bool {
    if record.get("uuid").and_then(Value::as_str).is_none() {
        return true;
    }
    let parent_uuid = record_parent_uuid(record);
    parent_uuid.as_deref() == state.latest_uuid.as_deref()
}

fn queued_prompt_delivery_is_on_active_branch(
    record: &Value,
    transcript_nodes: &mut BTreeMap<String, TranscriptNode>,
) -> bool {
    if let Some(uuid) = record.get("uuid").and_then(Value::as_str) {
        return transcript_nodes
            .get(uuid)
            .is_some_and(|node| node.membership == BranchMembership::Active);
    }
    let Some(parent_uuid) = record
        .get("parentUuid")
        .and_then(Value::as_str)
        .or_else(|| record.get("logicalParentUuid").and_then(Value::as_str))
    else {
        return true;
    };
    let Some(node) = transcript_nodes.get(parent_uuid) else {
        return false;
    };
    node.membership == BranchMembership::Active
}

#[derive(Clone)]
struct ToolUse {
    id: String,
    name: String,
    input: Option<Value>,
    started_at: Option<DateTime<Utc>>,
}

fn tool_result_blocks(message: &Value) -> impl Iterator<Item = &Value> {
    message
        .get("content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|block| block.get("type").and_then(Value::as_str) == Some("tool_result"))
}

fn tool_result_text(content: &Value) -> Option<String> {
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

fn matches_pending_tool(tool_use: &ToolUse, pending_tool: &str) -> bool {
    let (name, detail) = pending_tool
        .split_once('(')
        .map_or((pending_tool, None), |(name, detail)| {
            (name, detail.strip_suffix(')'))
        });
    if tool_use.name != name {
        return false;
    }
    let Some(detail) = detail else {
        return true;
    };
    tool_use
        .input
        .as_ref()
        .and_then(super::super::permission_request_tool_detail)
        == Some(detail)
}

fn tool_detail(name: &str, input: Option<&Value>) -> String {
    let Some(target) = tool_target(name, input) else {
        return name.to_owned();
    };
    format!("{name}: {target}")
}

#[cfg(test)]
mod tests {
    use std::io::{self, BufRead, Cursor, Read, Seek, SeekFrom};

    use chrono::{DateTime, TimeZone, Utc};
    use indoc::{formatdoc, indoc};
    use rstest::rstest;
    use serde_json::json;

    use super::{
        IncrementalAppend, IncrementalClaudeParser, IncrementalClaudeSnapshot, ParseContext,
        ParserAdditions, ProposalItem, parse, parse_with_decisions_and_queued_prompts,
        parse_with_image_directory,
        parse_with_queued_prompts_and_peer_out_messages_and_proposals_and_skills,
        parse_with_queued_prompts_and_proposals,
    };
    use crate::commands::agent::store::{AgentDecision, AgentTodo};
    use crate::commands::agent::watch_web::messages::{
        ChatItem, ParentSession, PeerDirection, ProposalOption, QuestionChoice, SkillSuggestion,
        ToolOperation, UserContentPart,
    };

    fn fixed_time() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).single().unwrap()
    }

    fn incremental_parser(bytes: &[u8]) -> IncrementalClaudeParser {
        IncrementalClaudeParser::from_reader(
            Cursor::new(bytes),
            None,
            None,
            None,
            fixed_time(),
            ParseContext {
                image_directory: None,
                queued_prompts: &[],
                peer_out_messages: &[],
                decisions: &[],
                proposals: &[],
                todos: &[],
            },
        )
        .unwrap()
    }

    fn full_parse(bytes: &[u8]) -> super::ParsedTranscript {
        full_parse_with(
            bytes,
            ParserAdditions {
                decisions: &[],
                queued_prompts: &[],
                peer_out_messages: &[],
                proposals: &[],
                todos: &[],
            },
        )
    }

    fn full_parse_with(bytes: &[u8], additions: ParserAdditions<'_>) -> super::ParsedTranscript {
        parse_with_queued_prompts_and_peer_out_messages_and_proposals_and_skills(
            Cursor::new(bytes),
            None,
            None,
            None,
            fixed_time(),
            additions,
        )
        .unwrap()
    }

    fn parsed_snapshot(parser: &IncrementalClaudeParser) -> super::ParsedTranscript {
        match parser.snapshot(fixed_time()).unwrap() {
            IncrementalClaudeSnapshot::Parsed(parsed) => parsed,
            IncrementalClaudeSnapshot::RestartRequired => {
                panic!("the fixture continues the active transcript branch")
            }
        }
    }

    #[test]
    fn appended_records_match_a_full_parse() {
        let prefix = indoc! {r#"
            {"type":"user","uuid":"message-one","parentUuid":null,"timestamp":"2026-01-01T00:00:01Z","message":{"content":"A question."}}
        "#}
        .as_bytes();
        let suffix = indoc! {r#"
            {"type":"assistant","uuid":"message-two","parentUuid":"message-one","timestamp":"2026-01-01T00:00:02Z","message":{"content":[{"type":"text","text":"An answer."}]}}
        "#}
        .as_bytes();
        let mut parser = incremental_parser(prefix);

        assert_eq!(
            parser.append_bytes(suffix, fixed_time()).unwrap(),
            IncrementalAppend::Applied,
        );
        let actual = parsed_snapshot(&parser);
        let mut transcript = prefix.to_vec();
        transcript.extend_from_slice(suffix);
        assert_eq!(actual, full_parse(&transcript));
    }

    #[test]
    fn appended_partial_record_matches_a_full_parse() {
        let prefix = indoc! {r#"
            {"type":"user","uuid":"message-one","parentUuid":null,"timestamp":"2026-01-01T00:00:01Z","message":{"content":"A question."}}
        "#}
        .as_bytes();
        let first_part = br#"{"type":"assistant","uuid":"message-two","parentUuid":"message-one","timestamp":"2026-01-01T00:00:02Z","message":{"content":[{"type":"text","text":"An answer."}]}}"#;
        let second_part = b"\n";
        let mut parser = incremental_parser(prefix);
        parser
            .append_bytes(first_part, fixed_time())
            .expect("the unfinished JSON line is buffered");
        assert_eq!(
            parser.append_bytes(second_part, fixed_time()).unwrap(),
            IncrementalAppend::Applied,
        );
        let actual = parsed_snapshot(&parser);
        let mut transcript = prefix.to_vec();
        transcript.extend_from_slice(first_part);
        transcript.extend_from_slice(second_part);
        assert_eq!(actual, full_parse(&transcript));
    }

    #[test]
    fn appended_fork_requires_a_full_parse() {
        let prefix = indoc! {r#"
            {"type":"user","uuid":"message-one","parentUuid":null,"timestamp":"2026-01-01T00:00:01Z","message":{"content":"A question."}}
            {"type":"assistant","uuid":"message-two","parentUuid":"message-one","timestamp":"2026-01-01T00:00:02Z","message":{"content":[{"type":"text","text":"An answer."}]}}
        "#}
        .as_bytes();
        let branch = indoc! {r#"
            {"type":"assistant","uuid":"message-three","parentUuid":"message-one","timestamp":"2026-01-01T00:00:03Z","message":{"content":[{"type":"text","text":"A regenerated answer."}]}}
        "#}
        .as_bytes();
        let mut parser = incremental_parser(prefix);

        assert_eq!(
            parser.append_bytes(branch, fixed_time()).unwrap(),
            IncrementalAppend::RestartRequired,
        );
    }

    #[rstest]
    #[case::completed(true)]
    #[case::pending(false)]
    fn places_todos_at_their_creation_time(#[case] is_completed: bool) {
        let transcript = indoc! {r#"
            {"uuid":"fixture-before","parentUuid":null,"timestamp":"2026-01-01T00:00:00Z","type":"assistant","message":{"content":[{"type":"text","text":"before"}]}}
            {"uuid":"fixture-after","parentUuid":"fixture-before","timestamp":"2026-01-01T00:05:00Z","type":"assistant","message":{"content":[{"type":"text","text":"after"}]}}
        "#};
        let created_at = DateTime::parse_from_rfc3339("2026-01-01T00:03:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let todo = AgentTodo {
            session_id: "fixture-session".to_string(),
            seq: 1,
            title: "Review the sample report".to_string(),
            detail: Some("Open the sample summary.".to_string()),
            created_at,
            completed_at: is_completed.then(|| created_at + chrono::Duration::minutes(1)),
            comment: is_completed.then(|| "The sample is ready.".to_string()),
        };

        let (messages, _, _, _) =
            parse_with_queued_prompts_and_peer_out_messages_and_proposals_and_skills(
                Cursor::new(transcript),
                None,
                None,
                None,
                Utc::now(),
                ParserAdditions {
                    decisions: &[],
                    queued_prompts: &[],
                    peer_out_messages: &[],
                    proposals: &[],
                    todos: &[todo],
                },
            )
            .unwrap();

        assert_eq!(
            messages,
            vec![
                ChatItem::Assistant {
                    text: "before".to_string(),
                },
                ChatItem::Todo {
                    seq: 1,
                    title: "Review the sample report".to_string(),
                    detail: Some("Open the sample summary.".to_string()),
                    completed: is_completed,
                    comment: is_completed.then(|| "The sample is ready.".to_string()),
                },
                ChatItem::Assistant {
                    text: "after".to_string(),
                },
            ],
        );
    }

    #[test]
    fn overlay_changes_replay_the_cached_transcript() {
        let prefix = indoc! {r#"
            {"type":"user","uuid":"message-one","parentUuid":null,"timestamp":"2026-01-01T00:00:01Z","message":{"content":"A question."}}
        "#}
        .as_bytes();
        let suffix = indoc! {r#"
            {"type":"assistant","uuid":"message-two","parentUuid":"message-one","timestamp":"2026-01-01T00:00:02Z","message":{"content":[{"type":"text","text":"An answer."}]}}
        "#}
        .as_bytes();
        let decision = AgentDecision {
            session_id: "fixture-session".to_string(),
            seq: 1,
            title: "Choose a fixture option".to_string(),
            why: "The fixture requires a choice.".to_string(),
            considered: vec!["Option A".to_string()],
            easy_to_change: "The fixture choice can be changed.".to_string(),
            created_at: "2026-01-01T00:00:01.500Z".to_string(),
            review: None,
            reviewed_at: None,
        };
        let todo = AgentTodo {
            session_id: "fixture-session".to_string(),
            seq: 1,
            title: "Review the sample report".to_string(),
            detail: None,
            created_at: DateTime::parse_from_rfc3339("2026-01-01T00:00:01.500Z")
                .unwrap()
                .with_timezone(&Utc),
            completed_at: None,
            comment: None,
        };
        let mut parser = incremental_parser(prefix);
        assert_eq!(
            parser.append_bytes(suffix, fixed_time()).unwrap(),
            IncrementalAppend::Applied,
        );
        let mut transcript = prefix.to_vec();
        transcript.extend_from_slice(suffix);
        let additions = ParserAdditions {
            decisions: std::slice::from_ref(&decision),
            queued_prompts: &[],
            peer_out_messages: &[],
            proposals: &[],
            todos: std::slice::from_ref(&todo),
        };

        let actual = match parser
            .snapshot_with(fixed_time(), None, None, None, additions)
            .unwrap()
        {
            IncrementalClaudeSnapshot::Parsed(parsed) => parsed,
            IncrementalClaudeSnapshot::RestartRequired => {
                panic!("the transcript remains on the active branch")
            }
        };
        assert_eq!(actual, full_parse_with(&transcript, additions));
    }

    #[test]
    fn uses_the_latest_skill_listing_from_the_transcript() {
        let transcript = indoc! {r#"
            {"attachment":{"type":"skill_listing","content":"- legacy-tool: An older entry."}}
            {"attachment":{"type":"skill_listing","content":"- design-flow: Help shape a design.\n- debug-flow: Investigate a failure.\n- example:review-tool: Review generated examples.\n- bare-tool\n- example:init"}}
        "#};
        let (_, _, _, skills) =
            parse_with_queued_prompts_and_peer_out_messages_and_proposals_and_skills(
                Cursor::new(transcript),
                None,
                None,
                None,
                Utc::now(),
                ParserAdditions {
                    decisions: &[],
                    queued_prompts: &[],
                    peer_out_messages: &[],
                    proposals: &[],
                    todos: &[],
                },
            )
            .unwrap();

        assert_eq!(
            skills,
            vec![
                SkillSuggestion {
                    name: "design-flow".to_string(),
                    description: "Help shape a design.".to_string(),
                },
                SkillSuggestion {
                    name: "debug-flow".to_string(),
                    description: "Investigate a failure.".to_string(),
                },
                SkillSuggestion {
                    name: "example:review-tool".to_string(),
                    description: "Review generated examples.".to_string(),
                },
                SkillSuggestion {
                    name: "bare-tool".to_string(),
                    description: String::new(),
                },
                SkillSuggestion {
                    name: "example:init".to_string(),
                    description: String::new(),
                },
            ],
        );
    }

    #[test]
    fn places_a_proposal_by_creation_time_and_keeps_its_bash_summary() {
        let transcript = [
            json!({
                "uuid": "fixture-assistant-before",
                "parentUuid": null,
                "timestamp": "2026-06-01T12:00:00Z",
                "type": "assistant",
                "message": {"content": [
                    {"type": "text", "text": "before"},
                    {"type": "tool_use", "id": "fixture-proposal-call", "name": "Bash", "input": {"command": "a agent proposal add"}}
                ]},
            }),
            json!({
                "uuid": "fixture-proposal-result",
                "parentUuid": "fixture-assistant-before",
                "timestamp": "2026-06-01T12:02:00Z",
                "type": "user",
                "message": {"content": [{
                    "type": "tool_result",
                    "tool_use_id": "fixture-proposal-call",
                    "content": "Recorded proposal 1",
                }]},
            }),
            json!({
                "uuid": "fixture-assistant-after",
                "parentUuid": "fixture-proposal-result",
                "timestamp": "2026-06-01T13:00:00Z",
                "type": "assistant",
                "message": {"content": [{"type": "text", "text": "after"}]},
            }),
        ]
        .iter()
        .map(serde_json::to_string)
        .collect::<serde_json::Result<Vec<_>>>()
        .unwrap()
        .join("\n");
        let proposal = ChatItem::Proposal {
            seq: 1,
            title: "Sample proposal".to_string(),
            summary: Some("Context".to_string()),
            options: vec![ProposalOption {
                label: "Option A".to_string(),
                description: None,
                pros: Vec::new(),
                cons: Vec::new(),
                recommended: false,
            }],
            answer: None,
            resolved: false,
        };

        let (messages, background_tasks, queued_prompt_ids) =
            parse_with_queued_prompts_and_proposals(
                Cursor::new(transcript),
                None,
                None,
                None,
                Utc::now(),
                &[],
                &[ProposalItem {
                    seq: 1,
                    created_at: DateTime::parse_from_rfc3339("2026-06-01T12:30:00Z")
                        .unwrap()
                        .with_timezone(&Utc),
                    item: proposal.clone(),
                }],
            )
            .unwrap();

        assert_eq!(
            (messages, background_tasks, queued_prompt_ids),
            (
                vec![
                    ChatItem::Assistant {
                        text: "before".to_string(),
                    },
                    ChatItem::ToolSummary {
                        summary: "Bash ×1".to_string(),
                        operations: vec![ToolOperation {
                            tool: "Bash".to_string(),
                            target: Some("a agent proposal add".to_string()),
                            failed: false,
                        }],
                    },
                    proposal,
                    ChatItem::Assistant {
                        text: "after".to_string(),
                    },
                ],
                Vec::new(),
                Vec::new(),
            ),
        );
    }

    #[test]
    fn unplaced_proposals_are_inserted_by_creation_time_before_later_recorded_proposals() {
        let transcript = [
            json!({
                "uuid": "fixture-assistant-before",
                "parentUuid": null,
                "timestamp": "2026-06-01T12:00:00Z",
                "type": "assistant",
                "message": {"content": [{"type": "text", "text": "before"}]},
            }),
            json!({
                "uuid": "fixture-assistant-middle",
                "parentUuid": "fixture-assistant-before",
                "timestamp": "2026-06-01T12:02:00Z",
                "type": "assistant",
                "message": {"content": [{"type": "text", "text": "middle"}]},
            }),
            json!({
                "uuid": "fixture-assistant-proposal",
                "parentUuid": "fixture-assistant-middle",
                "timestamp": "2026-06-01T12:03:00Z",
                "type": "assistant",
                "message": {"content": [{
                    "type": "tool_use",
                    "id": "fixture-proposal-call",
                    "name": "Bash",
                    "input": {"command": "a agent proposal add"},
                }]},
            }),
            json!({
                "uuid": "fixture-proposal-result",
                "parentUuid": "fixture-assistant-proposal",
                "timestamp": "2026-06-01T12:04:00Z",
                "type": "user",
                "message": {"content": [{
                    "type": "tool_result",
                    "tool_use_id": "fixture-proposal-call",
                    "content": "Recorded proposal 2",
                }]},
            }),
            json!({
                "uuid": "fixture-assistant-after",
                "parentUuid": "fixture-proposal-result",
                "timestamp": "2026-06-01T12:05:00Z",
                "type": "assistant",
                "message": {"content": [{"type": "text", "text": "after"}]},
            }),
        ]
        .iter()
        .map(serde_json::to_string)
        .collect::<serde_json::Result<Vec<_>>>()
        .unwrap()
        .join("\n");
        let created_at = |timestamp: &str| {
            DateTime::parse_from_rfc3339(timestamp)
                .unwrap()
                .with_timezone(&Utc)
        };
        let unplaced_proposal = ChatItem::Proposal {
            seq: 1,
            title: "Earlier proposal".to_string(),
            summary: None,
            options: vec![ProposalOption {
                label: "Option A".to_string(),
                description: None,
                pros: Vec::new(),
                cons: Vec::new(),
                recommended: false,
            }],
            answer: None,
            resolved: true,
        };
        let recorded_proposal = ChatItem::Proposal {
            seq: 2,
            title: "Later proposal".to_string(),
            summary: None,
            options: vec![ProposalOption {
                label: "Option B".to_string(),
                description: None,
                pros: Vec::new(),
                cons: Vec::new(),
                recommended: false,
            }],
            answer: None,
            resolved: false,
        };

        let (messages, background_tasks, queued_prompt_ids) =
            parse_with_queued_prompts_and_proposals(
                Cursor::new(transcript),
                None,
                None,
                None,
                Utc::now(),
                &[],
                &[
                    ProposalItem {
                        seq: 1,
                        created_at: created_at("2026-06-01T12:01:00Z"),
                        item: unplaced_proposal.clone(),
                    },
                    ProposalItem {
                        seq: 2,
                        created_at: created_at("2026-06-01T12:03:30Z"),
                        item: recorded_proposal.clone(),
                    },
                ],
            )
            .unwrap();

        assert_eq!(
            (messages, background_tasks, queued_prompt_ids),
            (
                vec![
                    ChatItem::Assistant {
                        text: "before".to_string(),
                    },
                    unplaced_proposal,
                    ChatItem::Assistant {
                        text: "middle".to_string(),
                    },
                    recorded_proposal,
                    ChatItem::ToolSummary {
                        summary: "Bash ×1".to_string(),
                        operations: vec![ToolOperation {
                            tool: "Bash".to_string(),
                            target: Some("a agent proposal add".to_string()),
                            failed: false,
                        }],
                    },
                    ChatItem::Assistant {
                        text: "after".to_string(),
                    },
                ],
                Vec::new(),
                Vec::new(),
            ),
        );
    }

    #[test]
    fn keeps_timed_proposals_in_creation_order_and_answers_later_questions() {
        let transcript = indoc! {r#"
            {"uuid":"fixture-before","parentUuid":null,"timestamp":"2026-06-01T12:00:00Z","type":"assistant","message":{"content":[{"type":"text","text":"before"}]}}
            {"uuid":"fixture-question","parentUuid":"fixture-before","timestamp":"2026-06-01T12:02:00Z","type":"assistant","message":{"content":[{"type":"tool_use","id":"fixture-question-call","name":"AskUserQuestion","input":{"questions":[{"question":"Which display mode?","header":"Display","multiSelect":false,"options":[{"label":"Option A"},{"label":"Option B"}]}]}}]}}
            {"uuid":"fixture-proposal-call","parentUuid":"fixture-question","timestamp":"2026-06-01T12:03:00Z","type":"assistant","message":{"content":[{"type":"tool_use","id":"fixture-proposal-call-id","name":"Bash","input":{"command":"a agent proposal add"}}]}}
            {"uuid":"fixture-proposal-result","parentUuid":"fixture-proposal-call","timestamp":"2026-06-01T12:04:00Z","type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"fixture-proposal-call-id","content":"Recorded proposal 1"}]}}
            {"uuid":"fixture-question-result","parentUuid":"fixture-proposal-result","timestamp":"2026-06-01T12:05:00Z","type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"fixture-question-call","content":"Your questions have been answered: \"Which display mode?\"=\"Option A\". You can now continue with these answers in mind."}]}}
            {"uuid":"fixture-after","parentUuid":"fixture-question-result","timestamp":"2026-06-01T12:06:00Z","type":"assistant","message":{"content":[{"type":"text","text":"after"}]}}
        "#};
        let proposal = ChatItem::Proposal {
            seq: 1,
            title: "Sample proposal".to_string(),
            summary: None,
            options: vec![ProposalOption {
                label: "Option A".to_string(),
                description: None,
                pros: Vec::new(),
                cons: Vec::new(),
                recommended: false,
            }],
            answer: None,
            resolved: false,
        };
        let proposal_item = ProposalItem {
            seq: 1,
            created_at: DateTime::parse_from_rfc3339("2026-06-01T12:01:00Z")
                .unwrap()
                .with_timezone(&Utc),
            item: proposal.clone(),
        };

        let (messages, background_tasks, queued_prompt_ids) =
            parse_with_queued_prompts_and_proposals(
                Cursor::new(transcript),
                None,
                None,
                None,
                Utc::now(),
                &[],
                &[proposal_item],
            )
            .unwrap();

        assert_eq!(
            (messages, background_tasks, queued_prompt_ids),
            (
                vec![
                    ChatItem::Assistant {
                        text: "before".to_string(),
                    },
                    proposal,
                    question_group(vec![super::super::super::QuestionPrompt {
                        header: Some("Display".to_string()),
                        question: "Which display mode?".to_string(),
                        choices: vec![question_choice("Option A"), question_choice("Option B")],
                        multi_select: false,
                        answer: Some("Option A".to_string()),
                    }]),
                    ChatItem::ToolSummary {
                        summary: "Bash ×1".to_string(),
                        operations: vec![ToolOperation {
                            tool: "Bash".to_string(),
                            target: Some("a agent proposal add".to_string()),
                            failed: false,
                        }],
                    },
                    ChatItem::Assistant {
                        text: "after".to_string(),
                    },
                ],
                Vec::new(),
                Vec::new(),
            ),
        );
    }

    const FAILED_ADD_TRANSCRIPT: &str = indoc! {r#"
        {"type":"assistant","message":{"content":[{"type":"tool_use","id":"fixture-decision-call","name":"Bash","input":{"command":"a agent decision add 'Decision 1'"}}]}}
        {"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"fixture-decision-call","is_error":true,"content":"could not record the decision"}]}}
    "#};
    const UNRECOGNIZED_ADD_TRANSCRIPT: &str = indoc! {r#"
        {"type":"assistant","message":{"content":[{"type":"tool_use","id":"fixture-decision-call","name":"Bash","input":{"command":"a agent decision add 'Decision 1'"}}]}}
        {"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"fixture-decision-call","content":"Recorded proposal 1"}]}}
    "#};

    fn decision(seq: i64) -> AgentDecision {
        decision_at(seq, "2026-11-11T00:00:00Z")
    }

    fn decision_at(seq: i64, created_at: &str) -> AgentDecision {
        AgentDecision {
            session_id: "fixture-session".to_string(),
            seq,
            title: format!("Decision {seq}"),
            why: "The fixture keeps the example reversible.".to_string(),
            considered: vec!["The other fixture option".to_string()],
            easy_to_change: "Change one fixture value.".to_string(),
            created_at: created_at.to_string(),
            review: None,
            reviewed_at: None,
        }
    }

    fn decision_item(seq: i64) -> ChatItem {
        decision_item_at(seq, "2026-11-11T00:00:00Z")
    }

    fn decision_item_at(seq: i64, created_at: &str) -> ChatItem {
        ChatItem::Decision {
            seq,
            title: format!("Decision {seq}"),
            why: "The fixture keeps the example reversible.".to_string(),
            considered: vec!["The other fixture option".to_string()],
            easy_to_change: "Change one fixture value.".to_string(),
            review: None,
            created_at: created_at.to_string(),
        }
    }

    #[rstest]
    #[case::current_output(
        indoc! {"
            Decision recorded for this session.
            Session-local sequence: 1
            To refer to it in another session or task, describe its contents; this sequence only works in this session.
        "}
    )]
    #[case::legacy_output("Recorded decision 1")]
    fn places_a_decision_by_creation_time_regardless_of_tool_output(#[case] result: &str) {
        let transcript = indoc! {r#"
            {"uuid":"fixture-before","parentUuid":null,"type":"assistant","timestamp":"2026-11-11T10:00:00Z","message":{"content":[{"type":"text","text":"Before the decision."}]}}
            {"uuid":"fixture-decision-call","parentUuid":"fixture-before","type":"assistant","timestamp":"2026-11-11T11:00:00Z","message":{"content":[{"type":"tool_use","id":"fixture-decision-call","name":"Bash","input":{"command":"a agent decision add 'Decision 1'"}}]}}
            {"uuid":"fixture-decision-result","parentUuid":"fixture-decision-call","type":"user","timestamp":"2026-11-11T12:00:00Z","message":{"content":[{"type":"tool_result","tool_use_id":"fixture-decision-call","content":"__RESULT__"}]}}
            {"uuid":"fixture-after","parentUuid":"fixture-decision-result","type":"assistant","timestamp":"2026-11-11T13:00:00Z","message":{"content":[{"type":"text","text":"After the decision."}]}}
        "#}
        .replace(
            "\"__RESULT__\"",
            &serde_json::to_string(result).expect("result should serialize"),
        );
        let decision = decision_at(1, "2026-11-11T12:30:00Z");

        assert_eq!(
            parse_with_decisions_and_queued_prompts(
                Cursor::new(transcript),
                None,
                None,
                None,
                Utc.timestamp_opt(1_000, 0).unwrap(),
                &[decision],
                &[],
            )
            .unwrap()
            .0,
            vec![
                ChatItem::Assistant {
                    text: "Before the decision.".to_string(),
                },
                ChatItem::ToolSummary {
                    summary: "Bash ×1".to_string(),
                    operations: vec![ToolOperation {
                        tool: "Bash".to_string(),
                        target: Some("a agent decision add 'Decision 1'".to_string()),
                        failed: false,
                    }],
                },
                decision_item_at(1, "2026-11-11T12:30:00Z"),
                ChatItem::Assistant {
                    text: "After the decision.".to_string(),
                },
            ],
        );
    }

    fn assert_unplaced_decisions(transcript: &str, failed: bool) {
        assert_eq!(
            parse_with_decisions_and_queued_prompts(
                Cursor::new(transcript),
                None,
                None,
                None,
                Utc.timestamp_opt(1_000, 0).unwrap(),
                &[decision(1), decision(2)],
                &[],
            )
            .unwrap()
            .0,
            vec![
                ChatItem::ToolSummary {
                    summary: if failed {
                        "Bash ×1 · 1 failed".to_string()
                    } else {
                        "Bash ×1".to_string()
                    },
                    operations: vec![ToolOperation {
                        tool: "Bash".to_string(),
                        target: Some("a agent decision add 'Decision 1'".to_string()),
                        failed,
                    }],
                },
                decision_item(1),
                decision_item(2),
            ],
        );
    }

    #[rstest]
    #[case::failed_add(FAILED_ADD_TRANSCRIPT, true)]
    #[case::unrecognized_result(UNRECOGNIZED_ADD_TRANSCRIPT, false)]
    fn appends_decisions_without_a_successful_recorded_result(
        #[case] transcript: &str,
        #[case] failed: bool,
    ) {
        assert_unplaced_decisions(transcript, failed);
    }

    #[test]
    fn preserves_timestamps_for_user_peer_and_event_records() {
        let transcript = indoc! {r#"
            {"type":"user","origin":{"kind":"human"},"timestamp":"2026-11-11T10:00:00Z","message":{"content":"Human request."}}
            {"type":"user","origin":{"kind":"peer","from":"fixture-peer"},"timestamp":"2026-11-11T10:01:00Z","message":{"content":"Peer request."}}
            {"type":"user","origin":{"kind":"armyknife"},"timestamp":"2026-11-11T10:02:00Z","message":{"content":"Moved a task."}}
            {"type":"assistant","timestamp":"2026-11-11T10:03:00Z","message":{"content":[{"type":"text","text":"Assistant response."}]}}
        "#};

        assert_eq!(
            parse(Cursor::new(transcript), None, None, None).unwrap(),
            vec![
                ChatItem::User {
                    text: "Human request.".to_string(),
                    from: None,
                    timestamp: Some("2026-11-11T10:00:00+00:00".to_string()),
                },
                ChatItem::Peer {
                    direction: PeerDirection::From,
                    session_id: "fixture-peer".to_string(),
                    session_title: None,
                    content: vec![UserContentPart::Text {
                        text: "Peer request.".to_string(),
                    }],
                    timestamp: Some("2026-11-11T10:01:00+00:00".to_string()),
                },
                ChatItem::Event {
                    kind: super::super::super::EventKind::Armyknife,
                    text: "Moved a task.".to_string(),
                    failed: false,
                    timestamp: Some("2026-11-11T10:02:00+00:00".to_string()),
                },
                ChatItem::Assistant {
                    text: "Assistant response.".to_string(),
                },
            ],
        );
    }

    #[test]
    fn places_unrecorded_decisions_before_the_first_later_transcript_record() {
        let transcript = indoc! {r#"
            {"uuid":"fixture-before","parentUuid":null,"type":"assistant","timestamp":"2026-11-11T10:00:00Z","message":{"content":[{"type":"text","text":"Before the decision."}]}}
            {"uuid":"fixture-after","parentUuid":"fixture-before","type":"assistant","timestamp":"2026-11-11T11:00:00Z","message":{"content":[{"type":"text","text":"After the decision."}]}}
        "#};
        let decision = decision_at(1, "2026-11-11T10:30:00Z");

        assert_eq!(
            parse_with_decisions_and_queued_prompts(
                Cursor::new(transcript),
                None,
                None,
                None,
                Utc.timestamp_opt(1_000, 0).unwrap(),
                &[decision],
                &[],
            )
            .unwrap()
            .0,
            vec![
                ChatItem::Assistant {
                    text: "Before the decision.".to_string(),
                },
                decision_item_at(1, "2026-11-11T10:30:00Z"),
                ChatItem::Assistant {
                    text: "After the decision.".to_string(),
                },
            ],
        );
    }

    #[test]
    fn appends_unrecorded_decisions_newer_than_every_transcript_record() {
        let transcript = indoc! {r#"
            {"uuid":"fixture-before","parentUuid":null,"type":"assistant","timestamp":"2026-11-11T10:00:00Z","message":{"content":[{"type":"text","text":"Earlier message."}]}}
        "#};
        let decision = decision_at(1, "2026-11-11T10:30:00Z");

        assert_eq!(
            parse_with_decisions_and_queued_prompts(
                Cursor::new(transcript),
                None,
                None,
                None,
                Utc.timestamp_opt(1_000, 0).unwrap(),
                &[decision],
                &[],
            )
            .unwrap()
            .0,
            vec![
                ChatItem::Assistant {
                    text: "Earlier message.".to_string(),
                },
                decision_item_at(1, "2026-11-11T10:30:00Z"),
            ],
        );
    }

    #[test]
    fn abandoned_decision_adds_do_not_count_as_active_transcript_positions() {
        let transcript = indoc! {r#"
            {"uuid":"fixture-root","parentUuid":null,"type":"user","timestamp":"2026-11-11T10:00:00Z","message":{"content":"Earlier message."}}
            {"uuid":"fixture-abandoned","parentUuid":"fixture-root","type":"assistant","timestamp":"2026-11-11T11:00:00Z","message":{"content":[{"type":"tool_use","id":"fixture-decision-call-a","name":"Bash","input":{"command":"a agent decision add 'Decision 1'"}},{"type":"tool_use","id":"fixture-decision-call-b","name":"Bash","input":{"command":"a agent decision add 'Decision 2'"}}]}}
            {"uuid":"fixture-abandoned-result","parentUuid":"fixture-abandoned","type":"user","timestamp":"2026-11-11T12:00:00Z","sourceToolAssistantUUID":"fixture-abandoned","message":{"content":[{"type":"tool_result","tool_use_id":"fixture-decision-call-a","content":"Recorded decision 1"},{"type":"tool_result","tool_use_id":"fixture-decision-call-b","content":"Recorded decision 2"}]}}
            {"uuid":"fixture-current","parentUuid":"fixture-root","type":"assistant","timestamp":"2026-11-11T13:00:00Z","message":{"content":[{"type":"text","text":"Current message."}]}}
        "#};
        let decisions = [
            decision_at(1, "2026-11-11T10:30:00Z"),
            decision_at(2, "2026-11-11T10:30:00Z"),
        ];

        assert_eq!(
            parse_with_decisions_and_queued_prompts(
                Cursor::new(transcript),
                None,
                None,
                None,
                Utc.timestamp_opt(1_000, 0).unwrap(),
                &decisions,
                &[],
            )
            .unwrap()
            .0,
            vec![
                ChatItem::User {
                    text: "Earlier message.".to_string(),
                    timestamp: Some("2026-11-11T10:00:00+00:00".to_string()),
                    from: None,
                },
                decision_item_at(1, "2026-11-11T10:30:00Z"),
                decision_item_at(2, "2026-11-11T10:30:00Z"),
                ChatItem::Assistant {
                    text: "Current message.".to_string(),
                },
            ],
        );
    }

    #[rstest]
    #[case::permission_requested(
        "a agent decision add 'Decision 1'",
        Some("Bash(a agent decision add 'Decision 1')"),
        ChatItem::Permission {
            request_id: None,
            tool: "Bash".to_string(),
            description: "Bash: a agent decision add 'Decision 1'".to_string(),
        }
    )]
    #[case::compound_bash_permission_requested(
        "cd ./fixture && a agent decision add 'Decision 1'",
        Some("Bash(cd ./fixture && a agent decision add 'Decision 1')"),
        ChatItem::Permission {
            request_id: None,
            tool: "Bash".to_string(),
            description: "Bash: cd ./fixture && a agent decision add 'Decision 1'".to_string(),
        }
    )]
    #[case::still_running(
        "a agent decision add 'Decision 1'",
        None,
        ChatItem::ToolInProgress {
            tool: "Bash".to_string(),
            target: Some("a agent decision add 'Decision 1'".to_string()),
        }
    )]
    fn pending_decision_add_keeps_normal_bash_status(
        #[case] decision_command: &str,
        #[case] pending_tool: Option<&str>,
        #[case] pending_item: ChatItem,
    ) {
        let transcript = [
            json!({
                "type": "assistant",
                "message": {"content": [{
                    "type": "tool_use",
                    "id": "fixture-completed-bash",
                    "name": "Bash",
                    "input": {"command": "echo prior work"},
                }]},
            })
            .to_string(),
            json!({
                "type": "user",
                "message": {"content": [{
                    "type": "tool_result",
                    "tool_use_id": "fixture-completed-bash",
                    "content": "prior result",
                }]},
            })
            .to_string(),
            json!({
                "type": "assistant",
                "message": {"content": [{
                    "type": "tool_use",
                    "id": "fixture-decision-call",
                    "name": "Bash",
                    "input": {"command": decision_command},
                }]},
            })
            .to_string(),
        ]
        .join("\n");

        assert_eq!(
            parse(Cursor::new(transcript), None, pending_tool, None).unwrap(),
            vec![
                ChatItem::ToolSummary {
                    summary: "Bash ×1".to_string(),
                    operations: vec![operation("Bash", Some("echo prior work"))],
                },
                pending_item,
            ],
        );
    }

    #[test]
    fn serializes_decision_fields_using_the_api_contract_names() {
        assert_eq!(
            serde_json::to_value(decision_item(1)).unwrap(),
            json!({
                "type": "decision",
                "seq": 1,
                "title": "Decision 1",
                "why": "The fixture keeps the example reversible.",
                "considered": ["The other fixture option"],
                "easyToChange": "Change one fixture value.",
                "review": null,
                "createdAt": "2026-11-11T00:00:00Z",
            }),
        );
    }

    fn question_choice(label: &str) -> QuestionChoice {
        QuestionChoice {
            label: label.to_string(),
            description: None,
        }
    }

    fn question_prompt(
        question: &str,
        choices: Vec<QuestionChoice>,
        multi_select: bool,
        answer: Option<&str>,
    ) -> super::super::super::QuestionPrompt {
        super::super::super::QuestionPrompt {
            header: None,
            question: question.to_string(),
            choices,
            multi_select,
            answer: answer.map(str::to_string),
        }
    }

    fn question_group(questions: Vec<super::super::super::QuestionPrompt>) -> ChatItem {
        ChatItem::Question {
            request_id: None,
            questions,
        }
    }

    #[test]
    fn parses_a_marked_peer_message_with_a_saved_image() {
        let directory = tempfile::tempdir().unwrap();
        let image_directory = directory.path().canonicalize().unwrap();
        let image_name = "00000000-0000-4000-8000-000000000001.png";
        let image_path = image_directory.join(image_name);
        std::fs::write(
            &image_path,
            [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A],
        )
        .unwrap();
        let marker = "⟦armyknife-user:00000000-0000-4000-8000-000000000002⟧";
        let content = formatdoc! {"
            Another Claude session sent a message:
            {marker}
            This message was typed by the human user in armyknife watch-web.
            Inspect this image.

            Attached image(s):
            - {}
        ", image_path.display()};
        let transcript = serde_json::to_string(&json!({
            "type": "user",
            "isMeta": true,
            "origin": {"kind": "peer", "from": "unknown"},
            "message": {"content": content},
        }))
        .unwrap();

        assert_eq!(
            parse_with_image_directory(
                Cursor::new(transcript),
                None,
                None,
                Some(marker),
                Some(&image_directory),
            )
            .unwrap(),
            vec![ChatItem::UserWithContent {
                content: vec![
                    UserContentPart::Text {
                        text: "Inspect this image.".to_string(),
                    },
                    UserContentPart::Image {
                        source: format!("/api/images/{image_name}"),
                        available: true,
                    },
                ],
                timestamp: None,
                from: None,
            }]
        );
    }

    #[test]
    fn pending_queued_message_resolves_saved_image_references() {
        let directory = tempfile::tempdir().unwrap();
        let image_directory = directory.path().canonicalize().unwrap();
        let image_name = "00000000-0000-4000-8000-000000000001.png";
        let image_path = image_directory.join(image_name);
        std::fs::write(
            &image_path,
            [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A],
        )
        .unwrap();
        let content = formatdoc! {"
            Inspect this image.

            Attached image(s):
            - {}
        ", image_path.display()};
        let transcript = serde_json::to_string(&json!({
            "type": "queue-operation",
            "operation": "enqueue",
            "content": content,
        }))
        .unwrap();

        assert_eq!(
            parse_with_image_directory(
                Cursor::new(transcript),
                None,
                None,
                None,
                Some(&image_directory),
            )
            .unwrap(),
            vec![ChatItem::QueuedUser {
                content: vec![
                    UserContentPart::Text {
                        text: "Inspect this image.".to_string(),
                    },
                    UserContentPart::Image {
                        source: format!("/api/images/{image_name}"),
                        available: true,
                    },
                ],
                timestamp: None,
                from: None,
            }],
        );
    }

    #[test]
    fn parses_an_unmarked_peer_message_with_a_saved_image() {
        let directory = tempfile::tempdir().unwrap();
        let image_directory = directory.path().canonicalize().unwrap();
        let image_name = "00000000-0000-4000-8000-000000000003.png";
        let image_path = image_directory.join(image_name);
        std::fs::write(
            &image_path,
            [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A],
        )
        .unwrap();
        let content = formatdoc! {"
            <peer-message>
            - From session_id: fixture-peer-session
            - Sender engine: fixture-engine

            Inspect this image.

            Attached image(s):
            - {}
            </peer-message>
        ", image_path.display()};
        let transcript = serde_json::to_string(&json!({
            "type": "user",
            "origin": {"kind": "peer", "from": "fixture-peer-session"},
            "message": {"content": content},
        }))
        .unwrap();

        assert_eq!(
            parse_with_image_directory(
                Cursor::new(transcript),
                None,
                None,
                None,
                Some(&image_directory),
            )
            .unwrap(),
            vec![ChatItem::Peer {
                direction: PeerDirection::From,
                session_id: "fixture-peer-session".to_string(),
                session_title: None,
                timestamp: None,
                content: vec![
                    UserContentPart::Text {
                        text: "Inspect this image.".to_string(),
                    },
                    UserContentPart::Image {
                        source: format!("/api/images/{image_name}"),
                        available: true,
                    },
                ],
            }]
        );
    }

    #[test]
    fn classifies_peer_messages_events_and_outgoing_notifications() {
        let transcript = indoc! {r#"
            {"attachment":{"type":"queued_command","prompt":"<peer-message>\n- From session_id: fixture-remote-session\n- Sender engine: fixture-engine\n\nReview the fictional output.\n</peer-message>"}}
            {"type":"user","isMeta":true,"message":{"content":"<delegation-update>Worker completed the fictional check.</delegation-update>"}}
            {"type":"user","message":{"content":"<task-notification>Background task failed while checking a fixture.</task-notification>"}}
            {"type":"user","message":{"content":"<local-command-stdout>Fictional hook output.\nSecond line.</local-command-stdout>"}}
            {"type":"user","message":{"content":"<future-notice>Unknown wrapped notice.</future-notice>"}}
            {"type":"user","message":{"content":"<system-reminder>Internal-only context.</system-reminder>"}}
            {"type":"system","subtype":"compact_boundary"}
            {"type":"assistant","message":{"content":[{"type":"tool_use","id":"fixture-peer-notify-call","name":"Bash","input":{"command":"a agent peer notify --message 'Please verify the fictional result.' $sid"}}]}}
            {"type":"user","sourceToolAssistantUUID":"fixture-assistant-call","toolUseResult":{},"message":{"content":[{"type":"tool_result","tool_use_id":"fixture-peer-notify-call","content":"Delivered to session fixture-resolved-session."}]}}
        "#};
        let parent = ParentSession {
            session_id: "fixture-parent-session".to_string(),
            title: "Fixture parent session".to_string(),
        };

        assert_eq!(
            parse(Cursor::new(transcript), Some(&parent), None, None).unwrap(),
            vec![
                ChatItem::Peer {
                    direction: PeerDirection::From,
                    session_id: "fixture-remote-session".to_string(),
                    session_title: None,
                    timestamp: None,
                    content: vec![UserContentPart::Text {
                        text: "Review the fictional output.".to_string(),
                    }],
                },
                ChatItem::Event {
                    kind: super::super::super::EventKind::Armyknife,
                    text: "Worker completed the fictional check.".to_string(),
                    timestamp: None,
                    failed: false,
                },
                ChatItem::Event {
                    kind: super::super::super::EventKind::System,
                    text: "Background task failed while checking a fixture.".to_string(),
                    timestamp: None,
                    failed: true,
                },
                ChatItem::Event {
                    kind: super::super::super::EventKind::Hook,
                    text: "Fictional hook output. Second line.".to_string(),
                    timestamp: None,
                    failed: false,
                },
                ChatItem::Event {
                    kind: super::super::super::EventKind::System,
                    text: "Unknown wrapped notice.".to_string(),
                    timestamp: None,
                    failed: false,
                },
                ChatItem::Event {
                    kind: super::super::super::EventKind::CompactBoundary,
                    text: "Context compacted".to_string(),
                    timestamp: None,
                    failed: false,
                },
                ChatItem::Peer {
                    direction: PeerDirection::To,
                    session_id: "fixture-resolved-session".to_string(),
                    session_title: None,
                    timestamp: None,
                    content: vec![UserContentPart::Text {
                        text: "Please verify the fictional result.".to_string(),
                    }],
                },
            ]
        );
    }

    #[test]
    fn serializes_compact_boundary_with_a_distinct_event_kind() {
        let transcript = indoc! {r#"
            {"type":"system","subtype":"compact_boundary"}
        "#};

        assert_eq!(
            serde_json::to_value(parse(Cursor::new(transcript), None, None, None).unwrap())
                .unwrap(),
            json!([{
                "type": "event",
                "kind": "compact_boundary",
                "text": "Context compacted",
                "failed": false,
            }]),
        );
    }

    #[test]
    fn hook_events_ignore_transcript_timestamps() {
        let transcript = indoc! {r#"
            {"type":"user","timestamp":"2026-11-11T10:00:00Z","message":{"content":"<local-command-stdout>Synthetic hook output.</local-command-stdout>"}}
        "#};

        assert_eq!(
            parse(Cursor::new(transcript), None, None, None).unwrap(),
            vec![ChatItem::Event {
                kind: super::super::super::EventKind::Hook,
                text: "Synthetic hook output.".to_string(),
                failed: false,
                timestamp: None,
            }],
        );
    }

    #[rstest]
    #[case::tool_error(
        true,
        "The fictional target was unavailable.\nPlease retry later.",
        "The fictional target was unavailable. Please retry later."
    )]
    #[case::saved_for_retry(
        false,
        "Failed to deliver to session fixture-parent-session: synthetic transport error. Saved the message; a agent sweep retries it for 24 hours.",
        "Failed to deliver to session fixture-parent-session: synthetic transport error. Saved the message; a agent sweep retries it for 24 hours."
    )]
    fn shows_failed_parent_peer_notifications_as_failed_events(
        #[case] is_error: bool,
        #[case] result_text: &str,
        #[case] expected_text: &str,
    ) {
        let tool_call = serde_json::to_string(&json!({
            "type": "assistant",
            "message": {"content": [{
                "type": "tool_use",
                "id": "fixture-peer-notify-call",
                "name": "Bash",
                "input": {"command": "a agent peer notify --parent -m 'Please verify the fictional result.'"},
            }]},
        }))
        .unwrap();
        let tool_result = serde_json::to_string(&json!({
            "type": "user",
            "toolUseResult": {},
            "message": {"content": [{
                "type": "tool_result",
                "tool_use_id": "fixture-peer-notify-call",
                "is_error": is_error,
                "content": result_text,
            }]},
        }))
        .unwrap();
        let transcript = format!("{tool_call}\n{tool_result}");
        let parent = ParentSession {
            session_id: "fixture-parent-session".to_string(),
            title: "Fixture parent session".to_string(),
        };

        assert_eq!(
            parse(Cursor::new(transcript), Some(&parent), None, None).unwrap(),
            vec![ChatItem::Event {
                kind: super::super::super::EventKind::Armyknife,
                text: expected_text.to_string(),
                timestamp: None,
                failed: true,
            }],
        );
    }

    #[test]
    fn agent_tool_summary_shows_its_type_and_description_without_its_prompt() {
        let transcript = indoc! {r#"
            {"type":"assistant","message":{"content":[{"type":"tool_use","id":"fixture-agent-call","name":"Agent","input":{"subagent_type":"Explore","description":"Find fixture loaders","prompt":"This full prompt stays hidden from the summary."}}]}}
            {"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"fixture-agent-call","content":"Fixture research result."}]}}
        "#};

        assert_eq!(
            parse(Cursor::new(transcript), None, None, None).unwrap(),
            vec![ChatItem::ToolSummary {
                summary: "Agent ×1".to_string(),
                operations: vec![operation("Agent", Some("Explore · Find fixture loaders"))],
            }],
        );
    }

    #[test]
    fn pending_agent_call_shows_its_type_and_description() {
        let transcript = indoc! {r#"
            {"type":"assistant","message":{"content":[{"type":"tool_use","id":"fixture-pending-agent-call","name":"Agent","input":{"subagent_type":"Explore","description":"Find fixture loaders","prompt":"This full prompt stays hidden from the summary."}}]}}
        "#};

        assert_eq!(
            parse(Cursor::new(transcript), None, None, None).unwrap(),
            vec![ChatItem::ToolInProgress {
                tool: "Agent".to_string(),
                target: Some("Explore · Find fixture loaders".to_string()),
            }],
        );
    }

    #[rstest]
    #[case::agent_tool("Agent")]
    #[case::task_tool("Task")]
    fn pending_agent_tool_uses_description_when_type_is_omitted(#[case] tool: &str) {
        let transcript = serde_json::json!({
            "type": "assistant",
            "message": {
                "content": [{
                    "type": "tool_use",
                    "id": "fixture-agent-without-type",
                    "name": tool,
                    "input": {
                        "description": "Find fixture loaders",
                        "prompt": "This full prompt stays hidden from the summary.",
                    },
                }],
            },
        })
        .to_string();

        assert_eq!(
            parse(Cursor::new(transcript), None, None, None).unwrap(),
            vec![ChatItem::ToolInProgress {
                tool: tool.to_string(),
                target: Some("Find fixture loaders".to_string()),
            }],
        );
    }

    #[rstest]
    #[case::prompt_array(
        indoc! {r#"
            {"type":"user","message":{"content":"Before the queued prompt."}}
            {"type":"assistant","message":{"content":[{"type":"tool_use","id":"fixture-tool-id","name":"Read","input":{}}]}}
            {"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"fixture-tool-id","content":"Synthetic result."}]}}
            {"type":"queue-operation","operation":"enqueue","content":"Show this prompt once."}
            {"parentUuid":"fixture-turn-id","attachment":{"type":"queued_command","prompt":[{"type":"text","text":"Show this prompt once."}]}}
            {"type":"queue-operation","operation":"remove","reason":"absorbed_mid_turn","content":"Show this prompt once."}
            {"type":"user","message":{"content":"After the queued prompt."}}
        "#},
        "Show this prompt once."
    )]
    #[case::prompt_string(
        indoc! {r#"
            {"type":"user","message":{"content":"Before the queued prompt."}}
            {"type":"assistant","message":{"content":[{"type":"tool_use","id":"fixture-tool-id","name":"Read","input":{}}]}}
            {"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"fixture-tool-id","content":"Synthetic result."}]}}
            {"type":"queue-operation","operation":"enqueue","content":"Show this prompt once."}
            {"parentUuid":"fixture-turn-id","attachment":{"type":"queued_command","prompt":"Show this prompt once."}}
            {"type":"queue-operation","operation":"remove","reason":"absorbed_mid_turn","content":"Show this prompt once."}
            {"type":"user","message":{"content":"After the queued prompt."}}
        "#},
        "Show this prompt once."
    )]
    fn queued_commands_are_shown_once_in_transcript_order(
        #[case] transcript: &str,
        #[case] queued_text: &str,
    ) {
        assert_eq!(
            parse(Cursor::new(transcript), None, None, None).unwrap(),
            vec![
                ChatItem::User {
                    text: "Before the queued prompt.".to_string(),
                    timestamp: None,
                    from: None,
                },
                ChatItem::ToolSummary {
                    summary: "Read ×1".to_string(),
                    operations: vec![operation("Read", None)],
                },
                ChatItem::User {
                    text: queued_text.to_string(),
                    timestamp: None,
                    from: None,
                },
                ChatItem::User {
                    text: "After the queued prompt.".to_string(),
                    timestamp: None,
                    from: None,
                },
            ]
        );
    }

    #[test]
    fn queued_peer_messages_keep_the_sender_session_id() {
        let transcript = indoc! {r#"
            {"parentUuid":"fixture-turn-id","attachment":{"type":"queued_command","prompt":[{"type":"text","text":"<peer-message>\n- From session_id: fixture-parent-id\n- Sender engine: fixture-engine\n\nPlease group these fictional events.\n</peer-message>"}]}}
        "#};
        let parent = ParentSession {
            session_id: "fixture-parent-id".to_string(),
            title: "fixture-parent-title".to_string(),
        };

        assert_eq!(
            parse(Cursor::new(transcript), Some(&parent), None, None).unwrap(),
            vec![ChatItem::Peer {
                direction: PeerDirection::From,
                session_id: "fixture-parent-id".to_string(),
                session_title: Some("fixture-parent-title".to_string()),
                timestamp: None,
                content: vec![UserContentPart::Text {
                    text: "Please group these fictional events.".to_string(),
                }],
            }]
        );
    }

    #[test]
    fn displays_base64_user_images_at_their_text_placeholders() {
        let marker = "⟦armyknife-user:fixture-marker-cobalt⟧";
        let transcript = indoc! {r#"
            {"type":"user","origin":{"kind":"human"},"message":{"content":[{"type":"text","text":"Before [Image #3] after."},{"type":"image","source":{"type":"base64","media_type":"image/png","data":"ZmFrZQ=="}}]}}
            {"attachment":{"type":"queued_command","isMeta":true,"origin":{"kind":"peer","from":"unknown"},"prompt":[{"type":"text","text":"⟦armyknife-user:fixture-marker-cobalt⟧\nThis message was typed by the human user in armyknife watch-web.\nQueued [Image #3] request."},{"type":"image","source":{"type":"base64","media_type":"image/jpeg","data":"c2FtcGxl"}}]}}
        "#};

        assert_eq!(
            parse(Cursor::new(transcript), None, None, Some(marker)).unwrap(),
            vec![
                ChatItem::UserWithContent {
                    content: vec![
                        UserContentPart::Text {
                            text: "Before ".to_string(),
                        },
                        UserContentPart::Image {
                            source: "data:image/png;base64,ZmFrZQ==".to_string(),
                            available: true,
                        },
                        UserContentPart::Text {
                            text: " after.".to_string(),
                        },
                    ],
                    timestamp: None,
                    from: None,
                },
                ChatItem::UserWithContent {
                    content: vec![
                        UserContentPart::Text {
                            text: "Queued ".to_string(),
                        },
                        UserContentPart::Image {
                            source: "data:image/jpeg;base64,c2FtcGxl".to_string(),
                            available: true,
                        },
                        UserContentPart::Text {
                            text: " request.".to_string(),
                        },
                    ],
                    timestamp: None,
                    from: None,
                },
            ],
        );
    }

    #[test]
    fn parses_messages_tools_questions_and_skips_unknown_rows() {
        let transcript = indoc! {r#"
            {"type":"user","message":{"content":"Review the notification flow."}}
            {"type":"assistant","message":{"content":[{"type":"text","text":"I will inspect the event handler."},{"type":"tool_use","id":"tool-1","name":"Read","input":{"file_path":"fixture/flow.rs"}},{"type":"tool_use","id":"tool-2","name":"Edit","input":{"file_path":"fixture/flow.rs"}}]}}
            {"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"tool-1","content":"fictional source text"}]}}
            {"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"tool-2","content":"fictional edit result"}]}}
            {"type":"assistant","message":{"content":[{"type":"tool_use","id":"tool-3","name":"AskUserQuestion","input":{"questions":[{"question":"Should notifications be grouped?","options":[{"label":"Yes","description":"Group them"},{"label":"No","description":"Show every event"}]}]}}]}}
            {"type":"user","message":{"content":"<peer-message>\n- From session_id: fixture-parent-id\n- Sender engine: fixture-engine\n\nGroup the events by source.\n</peer-message>"}}
            {"type":"user","message":{"content":"<peer-message>\n- From session_id: fixture-other-id\n\nThis came from another child.\n</peer-message>"}}
            {"type":"future_record","payload":{}}
            {not-json
        "#};

        let parent = ParentSession {
            session_id: "fixture-parent-id".to_string(),
            title: "fixture-parent-title".to_string(),
        };

        assert_eq!(
            parse(Cursor::new(transcript), Some(&parent), None, None).unwrap(),
            vec![
                ChatItem::User {
                    text: "Review the notification flow.".to_string(),
                    timestamp: None,
                    from: None,
                },
                ChatItem::Assistant {
                    text: "I will inspect the event handler.".to_string(),
                },
                ChatItem::ToolSummary {
                    summary: "Edit flow.rs  Read ×1".to_string(),
                    operations: vec![
                        operation("Read", Some("fixture/flow.rs")),
                        operation("Edit", Some("fixture/flow.rs")),
                    ],
                },
                question_group(vec![super::super::super::QuestionPrompt {
                    header: None,
                    question: "Should notifications be grouped?".to_string(),
                    choices: vec![
                        QuestionChoice {
                            label: "Yes".to_string(),
                            description: Some("Group them".to_string()),
                        },
                        QuestionChoice {
                            label: "No".to_string(),
                            description: Some("Show every event".to_string()),
                        },
                    ],
                    multi_select: false,
                    answer: None,
                }]),
                ChatItem::Peer {
                    direction: PeerDirection::From,
                    session_id: "fixture-parent-id".to_string(),
                    session_title: Some("fixture-parent-title".to_string()),
                    timestamp: None,
                    content: vec![UserContentPart::Text {
                        text: "Group the events by source.".to_string(),
                    }],
                },
                ChatItem::Peer {
                    direction: PeerDirection::From,
                    session_id: "fixture-other-id".to_string(),
                    session_title: None,
                    timestamp: None,
                    content: vec![UserContentPart::Text {
                        text: "This came from another child.".to_string(),
                    }],
                },
            ]
        );
    }

    #[test]
    fn preserves_each_tool_result_failure_on_its_operation() {
        let transcript = indoc! {r#"
            {"type":"assistant","message":{"content":[{"type":"tool_use","id":"fixture-read-call","name":"Read","input":{"file_path":"fixture/source.rs"}},{"type":"tool_use","id":"fixture-edit-call","name":"Edit","input":{"file_path":"fixture/change.rs"}}]}}
            {"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"fixture-read-call","is_error":true,"content":"fixture read failed"}]}}
            {"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"fixture-edit-call","content":"fixture edit succeeded"}]}}
        "#};

        assert_eq!(
            parse(Cursor::new(transcript), None, None, None).unwrap(),
            vec![ChatItem::ToolSummary {
                summary: "Edit change.rs  Read ×1 · 1 failed".to_string(),
                operations: vec![
                    ToolOperation {
                        tool: "Read".to_string(),
                        target: Some("fixture/source.rs".to_string()),
                        failed: true,
                    },
                    ToolOperation {
                        tool: "Edit".to_string(),
                        target: Some("fixture/change.rs".to_string()),
                        failed: false,
                    },
                ],
            }]
        );
    }

    #[rstest]
    #[case::one_question(
        indoc! {r#"
            {"type":"assistant","message":{"content":[{"type":"tool_use","id":"fixture-question-one","name":"AskUserQuestion","input":{"questions":[{"question":"Should the summary be brief?","options":[{"label":"Yes"},{"label":"No"}]}]}}]}}
            {"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"fixture-question-one","content":"Your questions have been answered: \"Should the summary be brief?\"=\"Yes\". You can now continue with these answers in mind."}]}}
        "#},
        vec![question_group(vec![question_prompt(
            "Should the summary be brief?",
            vec![question_choice("Yes"), question_choice("No")],
            false,
            Some("Yes"),
        )])]
    )]
    #[case::multiple_questions(
        indoc! {r#"
            {"type":"assistant","message":{"content":[{"type":"tool_use","id":"fixture-question-many","name":"AskUserQuestion","input":{"questions":[{"question":"Which output format?","options":[{"label":"Markdown"},{"label":"Plain text"}]},{"question":"Include examples?","options":[{"label":"Yes"},{"label":"No"}]}]}}]}}
            {"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"fixture-question-many","content":"Your questions have been answered: \"Which output format?\"=\"Markdown\", \"Include examples?\"=\"No\". You can now continue with these answers in mind."}]}}
        "#},
        vec![question_group(vec![
            question_prompt(
                "Which output format?",
                vec![question_choice("Markdown"), question_choice("Plain text")],
                false,
                Some("Markdown"),
            ),
            question_prompt(
                "Include examples?",
                vec![question_choice("Yes"), question_choice("No")],
                false,
                Some("No"),
            ),
        ])]
    )]
    #[case::answer_outside_choices(
        indoc! {r#"
            {"type":"assistant","message":{"content":[{"type":"tool_use","id":"fixture-question-custom","name":"AskUserQuestion","input":{"questions":[{"question":"How should the cache expire?","options":[{"label":"After one hour"},{"label":"After one day"}]}]}}]}}
            {"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"fixture-question-custom","content":"The user answered: \"How should the cache expire?\"=\"Keep it until deployment\". You can now continue with these answers in mind."}]}}
        "#},
        vec![question_group(vec![question_prompt(
            "How should the cache expire?",
            vec![question_choice("After one hour"), question_choice("After one day")],
            false,
            Some("Keep it until deployment"),
        )])]
    )]
    #[case::tool_use_result_answers(
        indoc! {r#"
            {"type":"assistant","message":{"content":[{"type":"tool_use","id":"fixture-question-structured","name":"AskUserQuestion","input":{"questions":[{"question":"Which color theme?","options":[{"label":"Light"},{"label":"Dark"}]}]}}]}}
            {"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"fixture-question-structured","content":"Your questions have been answered: \"Which color theme?\"=\"Dark\". You can now continue with these answers in mind."}]},"toolUseResult":{"questions":[{"question":"Which color theme?"}],"answers":{"Which color theme?":"Use a fictional palette"},"annotations":{}}}
        "#},
        vec![question_group(vec![question_prompt(
            "Which color theme?",
            vec![question_choice("Light"), question_choice("Dark")],
            false,
            Some("Use a fictional palette"),
        )])]
    )]
    #[case::null_tool_use_result_uses_text_answer(
        indoc! {r#"
            {"type":"assistant","message":{"content":[{"type":"tool_use","id":"fixture-question-null-result","name":"AskUserQuestion","input":{"questions":[{"question":"Should weekends be shown?","options":[{"label":"Yes"},{"label":"No"}]}]}}]}}
            {"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"fixture-question-null-result","content":"The user answered: \"Should weekends be shown?\"=\"No\". You can now continue with these answers in mind."}]},"toolUseResult":null}
        "#},
        vec![question_group(vec![question_prompt(
            "Should weekends be shown?",
            vec![question_choice("Yes"), question_choice("No")],
            false,
            Some("No"),
        )])]
    )]
    #[case::no_answer(
        indoc! {r#"
            {"type":"assistant","message":{"content":[{"type":"tool_use","id":"fixture-question-pending","name":"AskUserQuestion","input":{"questions":[{"question":"Should the report include a chart?","options":[{"label":"Yes"},{"label":"No"}]}]}}]}}
        "#},
        vec![question_group(vec![question_prompt(
            "Should the report include a chart?",
            vec![question_choice("Yes"), question_choice("No")],
            false,
            None,
        )])]
    )]
    #[case::answer_extraction_fails(
        indoc! {r#"
            {"type":"assistant","message":{"content":[{"type":"tool_use","id":"fixture-question-cancelled","name":"AskUserQuestion","input":{"questions":[{"question":"Should the report include a chart?","options":[{"label":"Yes"},{"label":"No"}]}]}}]}}
            {"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"fixture-question-cancelled","content":"The user cancelled this question."}]}}
        "#},
        vec![question_group(vec![question_prompt(
            "Should the report include a chart?",
            vec![question_choice("Yes"), question_choice("No")],
            false,
            None,
        )])]
    )]
    fn parses_question_answers_from_tool_results(
        #[case] transcript: &str,
        #[case] expected: Vec<ChatItem>,
    ) {
        assert_eq!(
            parse(Cursor::new(transcript), None, None, None).unwrap(),
            expected
        );
    }

    #[test]
    fn only_unanswered_tool_uses_matching_the_pending_request_become_permissions() {
        let transcript = indoc! {r#"
            {"type":"assistant","message":{"content":[{"type":"tool_use","id":"completed-bash","name":"Bash","input":{"command":"echo earlier"}}]}}
            {"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"completed-bash","content":"fictional command result"}]}}
            {"type":"assistant","message":{"content":[{"type":"tool_use","id":"pending-bash","name":"Bash","input":{"command":"echo awaiting approval"}}]}}
        "#};

        assert_eq!(
            parse(
                Cursor::new(transcript),
                None,
                Some("Bash(echo awaiting approval)"),
                None,
            )
            .unwrap(),
            vec![
                ChatItem::ToolSummary {
                    summary: "Bash ×1".to_string(),
                    operations: vec![operation("Bash", Some("echo earlier"))],
                },
                ChatItem::Permission {
                    request_id: None,
                    tool: "Bash".to_string(),
                    description: "Bash: echo awaiting approval".to_string(),
                },
            ]
        );
    }

    #[test]
    fn returns_the_last_unanswered_tool_call() {
        let transcript = indoc! {r#"
            {"type":"assistant","message":{"content":[{"type":"tool_use","id":"fixture-first-call","name":"Read","input":{"file_path":"fixture/old.rs"}},{"type":"tool_use","id":"fixture-last-call","name":"Bash","input":{"command":"cargo check"}}]}}
            {"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"fixture-first-call","content":"fixture result"}]}}
        "#};

        assert_eq!(
            parse(Cursor::new(transcript), None, None, None).unwrap(),
            vec![
                ChatItem::ToolSummary {
                    summary: "Read ×1".to_string(),
                    operations: vec![operation("Read", Some("fixture/old.rs"))],
                },
                ChatItem::ToolInProgress {
                    tool: "Bash".to_string(),
                    target: Some("cargo check".to_string()),
                },
            ]
        );
    }

    #[test]
    fn skips_claude_harness_rows_and_keeps_tool_results_for_action_counts() {
        let transcript = indoc! {r#"
            {"type":"user","isMeta":true,"message":{"content":"Synthetic hook metadata."}}
            {"type":"user","isSidechain":true,"message":{"content":"Synthetic sidechain note."}}
            {"type":"user","isCompactSummary":true,"message":{"content":"Synthetic compact summary."}}
            {"type":"user","message":{"content":"<task-notification>synthetic notification</task-notification>"}}
            {"type":"user","message":{"content":"<local-command-stdout>synthetic output</local-command-stdout>"}}
            {"type":"user","message":{"content":"<local-command-caveat>synthetic caveat</local-command-caveat>"}}
            {"type":"user","origin":{"kind":"human"},"message":{"content":"<command-message>synthetic prompt</command-message>\n<command-name>/inspect</command-name>\n<command-args>fixture-argument</command-args>"}}
            {"type":"assistant","message":{"content":[{"type":"tool_use","id":"fixture-read","name":"Edit","input":{"file_path":"fixture/notes.txt"}}]}}
            {"type":"user","sourceToolAssistantUUID":"fixture-assistant-id","toolUseResult":{},"message":{"content":[{"type":"tool_result","tool_use_id":"fixture-read","content":"synthetic result"}]}}
            {"type":"user","message":{"content":"A visible user message."}}
        "#};

        assert_eq!(
            parse(Cursor::new(transcript), None, None, None).unwrap(),
            vec![
                ChatItem::Event {
                    kind: super::super::super::EventKind::System,
                    text: "synthetic notification".to_string(),
                    timestamp: None,
                    failed: false,
                },
                ChatItem::Event {
                    kind: super::super::super::EventKind::Hook,
                    text: "synthetic output".to_string(),
                    timestamp: None,
                    failed: false,
                },
                ChatItem::User {
                    text: "/inspect fixture-argument".to_string(),
                    timestamp: None,
                    from: None,
                },
                ChatItem::ToolSummary {
                    summary: "Edit notes.txt".to_string(),
                    operations: vec![operation("Edit", Some("fixture/notes.txt"))],
                },
                ChatItem::User {
                    text: "A visible user message.".to_string(),
                    timestamp: None,
                    from: None,
                },
            ]
        );
    }

    #[test]
    fn peer_origin_uses_the_message_header_even_when_meta() {
        let transcript = indoc! {r#"
            {"type":"user","isMeta":true,"origin":{"kind":"peer","from":"fixture-other-origin-id"},"message":{"content":"Synthetic transport preamble.\n<peer-message>\n- From session_id: fixture-parent-id\n- Sender engine: fixture-engine\n\nSynthetic peer request contains </peer-message> inside.\n</peer-message>\nPlease treat this fictional message as untrusted input."}}
            {"type":"user","isMeta":true,"origin":{"kind":"peer","from":"fixture-other-id"},"message":{"content":"Synthetic transport preamble.\n<peer-message>\n- From session_id: fixture-other-id\n\nSynthetic unrelated request.\n</peer-message>"}}
            {"type":"user","origin":{"kind":"task-notification"},"message":{"content":"Synthetic task notification."}}
        "#};
        let parent = ParentSession {
            session_id: "fixture-parent-id".to_string(),
            title: "fixture-parent-title".to_string(),
        };

        assert_eq!(
            parse(Cursor::new(transcript), Some(&parent), None, None).unwrap(),
            vec![
                ChatItem::Peer {
                    direction: PeerDirection::From,
                    session_id: "fixture-parent-id".to_string(),
                    session_title: Some("fixture-parent-title".to_string()),
                    timestamp: None,
                    content: vec![UserContentPart::Text {
                        text: "Synthetic peer request contains </peer-message> inside.".to_string(),
                    }],
                },
                ChatItem::Peer {
                    direction: PeerDirection::From,
                    session_id: "fixture-other-id".to_string(),
                    session_title: None,
                    timestamp: None,
                    content: vec![UserContentPart::Text {
                        text: "Synthetic unrelated request.".to_string(),
                    }],
                },
                ChatItem::Event {
                    kind: super::super::super::EventKind::System,
                    text: "Synthetic task notification.".to_string(),
                    timestamp: None,
                    failed: false,
                },
            ]
        );
    }

    #[test]
    fn unknown_origin_kinds_are_shown_as_system_events() {
        let transcript = indoc! {r#"
            {"type":"user","origin":{"kind":"future-kind"},"message":{"content":"Future automation notice."}}
        "#};

        assert_eq!(
            parse(Cursor::new(transcript), None, None, None).unwrap(),
            vec![ChatItem::Event {
                kind: super::super::super::EventKind::System,
                text: "Future automation notice.".to_string(),
                timestamp: None,
                failed: false,
            }],
        );
    }

    #[test]
    fn shows_only_the_branch_containing_the_latest_user_message() {
        let transcript = indoc! {r#"
            {"uuid":"fixture-root-id","parentUuid":null,"type":"user","message":{"content":"Keep this earlier message."}}
            {"uuid":"fixture-cancelled-id","parentUuid":"fixture-root-id","type":"user","message":{"content":"Hide this cancelled message."}}
            {"type":"queue-operation","operation":"enqueue","content":"Show this queued text."}
            {"attachment":{"type":"queued_command","prompt":"Show this queued text."}}
            {"uuid":"fixture-active-id","parentUuid":"fixture-root-id","type":"user","message":{"content":"Keep this replacement message."}}
        "#};

        assert_eq!(
            parse(Cursor::new(transcript), None, None, None).unwrap(),
            vec![
                ChatItem::User {
                    text: "Keep this earlier message.".to_string(),
                    timestamp: None,
                    from: None,
                },
                ChatItem::User {
                    text: "Show this queued text.".to_string(),
                    timestamp: None,
                    from: None,
                },
                ChatItem::User {
                    text: "Keep this replacement message.".to_string(),
                    timestamp: None,
                    from: None,
                },
            ]
        );
    }

    #[test]
    fn keeps_parallel_tool_results_from_sibling_branches() {
        let transcript = indoc! {r#"
            {"uuid":"fixture-root-id","parentUuid":null,"type":"user","message":{"content":"Update the fixture."}}
            {"uuid":"fixture-tool-use-id","parentUuid":"fixture-root-id","type":"assistant","message":{"content":[{"type":"tool_use","id":"fixture-read-id","name":"Read","input":{"file_path":"fixture/plan.md"}},{"type":"tool_use","id":"fixture-edit-id","name":"Edit","input":{"file_path":"fixture/plan.md"}}]}}
            {"uuid":"fixture-read-result-id","parentUuid":"fixture-tool-use-id","type":"user","sourceToolAssistantUUID":"fixture-tool-use-id","toolUseResult":{},"message":{"content":[{"type":"tool_result","tool_use_id":"fixture-read-id","content":"Fictional source text."}]}}
            {"uuid":"fixture-edit-result-id","parentUuid":"fixture-tool-use-id","type":"user","sourceToolAssistantUUID":"fixture-tool-use-id","toolUseResult":{},"message":{"content":[{"type":"tool_result","tool_use_id":"fixture-edit-id","content":"Fictional edit result."}]}}
            {"uuid":"fixture-current-id","parentUuid":"fixture-tool-use-id","type":"assistant","message":{"content":[{"type":"text","text":"The fixture is updated."}]}}
        "#};

        assert_eq!(
            parse(Cursor::new(transcript), None, None, None).unwrap(),
            vec![
                ChatItem::User {
                    text: "Update the fixture.".to_string(),
                    timestamp: None,
                    from: None,
                },
                ChatItem::ToolSummary {
                    summary: "Edit plan.md  Read ×1".to_string(),
                    operations: vec![
                        operation("Read", Some("fixture/plan.md")),
                        operation("Edit", Some("fixture/plan.md")),
                    ],
                },
                ChatItem::Assistant {
                    text: "The fixture is updated.".to_string(),
                },
            ]
        );
    }

    #[test]
    fn follows_logical_parent_across_compact_boundaries() {
        let transcript = indoc! {r#"
            {"uuid":"fixture-before-compact-id","parentUuid":null,"type":"user","message":{"content":"Keep this pre-compact message."}}
            {"uuid":"fixture-compact-id","parentUuid":null,"logicalParentUuid":"fixture-before-compact-id","type":"system","subtype":"compact_boundary"}
            {"uuid":"fixture-after-compact-id","parentUuid":"fixture-compact-id","type":"user","message":{"content":"Keep this post-compact message."}}
        "#};

        assert_eq!(
            parse(Cursor::new(transcript), None, None, None).unwrap(),
            vec![
                ChatItem::User {
                    text: "Keep this pre-compact message.".to_string(),
                    timestamp: None,
                    from: None,
                },
                ChatItem::Event {
                    kind: super::super::super::EventKind::CompactBoundary,
                    text: "Context compacted".to_string(),
                    timestamp: None,
                    failed: false,
                },
                ChatItem::User {
                    text: "Keep this post-compact message.".to_string(),
                    timestamp: None,
                    from: None,
                },
            ]
        );
    }

    #[test]
    fn keeps_unconnected_history_when_compact_parent_is_missing() {
        let transcript = indoc! {r#"
            {"uuid":"fixture-earlier-history-id","parentUuid":null,"type":"user","message":{"content":"Keep the earlier history."}}
            {"uuid":"fixture-compact-boundary-id","parentUuid":null,"logicalParentUuid":"fixture-unrecorded-parent-id","type":"system","subtype":"compact_boundary"}
            {"uuid":"fixture-post-compact-context-id","parentUuid":"fixture-compact-boundary-id","type":"user","message":{"content":"Keep the post-compact context."}}
            {"uuid":"fixture-cancelled-id","parentUuid":"fixture-post-compact-context-id","type":"user","message":{"content":"Hide this cancelled message."}}
            {"uuid":"fixture-current-id","parentUuid":"fixture-post-compact-context-id","type":"user","message":{"content":"Keep this replacement message."}}
        "#};

        assert_eq!(
            parse(Cursor::new(transcript), None, None, None).unwrap(),
            vec![
                ChatItem::User {
                    text: "Keep the earlier history.".to_string(),
                    timestamp: None,
                    from: None,
                },
                ChatItem::Event {
                    kind: super::super::super::EventKind::CompactBoundary,
                    text: "Context compacted".to_string(),
                    timestamp: None,
                    failed: false,
                },
                ChatItem::User {
                    text: "Keep the post-compact context.".to_string(),
                    timestamp: None,
                    from: None,
                },
                ChatItem::User {
                    text: "Keep this replacement message.".to_string(),
                    timestamp: None,
                    from: None,
                },
            ]
        );
    }

    #[test]
    fn skips_uuid_rows_appended_after_branch_discovery() {
        let transcript = indoc! {r#"
            {"uuid":"fixture-root-id","parentUuid":null,"type":"user","message":{"content":"Keep this earlier message."}}
            {"uuid":"fixture-current-id","parentUuid":"fixture-root-id","type":"user","message":{"content":"Keep this current message."}}
        "#};
        let reader = AppendingReader {
            inner: Cursor::new(transcript.as_bytes().to_vec()),
            appended_row: Some(
                indoc! {r#"
                    {"uuid":"fixture-new-fork-id","parentUuid":"fixture-root-id","type":"user","message":{"content":"Hide this newly appended fork."}}
                "#}
                .as_bytes()
                .to_vec(),
            ),
        };

        assert_eq!(
            parse(reader, None, None, None).unwrap(),
            vec![
                ChatItem::User {
                    text: "Keep this earlier message.".to_string(),
                    timestamp: None,
                    from: None,
                },
                ChatItem::User {
                    text: "Keep this current message.".to_string(),
                    timestamp: None,
                    from: None,
                },
            ]
        );
    }

    #[test]
    fn keeps_tool_results_appended_after_branch_discovery() {
        let transcript = indoc! {r#"
            {"uuid":"fixture-root-id","parentUuid":null,"type":"user","message":{"content":"Read the plan."}}
            {"uuid":"fixture-tool-use-id","parentUuid":"fixture-root-id","type":"assistant","message":{"content":[{"type":"tool_use","id":"fixture-read-id","name":"Read","input":{"file_path":"fixture/plan.md"}}]}}
        "#};
        let reader = AppendingReader {
            inner: Cursor::new(transcript.as_bytes().to_vec()),
            appended_row: Some(
                indoc! {r#"
                    {"uuid":"fixture-result-id","parentUuid":"fixture-tool-use-id","type":"user","sourceToolAssistantUUID":"fixture-tool-use-id","toolUseResult":{},"message":{"content":[{"type":"tool_result","tool_use_id":"fixture-read-id","content":"Fictional delayed result."}]}}
                "#}
                .as_bytes()
                .to_vec(),
            ),
        };

        assert_eq!(
            parse(reader, None, None, None).unwrap(),
            vec![
                ChatItem::User {
                    text: "Read the plan.".to_string(),
                    timestamp: None,
                    from: None,
                },
                ChatItem::ToolSummary {
                    summary: "Read ×1".to_string(),
                    operations: vec![operation("Read", Some("fixture/plan.md"))],
                },
            ]
        );
    }

    #[test]
    fn shows_all_messages_when_transcript_rows_have_no_uuids() {
        let transcript = indoc! {r#"
            {"type":"user","message":{"content":"Keep the first legacy message."}}
            {"type":"user","message":{"content":"Keep the second legacy message."}}
        "#};

        assert_eq!(
            parse(Cursor::new(transcript), None, None, None).unwrap(),
            vec![
                ChatItem::User {
                    text: "Keep the first legacy message.".to_string(),
                    timestamp: None,
                    from: None,
                },
                ChatItem::User {
                    text: "Keep the second legacy message.".to_string(),
                    timestamp: None,
                    from: None,
                },
            ]
        );
    }

    #[test]
    fn keeps_the_reachable_branch_when_an_ancestor_is_missing() {
        let transcript = indoc! {r#"
            {"uuid":"fixture-root-id","parentUuid":null,"type":"user","message":{"content":"Keep the first legacy message."}}
            {"uuid":"fixture-unreachable-id","parentUuid":"fixture-missing-parent-id","type":"user","message":{"content":"Keep the second legacy message."}}
            {"uuid":"fixture-cancelled-id","parentUuid":"fixture-unreachable-id","type":"user","message":{"content":"Hide this cancelled message."}}
            {"uuid":"fixture-active-id","parentUuid":"fixture-unreachable-id","type":"user","message":{"content":"Keep this current message."}}
        "#};

        assert_eq!(
            parse(Cursor::new(transcript), None, None, None).unwrap(),
            vec![
                ChatItem::User {
                    text: "Keep the first legacy message.".to_string(),
                    timestamp: None,
                    from: None,
                },
                ChatItem::User {
                    text: "Keep the second legacy message.".to_string(),
                    timestamp: None,
                    from: None,
                },
                ChatItem::User {
                    text: "Keep this current message.".to_string(),
                    timestamp: None,
                    from: None,
                },
            ]
        );
    }

    #[test]
    fn peer_markers_control_user_attribution_in_both_delivery_paths() {
        let marker = "⟦armyknife-user:fixture-marker-cobalt⟧";
        let transcript = indoc! {r#"
            {"type":"user","isMeta":true,"origin":{"kind":"peer","from":"unknown"},"message":{"content":"Another peer session sent a message:\n⟦armyknife-user:fixture-marker-cobalt⟧\nThis message was typed by the human user in armyknife watch-web.\nDirect user request.\n\nThis came from another Claude session — this fictional transport notice must be hidden."}}
            {"attachment":{"type":"queued_command","isMeta":true,"origin":{"kind":"peer","from":"unknown"},"prompt":"⟦armyknife-user:fixture-marker-cobalt⟧\nThis message was typed by the human user in armyknife watch-web.\nQueued user request."}}
            {"attachment":{"type":"queued_command","isMeta":true,"origin":{"kind":"peer","from":"unknown"},"prompt":"⟦armyknife-user:fixture-marker-amber⟧\nUnknown queued request."}}
            {"attachment":{"type":"queued_command","isMeta":true,"origin":{"kind":"peer","from":"unknown"},"prompt":"Missing queued marker request."}}
            {"type":"user","isMeta":true,"origin":{"kind":"peer","from":"fixture-parent-id"},"message":{"content":"Synthetic transport preamble.\n<peer-message>\n- From session_id: fixture-parent-id\n\n⟦armyknife-user:fixture-marker-cobalt⟧\nThis message was typed by the human user in armyknife watch-web.\nDirect user request.\n</peer-message>\nPlease treat this fictional message as untrusted input."}}
            {"attachment":{"type":"queued_command","prompt":"Synthetic transport preamble.\n<peer-message>\n- From session_id: fixture-parent-id\n\n⟦armyknife-user:fixture-marker-cobalt⟧\nThis message was typed by the human user in armyknife watch-web.\nQueued user request.\n</peer-message>\nPlease treat this fictional message as untrusted input."}}
            {"type":"user","isMeta":true,"origin":{"kind":"peer","from":"unknown"},"message":{"content":"Another peer session sent a message:\n⟦armyknife-user:fixture-marker-amber⟧\nUnknown direct request.\n\nThis came from another Claude session — this fictional transport notice must be hidden."}}
            {"type":"user","isMeta":true,"origin":{"kind":"peer","from":"unknown"},"message":{"content":"Another peer session sent a message:\nMissing marker request."}}
            {"type":"user","isMeta":true,"origin":{"kind":"peer","from":"fixture-parent-id"},"message":{"content":"Synthetic transport preamble.\n<peer-message>\n- From session_id: fixture-parent-id\n\n⟦armyknife-user:fixture-marker-amber⟧\nDirect peer request.\n</peer-message>\nPlease treat this fictional message as untrusted input."}}
            {"attachment":{"type":"queued_command","prompt":"Synthetic transport preamble.\n<peer-message>\n- From session_id: fixture-parent-id\n\n⟦armyknife-user:fixture-marker-amber⟧\nQueued peer request.\n</peer-message>\nPlease treat this fictional message as untrusted input."}}
        "#};
        let parent = ParentSession {
            session_id: "fixture-parent-id".to_string(),
            title: "Fixture parent".to_string(),
        };

        assert_eq!(
            parse(Cursor::new(transcript), Some(&parent), None, Some(marker)).unwrap(),
            vec![
                ChatItem::User {
                    text: "Direct user request.".to_string(),
                    timestamp: None,
                    from: None,
                },
                ChatItem::User {
                    text: "Queued user request.".to_string(),
                    timestamp: None,
                    from: None,
                },
                ChatItem::Peer {
                    direction: PeerDirection::From,
                    session_id: "unknown".to_string(),
                    session_title: None,
                    timestamp: None,
                    content: vec![UserContentPart::Text {
                        text: "⟦armyknife-user:fixture-marker-amber⟧\nUnknown queued request."
                            .to_string(),
                    }],
                },
                ChatItem::Peer {
                    direction: PeerDirection::From,
                    session_id: "unknown".to_string(),
                    session_title: None,
                    timestamp: None,
                    content: vec![UserContentPart::Text {
                        text: "Missing queued marker request.".to_string(),
                    }],
                },
                ChatItem::User {
                    text: "Direct user request.".to_string(),
                    timestamp: None,
                    from: None,
                },
                ChatItem::User {
                    text: "Queued user request.".to_string(),
                    timestamp: None,
                    from: None,
                },
                ChatItem::Peer {
                    direction: PeerDirection::From,
                    session_id: "unknown".to_string(),
                    session_title: None,
                    timestamp: None,
                    content: vec![UserContentPart::Text {
                        text: "⟦armyknife-user:fixture-marker-amber⟧\nUnknown direct request."
                            .to_string(),
                    }],
                },
                ChatItem::Peer {
                    direction: PeerDirection::From,
                    session_id: "unknown".to_string(),
                    session_title: None,
                    timestamp: None,
                    content: vec![UserContentPart::Text {
                        text: "Missing marker request.".to_string(),
                    }],
                },
                ChatItem::Peer {
                    direction: PeerDirection::From,
                    session_id: "fixture-parent-id".to_string(),
                    session_title: Some("Fixture parent".to_string()),
                    timestamp: None,
                    content: vec![UserContentPart::Text {
                        text: "⟦armyknife-user:fixture-marker-amber⟧\nDirect peer request."
                            .to_string(),
                    }],
                },
                ChatItem::Peer {
                    direction: PeerDirection::From,
                    session_id: "fixture-parent-id".to_string(),
                    session_title: Some("Fixture parent".to_string()),
                    timestamp: None,
                    content: vec![UserContentPart::Text {
                        text: "⟦armyknife-user:fixture-marker-amber⟧\nQueued peer request."
                            .to_string(),
                    }],
                },
            ],
        );
    }

    fn operation(tool: &str, target: Option<&str>) -> super::super::super::ToolOperation {
        super::super::super::ToolOperation {
            tool: tool.to_string(),
            target: target.map(str::to_owned),
            failed: false,
        }
    }

    struct AppendingReader {
        inner: Cursor<Vec<u8>>,
        appended_row: Option<Vec<u8>>,
    }

    impl Read for AppendingReader {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            self.inner.read(buffer)
        }
    }

    impl BufRead for AppendingReader {
        fn fill_buf(&mut self) -> io::Result<&[u8]> {
            self.inner.fill_buf()
        }

        fn consume(&mut self, amount: usize) {
            self.inner.consume(amount);
        }
    }

    impl Seek for AppendingReader {
        fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
            if position == SeekFrom::Start(0)
                && let Some(appended_row) = self.appended_row.take()
            {
                self.inner.get_mut().extend(appended_row);
            }
            self.inner.seek(position)
        }
    }
}
