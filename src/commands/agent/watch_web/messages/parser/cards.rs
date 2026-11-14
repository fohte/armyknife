use std::collections::HashSet;

use chrono::{DateTime, Utc};

use super::super::proposals::ProposalItem;
use super::super::{ChatItem, PeerDirection, UserContentPart};
use super::ToolSummary;
use crate::commands::agent::store::{AgentDecision, AgentTodo, WatchWebPeerOutMessage};

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(in crate::commands::agent::watch_web::messages) enum CardKind {
    Decision,
    Proposal,
    Todo,
    PeerOut,
    Permission,
    PermissionDenied,
}

#[derive(Clone)]
pub(in crate::commands::agent::watch_web::messages) struct TimedCard {
    kind: CardKind,
    created_at: Option<DateTime<Utc>>,
    seq: i64,
    awaiting_user: bool,
    item: ChatItem,
}

impl TimedCard {
    pub(in crate::commands::agent::watch_web::messages) fn new(
        kind: CardKind,
        created_at: Option<DateTime<Utc>>,
        seq: i64,
        awaiting_user: bool,
        item: ChatItem,
    ) -> Self {
        Self {
            kind,
            created_at,
            seq,
            awaiting_user,
            item,
        }
    }

    fn key(&self) -> (bool, Option<DateTime<Utc>>, CardKind, i64) {
        (
            self.created_at.is_none(),
            self.created_at,
            self.kind,
            self.seq,
        )
    }
}

pub(in crate::commands::agent::watch_web::messages) struct CardTimeline {
    cards: Vec<TimedCard>,
    inserted: HashSet<(CardKind, i64)>,
}

impl CardTimeline {
    pub(in crate::commands::agent::watch_web::messages) fn new(cards: &[TimedCard]) -> Self {
        Self {
            cards: cards.to_vec(),
            inserted: HashSet::new(),
        }
    }

    pub(in crate::commands::agent::watch_web::messages) fn insert_before_record(
        &mut self,
        timestamp: Option<DateTime<Utc>>,
        items: &mut Vec<ChatItem>,
        tools: &mut ToolSummary,
    ) {
        let Some(timestamp) = timestamp else {
            return;
        };
        let mut due = self
            .cards
            .iter()
            .filter(|card| {
                !card.awaiting_user
                    && card
                        .created_at
                        .is_some_and(|created_at| created_at <= timestamp)
                    && !self.inserted.contains(&(card.kind, card.seq))
            })
            .collect::<Vec<_>>();
        due.sort_by_key(|card| card.key());
        if due.is_empty() {
            return;
        }

        tools.flush_into(items);
        for card in due {
            items.push(card.item.clone());
            self.inserted.insert((card.kind, card.seq));
        }
    }

    pub(in crate::commands::agent::watch_web::messages) fn push(&mut self, card: TimedCard) {
        self.cards.push(card);
    }

    pub(in crate::commands::agent::watch_web::messages) fn update_pending(
        &mut self,
        kind: CardKind,
        seq: i64,
        created_at: Option<DateTime<Utc>>,
        item: ChatItem,
    ) -> bool {
        let Some(card) = self
            .cards
            .iter_mut()
            .find(|card| card.kind == kind && card.seq == seq && card.awaiting_user)
        else {
            return false;
        };
        card.created_at = created_at;
        card.item = item;
        true
    }

    pub(in crate::commands::agent::watch_web::messages) fn append_remaining(
        &mut self,
        items: &mut Vec<ChatItem>,
    ) {
        let mut remaining = self
            .cards
            .iter()
            .filter(|card| !self.inserted.contains(&(card.kind, card.seq)))
            .collect::<Vec<_>>();
        remaining.sort_by_key(|card| (card.awaiting_user, card.key()));
        items.extend(remaining.into_iter().map(|card| card.item.clone()));
        self.inserted
            .extend(self.cards.iter().map(|card| (card.kind, card.seq)));
    }
}

pub(in crate::commands::agent::watch_web::messages) fn cards_from_sources(
    decisions: &[AgentDecision],
    proposals: &[ProposalItem],
    todos: &[AgentTodo],
    peer_out_messages: &[WatchWebPeerOutMessage],
    additional_cards: &[TimedCard],
) -> Vec<TimedCard> {
    let mut cards = decisions
        .iter()
        .map(|decision| {
            let created_at = DateTime::parse_from_rfc3339(&decision.created_at)
                .ok()
                .map(|value| value.with_timezone(&Utc));
            TimedCard::new(
                CardKind::Decision,
                created_at,
                decision.seq,
                decision.review.is_none(),
                ChatItem::from(decision.clone()),
            )
        })
        .collect::<Vec<_>>();
    cards.extend(proposals.iter().map(|proposal| {
        let resolved = matches!(&proposal.item, ChatItem::Proposal { resolved: true, .. });
        TimedCard::new(
            CardKind::Proposal,
            Some(proposal.created_at),
            proposal.seq,
            !resolved,
            proposal.item.clone(),
        )
    }));
    cards.extend(todos.iter().map(|todo| {
        TimedCard::new(
            CardKind::Todo,
            Some(todo.created_at),
            todo.seq,
            todo.completed_at.is_none(),
            ChatItem::Todo {
                seq: todo.seq,
                title: todo.title.clone(),
                detail: todo.detail.clone(),
                completed: todo.completed_at.is_some(),
                resolution: todo.resolution,
                comment: todo.comment.clone(),
            },
        )
    }));
    cards.extend(peer_out_messages.iter().map(|message| {
        TimedCard::new(
            CardKind::PeerOut,
            Some(message.sent_at),
            message.id,
            false,
            ChatItem::Peer {
                direction: PeerDirection::To,
                session_id: message.to_session_id.clone(),
                session_title: None,
                content: vec![UserContentPart::Text {
                    text: message.content.clone(),
                }],
                timestamp: Some(message.sent_at.to_rfc3339()),
                failed: message.failed,
            },
        )
    }));
    cards.extend_from_slice(additional_cards);
    cards
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use chrono::{DateTime, Utc};
    use indoc::indoc;

    use super::*;
    use crate::commands::agent::proposal::{ProposalAnswer, ProposalOption};
    use crate::commands::agent::store::{AgentDecision, PermissionRequestData, TodoResolution};
    use crate::commands::agent::watch_web::messages::parser::claude::{
        ParserAdditions, parse_with_queued_prompts_and_peer_out_messages_and_proposals,
    };
    use crate::commands::agent::watch_web::messages::parser::codex::CodexParserState;

    fn timestamp(value: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(value)
            .expect("test timestamp should parse")
            .with_timezone(&Utc)
    }

    #[test]
    fn claude_and_codex_place_every_card_kind_with_the_same_timeline() {
        let decisions = [AgentDecision {
            session_id: "fixture-session".to_string(),
            seq: 1,
            title: "A pending decision".to_string(),
            why: "The example should remain adjustable.".to_string(),
            considered: Vec::new(),
            easy_to_change: "Adjust the example.".to_string(),
            created_at: "2026-06-01T12:01:30Z".to_string(),
            review: None,
            reviewed_at: None,
        }];
        let proposals = [
            ProposalItem {
                seq: 1,
                created_at: timestamp("2026-06-01T12:01:00Z"),
                item: ChatItem::Proposal {
                    seq: 1,
                    title: "A resolved proposal".to_string(),
                    summary: None,
                    options: vec![ProposalOption {
                        label: "Option A".to_string(),
                        description: None,
                        pros: Vec::new(),
                        cons: Vec::new(),
                        recommended: false,
                    }],
                    answer: Some(ProposalAnswer {
                        choice: Some(0),
                        comment: None,
                    }),
                    resolved: true,
                    resolution: None,
                    comment: None,
                },
            },
            ProposalItem {
                seq: 2,
                created_at: timestamp("2026-06-01T12:00:10Z"),
                item: ChatItem::Proposal {
                    seq: 2,
                    title: "A pending proposal".to_string(),
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
                    resolution: None,
                    comment: None,
                },
            },
            ProposalItem {
                seq: 3,
                created_at: timestamp("2026-06-01T12:00:30Z"),
                item: ChatItem::Proposal {
                    seq: 3,
                    title: "A later pending proposal".to_string(),
                    summary: None,
                    options: vec![ProposalOption {
                        label: "Option C".to_string(),
                        description: None,
                        pros: Vec::new(),
                        cons: Vec::new(),
                        recommended: false,
                    }],
                    answer: None,
                    resolved: false,
                    resolution: None,
                    comment: None,
                },
            },
        ];
        let todos = [
            AgentTodo {
                session_id: "fixture-session".to_string(),
                seq: 1,
                title: "A completed todo".to_string(),
                detail: None,
                created_at: timestamp("2026-06-01T12:02:00Z"),
                completed_at: Some(timestamp("2026-06-01T12:03:00Z")),
                resolution: Some(TodoResolution::Completed),
                comment: None,
            },
            AgentTodo {
                session_id: "fixture-session".to_string(),
                seq: 2,
                title: "A pending todo".to_string(),
                detail: None,
                created_at: timestamp("2026-06-01T12:00:20Z"),
                completed_at: None,
                resolution: None,
                comment: None,
            },
        ];
        let peer_out_messages = [WatchWebPeerOutMessage {
            id: 1,
            from_session_id: "fixture-session".to_string(),
            to_session_id: "fixture-peer".to_string(),
            content: "A peer message".to_string(),
            sent_at: timestamp("2026-06-01T12:03:00Z"),
            failed: false,
        }];
        let additional_cards = [
            TimedCard::new(
                CardKind::PermissionDenied,
                Some(timestamp("2026-06-01T12:04:00Z")),
                1,
                false,
                ChatItem::PermissionDenied {
                    seq: 1,
                    tool_use_id: None,
                    tool: "Read".to_string(),
                    input_summary: "A sample file".to_string(),
                    command: None,
                    reason: Some("A resolved denial".to_string()),
                    retryable: true,
                    resolved: true,
                },
            ),
            TimedCard::new(
                CardKind::PermissionDenied,
                Some(timestamp("2026-06-01T12:02:30Z")),
                2,
                true,
                ChatItem::PermissionDenied {
                    seq: 2,
                    tool_use_id: None,
                    tool: "Write".to_string(),
                    input_summary: "Another sample file".to_string(),
                    command: None,
                    reason: Some("A pending denial".to_string()),
                    retryable: true,
                    resolved: false,
                },
            ),
            TimedCard::new(
                CardKind::Permission,
                None,
                0,
                true,
                ChatItem::Permission {
                    request_id: Some("fixture-permission-request".to_string()),
                    tool: "Permission fallback".to_string(),
                    description: "Waiting for permission".to_string(),
                    command: None,
                },
            ),
        ];
        let cards = cards_from_sources(
            &decisions,
            &proposals,
            &todos,
            &peer_out_messages,
            &additional_cards,
        );
        let permission = PermissionRequestData {
            tool_name: Some("Bash".to_string()),
            tool_input: serde_json::json!({"command": "a sample command"}),
            pending: true,
        };
        let claude_transcript = indoc! {r#"
            {"uuid":"fixture-before","parentUuid":null,"timestamp":"2026-06-01T12:00:00Z","type":"assistant","message":{"content":[{"type":"text","text":"before"}]}}
            {"uuid":"fixture-after","parentUuid":"fixture-before","timestamp":"2026-06-01T12:05:00Z","type":"assistant","message":{"content":[{"type":"text","text":"after"}]}}
            {"uuid":"fixture-permission","parentUuid":"fixture-after","timestamp":"2026-06-01T12:06:00Z","type":"assistant","message":{"content":[{"type":"tool_use","id":"fixture-permission-call","name":"Bash","input":{"command":"a sample command"}}]}}
        "#};
        let codex_transcript = indoc! {r#"
            {"type":"response_item","timestamp":"2026-06-01T12:00:00Z","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"before"}]}}
            {"type":"response_item","timestamp":"2026-06-01T12:05:00Z","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"after"}]}}
            {"type":"response_item","timestamp":"2026-06-01T12:06:00Z","payload":{"type":"custom_tool_call","call_id":"fixture-permission-call","name":"Bash","input":{"command":"a sample command"}}}
        "#};
        let (claude_messages, _, _) =
            parse_with_queued_prompts_and_peer_out_messages_and_proposals(
                Cursor::new(claude_transcript),
                timestamp("2026-06-01T12:07:00Z"),
                ParserAdditions {
                    session_id: "fixture-session",
                    parent: None,
                    pending_tool: Some(&permission),
                    user_marker: None,
                    decisions: &decisions,
                    queued_prompts: &[],
                    peer_out_messages: &peer_out_messages,
                    proposals: &proposals,
                    todos: &todos,
                    additional_cards: &additional_cards,
                },
            )
            .expect("Claude transcript should render");
        let mut codex_parser = CodexParserState::new();
        codex_parser
            .append(Cursor::new(codex_transcript))
            .expect("Codex transcript should parse");
        let (codex_messages, _) = codex_parser
            .render_with_placement_cards(None, &cards, Some(&permission))
            .expect("Codex transcript should render");
        let expected = vec![
            ChatItem::Assistant {
                text: "before".to_string(),
            },
            proposals[0].item.clone(),
            ChatItem::Todo {
                seq: 1,
                title: "A completed todo".to_string(),
                detail: None,
                completed: true,
                resolution: Some(TodoResolution::Completed),
                comment: None,
            },
            ChatItem::Peer {
                direction: PeerDirection::To,
                session_id: "fixture-peer".to_string(),
                session_title: None,
                content: vec![UserContentPart::Text {
                    text: "A peer message".to_string(),
                }],
                timestamp: Some("2026-06-01T12:03:00+00:00".to_string()),
                failed: false,
            },
            additional_cards[0].item.clone(),
            ChatItem::Assistant {
                text: "after".to_string(),
            },
            proposals[1].item.clone(),
            ChatItem::Todo {
                seq: 2,
                title: "A pending todo".to_string(),
                detail: None,
                completed: false,
                resolution: None,
                comment: None,
            },
            proposals[2].item.clone(),
            ChatItem::Decision {
                seq: 1,
                title: "A pending decision".to_string(),
                why: "The example should remain adjustable.".to_string(),
                considered: Vec::new(),
                easy_to_change: "Adjust the example.".to_string(),
                review: None,
                created_at: "2026-06-01T12:01:30Z".to_string(),
            },
            additional_cards[1].item.clone(),
            ChatItem::Permission {
                request_id: None,
                tool: "Bash".to_string(),
                description: String::new(),
                command: Some("a sample command".to_string()),
            },
        ];

        assert_eq!(
            (claude_messages, codex_messages),
            (expected.clone(), expected)
        );
    }
}
