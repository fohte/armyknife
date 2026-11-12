use serde_json::Value;

use super::super::super::{ChatItem, question_item_from_input};

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
        let Some(ChatItem::Question { questions, .. }) = items.get_mut(*index) else {
            continue;
        };
        for question in questions {
            if let Some(answer_index) = remaining_answers
                .iter()
                .position(|(asked, _)| asked == &question.question)
            {
                let (_, selected_answer) = remaining_answers.remove(answer_index);
                question.answer = Some(selected_answer);
            }
        }
    }
}

pub(super) fn append_questions(items: &mut Vec<ChatItem>, input: Option<&Value>) -> Vec<usize> {
    let Some(input) = input else {
        return Vec::new();
    };
    let Some(item) = question_item_from_input(None, input) else {
        return Vec::new();
    };
    let index = items.len();
    items.push(item);
    vec![index]
}
