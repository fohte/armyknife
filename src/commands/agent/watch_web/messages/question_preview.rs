use anyhow::Result;

use crate::commands::agent::store::PermissionRequestData;
use crate::commands::agent::types::Session;

use super::cache::transcript_fingerprint;
use super::questions::question_item_from_input;
use super::{ChatItem, QuestionPrompt, RouteLoaders, TranscriptInput, incremental, parser};

impl RouteLoaders {
    pub(in crate::commands::agent::watch_web) fn question_preview(
        &self,
        session: &Session,
    ) -> Result<Option<String>> {
        if let Some(preview) = transcript_question_preview(session, &self.message_cache) {
            return Ok(Some(preview));
        }
        for agent_id in &session.pending_permission_agent_ids {
            let Some(request_id) = session.pending_permission_request_ids.get(agent_id) else {
                continue;
            };
            let Some(data) = (self.permission_request_data)(&session.session_id, request_id)?
            else {
                continue;
            };
            if let Some(preview) = permission_question_preview(request_id, &data) {
                return Ok(Some(preview));
            }
        }
        Ok(None)
    }
}

fn transcript_question_preview(
    session: &Session,
    message_cache: &super::cache::MessageCache,
) -> Option<String> {
    let parse = || -> Result<Option<String>> {
        let fingerprint =
            transcript_fingerprint(session.engine, session.transcript_path.as_deref())?;
        let input = TranscriptInput {
            path: session.transcript_path.as_deref(),
            engine: session.engine,
            additions: parser::claude::ParserAdditions {
                session_id: &session.session_id,
                parent: None,
                pending_tool: None,
                user_marker: None,
                decisions: &[],
                queued_prompts: &[],
                peer_out_messages: &[],
                proposals: &[],
                todos: &[],
            },
        };
        let (conversation, _) = incremental::parse_session_transcript_incrementally(
            &session.session_id,
            input,
            fingerprint,
            message_cache,
        )?;
        let questions = conversation.messages.iter().rev().find_map(|message| {
            let ChatItem::Question { questions, .. } = message else {
                return None;
            };
            questions
                .iter()
                .any(|question| question.answer.is_none())
                .then_some(questions)
        });
        Ok(questions.and_then(|questions| format_question_preview(questions)))
    };

    match parse() {
        Ok(preview) => preview,
        Err(error) => {
            tracing::warn!(
                session_id = %session.session_id,
                %error,
                "failed to parse session question preview transcript",
            );
            None
        }
    }
}

fn permission_question_preview(request_id: &str, data: &PermissionRequestData) -> Option<String> {
    if data.tool_name.as_deref() != Some("AskUserQuestion") {
        return None;
    }
    let Some(ChatItem::Question { questions, .. }) =
        question_item_from_input(Some(request_id.to_owned()), &data.tool_input)
    else {
        return None;
    };
    format_question_preview(&questions)
}

fn format_question_preview(questions: &[QuestionPrompt]) -> Option<String> {
    let first_question = questions.first()?;
    Some(if questions.len() > 1 {
        format!("{} +{}", first_question.question, questions.len() - 1)
    } else {
        first_question.question.clone()
    })
}
