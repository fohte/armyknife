use serde::Serialize;
use serde_json::Value;

use crate::commands::agent::store::PermissionRequestData;

use super::ChatItem;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct QuestionChoice {
    pub(super) label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct QuestionPrompt {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) header: Option<String>,
    pub(super) question: String,
    pub(super) choices: Vec<QuestionChoice>,
    #[serde(rename = "multi_select")]
    pub(super) multi_select: bool,
    pub(super) answer: Option<String>,
}

pub(super) fn attach_question_request_if_present(
    messages: &mut Vec<ChatItem>,
    request_id: Option<&str>,
    data: Option<&PermissionRequestData>,
) {
    let (Some(request_id), Some(data)) = (request_id, data) else {
        return;
    };
    if data.tool_name.as_deref() != Some("AskUserQuestion") {
        return;
    }
    attach_question_request(messages, request_id, &data.tool_input);
}

fn attach_question_request(messages: &mut Vec<ChatItem>, request_id: &str, tool_input: &Value) {
    let Some(ChatItem::Question {
        questions: pending_questions,
        ..
    }) = question_item_from_input(Some(request_id.to_owned()), tool_input)
    else {
        return;
    };
    let pending_question_texts = pending_questions
        .iter()
        .map(|question| question.question.as_str())
        .collect::<Vec<_>>();
    let matching_question = messages.iter_mut().rev().find(|message| {
        let ChatItem::Question { questions, .. } = message else {
            return false;
        };
        questions
            .iter()
            .map(|question| question.question.as_str())
            .eq(pending_question_texts.iter().copied())
    });
    if let Some(ChatItem::Question {
        request_id: current_request_id,
        ..
    }) = matching_question
    {
        *current_request_id = Some(request_id.to_owned());
    } else if let Some(question) = question_item_from_input(Some(request_id.to_owned()), tool_input)
    {
        messages.push(question);
    }
}

pub(super) fn question_item_from_input(
    request_id: Option<String>,
    input: &Value,
) -> Option<ChatItem> {
    let questions = input.get("questions")?.as_array()?;
    let questions = questions
        .iter()
        .filter_map(|question| {
            Some(QuestionPrompt {
                header: question
                    .get("header")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                question: question.get("question").and_then(Value::as_str)?.to_owned(),
                choices: question
                    .get("options")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|option| {
                        Some(QuestionChoice {
                            label: option.get("label").and_then(Value::as_str)?.to_owned(),
                            description: option
                                .get("description")
                                .and_then(Value::as_str)
                                .map(str::to_owned),
                        })
                    })
                    .collect(),
                multi_select: question
                    .get("multiSelect")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                answer: None,
            })
        })
        .collect::<Vec<_>>();
    (!questions.is_empty()).then_some(ChatItem::Question {
        request_id,
        questions,
    })
}
