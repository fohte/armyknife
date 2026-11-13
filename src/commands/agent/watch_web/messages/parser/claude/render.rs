use std::collections::HashMap;
use std::path::Path;

use chrono::{DateTime, Utc};
use serde_json::Value;

use super::super::super::ChatItem;
use super::super::decision_position::{DecisionTimeline, insert_time_placed_decisions};
use super::super::time_placed::TimePlacedTimeline;
use super::super::todos::TodoTimeline;
use super::super::{
    ToolSummary, append_classification_with_image_directory, append_text_with_image_directory,
    classifier, tool_target,
};
use super::events::{ClaudeEffect, TimedClaudeEvent};
use super::proposals::ProposalTimeline;
use super::questions::apply_question_answers;
use super::state::{ClaudeParserState, ToolUse};
use super::{ParsedTranscript, ParserAdditions};

pub(super) fn render(
    state: &ClaudeParserState,
    image_directory: Option<&Path>,
    now: DateTime<Utc>,
    additions: ParserAdditions<'_>,
) -> ParsedTranscript {
    let mut items = Vec::new();
    let mut tools = ToolSummary::default();
    let mut decision_timeline = DecisionTimeline::new(additions.decisions);
    let mut proposal_timeline = ProposalTimeline::new(additions.proposals);
    let mut todo_timeline = TodoTimeline::new(additions.todos);
    let mut peer_out_timeline = TimePlacedTimeline::new(additions.peer_out_messages);
    let mut question_indices = HashMap::<String, Vec<usize>>::new();

    for event in &state.events {
        render_timed_event(
            event,
            image_directory,
            additions,
            &mut decision_timeline,
            &mut proposal_timeline,
            &mut todo_timeline,
            &mut peer_out_timeline,
            &mut tools,
            &mut items,
            &mut question_indices,
        );
    }

    let active_tool = active_tool_item(&state.tool_uses, additions.pending_tool);
    tools.flush_into(&mut items);
    peer_out_timeline.append_remaining(&mut items);
    if let Some(active_tool) = active_tool {
        items.push(active_tool);
    }

    let mut queue = super::super::queue::Queue::new(additions.queued_prompts);
    for event in &state.queue_events {
        queue.observe(event);
        if event.active_branch_delivery {
            queue.observe_queued_prompt_delivery(event, additions.user_marker);
        }
    }
    let resolved_queued_prompt_ids = queue.resolved_queued_prompt_ids();
    items.extend(queue.pending_items(additions.parent, additions.user_marker, image_directory));
    decision_timeline.append_remaining(&mut items);
    proposal_timeline.append_remaining(&mut items);
    todo_timeline.append_remaining(&mut items);

    (
        items,
        state.background_tasks.clone().into_api_tasks(now),
        resolved_queued_prompt_ids,
        state.skills.clone(),
    )
}

#[expect(
    clippy::too_many_arguments,
    reason = "timed events update several render timelines"
)]
fn render_timed_event(
    event: &TimedClaudeEvent,
    image_directory: Option<&Path>,
    additions: ParserAdditions<'_>,
    decision_timeline: &mut DecisionTimeline,
    proposal_timeline: &mut ProposalTimeline,
    todo_timeline: &mut TodoTimeline,
    peer_out_timeline: &mut TimePlacedTimeline<
        crate::commands::agent::store::WatchWebPeerOutMessage,
    >,
    tools: &mut ToolSummary,
    items: &mut Vec<ChatItem>,
    question_indices: &mut HashMap<String, Vec<usize>>,
) {
    insert_time_placed_decisions(event.timestamp, decision_timeline, tools, items);
    proposal_timeline.insert_before_record(event.timestamp, items, tools);
    todo_timeline.insert_before_record(event.timestamp, items, tools);
    peer_out_timeline.insert_before_record(event.timestamp, items, tools);

    for effect in &event.effects {
        match effect {
            ClaudeEffect::FlushTools => tools.flush_into(items),
            ClaudeEffect::AssistantText { text, .. } => {
                append_text_with_image_directory(
                    items,
                    text.clone(),
                    false,
                    None,
                    image_directory,
                    None,
                );
            }
            ClaudeEffect::Inbound {
                record,
                text,
                images,
                use_user_marker,
                simplify_slash_command,
            } => {
                let marker = use_user_marker.then_some(additions.user_marker).flatten();
                let classification = classifier::classify_inbound(record, text, marker);
                let classification = match classification {
                    classifier::MessageKind::User { text } if *simplify_slash_command => {
                        classifier::MessageKind::User {
                            text: super::user_input::slash_command_text(&text).unwrap_or(text),
                        }
                    }
                    classification => classification,
                };
                append_classification_with_image_directory(
                    items,
                    classification,
                    images.clone(),
                    image_directory,
                    additions.parent,
                    event.timestamp,
                );
            }
            ClaudeEffect::ToolAction {
                name,
                input,
                failed,
            } => tools.record(name, input.as_ref(), *failed),
            ClaudeEffect::Question { id, item } => {
                let index = items.len();
                items.push(item.clone());
                if let Some(id) = id {
                    question_indices.entry(id.clone()).or_default().push(index);
                }
            }
            ClaudeEffect::QuestionAnswers { id, answers } => {
                if let Some(indices) = question_indices.get(id) {
                    apply_question_answers(items, indices, answers);
                }
            }
        }
    }
}

fn active_tool_item(tool_uses: &[ToolUse], pending_tool: Option<&str>) -> Option<ChatItem> {
    let tool_use = pending_tool
        .and_then(|pending| {
            tool_uses
                .iter()
                .rev()
                .find(|tool_use| matches_pending_tool(tool_use, pending))
        })
        .or_else(|| tool_uses.last())?;
    if pending_tool.is_some_and(|pending| matches_pending_tool(tool_use, pending)) {
        Some(ChatItem::Permission {
            request_id: None,
            tool: tool_use.name.clone(),
            description: tool_detail(&tool_use.name, tool_use.input.as_ref()),
        })
    } else {
        Some(ChatItem::ToolInProgress {
            tool: tool_use.name.clone(),
            target: tool_target(&tool_use.name, tool_use.input.as_ref()),
        })
    }
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
        .and_then(super::super::super::permission_request_tool_detail)
        == Some(detail)
}

fn tool_detail(name: &str, input: Option<&Value>) -> String {
    let Some(target) = tool_target(name, input) else {
        return name.to_owned();
    };
    format!("{name}: {target}")
}
