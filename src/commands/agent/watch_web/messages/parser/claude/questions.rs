use serde_json::Value;

use super::super::super::{ChatItem, QuestionChoice};

pub(super) fn parse_question_answers(content: &str) -> Option<Vec<(String, String)>> {
    let mut remaining = content
        .strip_prefix("Your questions have been answered:")
        .or_else(|| content.strip_prefix("The user answered:"))?
        .trim_start();
    let mut answers = Vec::new();

    loop {
        let (question, after_question) = parse_quoted_json_string(remaining)?;
        let after_equals = after_question.strip_prefix('=')?;
        let (answer, after_answer) = parse_quoted_json_string(after_equals)?;
        answers.push((question, answer));

        remaining = after_answer.trim_start();
        if let Some(after_comma) = remaining.strip_prefix(',') {
            remaining = after_comma.trim_start();
        } else if !remaining.starts_with('"') {
            break;
        }
    }

    (!answers.is_empty()).then_some(answers)
}

pub(super) fn tool_use_result_answers(tool_use_result: &Value) -> Option<Vec<(String, String)>> {
    let answers = tool_use_result.get("answers")?.as_object()?;
    let answers = answers
        .iter()
        .filter_map(|(question, answer)| {
            answer
                .as_str()
                .map(|answer| (question.clone(), answer.to_owned()))
        })
        .collect::<Vec<_>>();
    (!answers.is_empty()).then_some(answers)
}

fn parse_quoted_json_string(value: &str) -> Option<(String, &str)> {
    if !value.starts_with('"') {
        return None;
    }

    let mut escaped = false;
    for (index, character) in value.char_indices().skip(1) {
        if escaped {
            escaped = false;
        } else if character == '\\' {
            escaped = true;
        } else if character == '"' {
            let end = index + character.len_utf8();
            let parsed = serde_json::from_str(&value[..end]).ok()?;
            return Some((parsed, &value[end..]));
        }
    }

    None
}

pub(super) fn apply_question_answers(
    items: &mut [ChatItem],
    question_indices: &[usize],
    answers: &[(String, String)],
) {
    let mut remaining_answers = answers.to_vec();
    for index in question_indices {
        let Some(ChatItem::Question {
            question, answer, ..
        }) = items.get_mut(*index)
        else {
            continue;
        };
        if let Some(answer_index) = remaining_answers
            .iter()
            .position(|(asked, _)| asked == question)
        {
            let (_, selected_answer) = remaining_answers.remove(answer_index);
            *answer = Some(selected_answer);
        }
    }
}

pub(super) fn append_questions(items: &mut Vec<ChatItem>, input: Option<&Value>) -> Vec<usize> {
    let Some(questions) = input
        .and_then(|input| input.get("questions"))
        .and_then(Value::as_array)
    else {
        return Vec::new();
    };
    let mut indices = Vec::new();
    for question in questions {
        let Some(text) = question.get("question").and_then(Value::as_str) else {
            continue;
        };
        let choices = question
            .get("options")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|option| {
                Some(QuestionChoice {
                    label: option.get("label")?.as_str()?.to_owned(),
                    description: option
                        .get("description")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                })
            })
            .collect();
        indices.push(items.len());
        items.push(ChatItem::Question {
            header: question
                .get("header")
                .and_then(Value::as_str)
                .map(str::to_owned),
            question: text.to_owned(),
            choices,
            multi_select: question
                .get("multiSelect")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            answer: None,
        });
    }
    indices
}
