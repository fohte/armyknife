use super::{ChatItem, UserContentPart};

pub(super) fn contains_markdown_source(messages: &[ChatItem], expected: &str) -> bool {
    messages
        .iter()
        .any(|message| message_contains_markdown_source(message, expected))
}

fn message_contains_markdown_source(message: &ChatItem, expected: &str) -> bool {
    match message {
        ChatItem::User { text, .. } | ChatItem::Assistant { text } => text == expected,
        ChatItem::QueuedUser { content, .. }
        | ChatItem::UserWithContent { content, .. }
        | ChatItem::Peer { content, .. } => content_contains_markdown_source(content, expected),
        ChatItem::SubagentReport { text, .. } => text == expected,
        ChatItem::Question { questions, .. } => questions.iter().any(|question| {
            question.question == expected
                || question.choices.iter().any(|choice| {
                    choice.label == expected
                        || choice
                            .description
                            .as_deref()
                            .is_some_and(|description| description == expected)
                })
        }),
        ChatItem::Proposal {
            summary,
            options,
            answer,
            comment,
            ..
        } => {
            summary.as_deref() == Some(expected)
                || comment.as_deref() == Some(expected)
                || answer.as_ref().and_then(|answer| answer.comment.as_deref()) == Some(expected)
                || options
                    .iter()
                    .any(|option| option.description.as_deref() == Some(expected))
        }
        ChatItem::Todo {
            detail, comment, ..
        } => detail.as_deref() == Some(expected) || comment.as_deref() == Some(expected),
        ChatItem::Decision {
            why,
            considered,
            easy_to_change,
            ..
        } => {
            why == expected
                || easy_to_change == expected
                || considered.iter().any(|item| item == expected)
        }
        ChatItem::Event { .. }
        | ChatItem::FileAttachment { .. }
        | ChatItem::ToolSummary { .. }
        | ChatItem::ToolInProgress { .. }
        | ChatItem::Permission { .. }
        | ChatItem::PermissionDenied { .. } => false,
    }
}

fn content_contains_markdown_source(content: &[UserContentPart], expected: &str) -> bool {
    content.iter().any(|part| match part {
        UserContentPart::Text { text } => text == expected,
        UserContentPart::Image { .. } | UserContentPart::File { .. } => false,
    })
}

#[cfg(test)]
mod tests {
    use crate::commands::agent::proposal::{ProposalAnswer, ProposalOption};
    use crate::commands::agent::store::ProposalResolution;

    use super::super::{ChatItem, PeerDirection, QuestionChoice, QuestionPrompt, UserContentPart};
    use super::contains_markdown_source;

    #[test]
    fn finds_markdown_source_in_rendered_message_and_card_fields() {
        let messages = vec![
            ChatItem::Assistant {
                text: "assistant text".to_string(),
            },
            ChatItem::User {
                text: "user text".to_string(),
                rewind_id: None,
                queued_prompt_id: None,
                timestamp: None,
                from: None,
            },
            ChatItem::Peer {
                direction: PeerDirection::From,
                session_id: "peer-session".to_string(),
                session_title: None,
                content: vec![UserContentPart::Text {
                    text: "peer text".to_string(),
                }],
                timestamp: None,
                failed: false,
            },
            ChatItem::SubagentReport {
                agent_id: "agent-id".to_string(),
                agent_name: "Agent".to_string(),
                description: None,
                text: "subagent text".to_string(),
            },
            ChatItem::Question {
                request_id: None,
                questions: vec![QuestionPrompt {
                    header: None,
                    question: "question text".to_string(),
                    choices: vec![QuestionChoice {
                        label: "choice label".to_string(),
                        description: Some("choice description".to_string()),
                    }],
                    multi_select: false,
                    answer: None,
                }],
            },
            ChatItem::Proposal {
                seq: 1,
                title: "proposal title".to_string(),
                summary: Some("proposal summary".to_string()),
                options: vec![ProposalOption {
                    label: "proposal option label".to_string(),
                    description: Some("proposal option description".to_string()),
                    pros: Vec::new(),
                    cons: Vec::new(),
                    recommended: false,
                }],
                answer: Some(ProposalAnswer {
                    choice: Some(0),
                    comment: Some("proposal answer".to_string()),
                }),
                resolved: true,
                resolution: Some(ProposalResolution::Withdrawn),
                comment: Some("proposal withdrawal".to_string()),
            },
            ChatItem::Todo {
                seq: 2,
                title: "todo title".to_string(),
                detail: Some("todo detail".to_string()),
                completed: false,
                resolution: None,
                comment: Some("todo comment".to_string()),
            },
            ChatItem::Decision {
                seq: 3,
                title: "decision title".to_string(),
                why: "decision why".to_string(),
                considered: vec!["decision option".to_string()],
                easy_to_change: "decision reversibility".to_string(),
                review: None,
                created_at: "2026-01-01T00:00:00Z".to_string(),
            },
        ];
        let sources = [
            "assistant text",
            "user text",
            "peer text",
            "subagent text",
            "question text",
            "choice label",
            "choice description",
            "proposal summary",
            "proposal option description",
            "proposal answer",
            "proposal withdrawal",
            "todo detail",
            "todo comment",
            "decision why",
            "decision option",
            "decision reversibility",
            "proposal option label",
            "decision title",
        ];

        assert_eq!(
            sources
                .iter()
                .map(|source| contains_markdown_source(&messages, source))
                .collect::<Vec<_>>(),
            [
                true, true, true, true, true, true, true, true, true, true, true, true, true, true,
                true, true, false, false,
            ],
        );
    }
}
