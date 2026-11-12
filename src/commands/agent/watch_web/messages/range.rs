use serde::Deserialize;
use sha2::{Digest, Sha256};

use super::{ChatItem, ConversationResponse};

const RECENT_TAIL_MESSAGE_COUNT: usize = 20;

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct MessageRange {
    limit: Option<usize>,
    before: Option<usize>,
    from: Option<usize>,
    stable_start: Option<usize>,
    stable_end: Option<usize>,
    stable_digest: Option<String>,
}

impl MessageRange {
    pub(super) fn is_valid(&self) -> bool {
        matches!(
            (
                self.limit,
                self.before,
                self.from,
                self.stable_start,
                self.stable_end,
                self.stable_digest.as_deref(),
            ),
            (None, None, None, None, None, None)
                | (Some(_), None, None, None, None, None)
                | (Some(_), Some(_), None, None, None, None)
                | (None, None, Some(_), None, None, None)
                | (None, None, Some(_), Some(_), Some(_), Some(_))
        ) && self
            .stable_start
            .zip(self.stable_end)
            .is_none_or(|(start, end)| self.from == Some(start) && start <= end)
    }
}

impl ConversationResponse {
    pub(super) fn update_metadata(&mut self) {
        self.total = self.messages.len();
        self.decisions = self
            .messages
            .iter()
            .filter(|message| matches!(message, ChatItem::Decision { .. }))
            .cloned()
            .collect();
        self.has_unanswered_proposal = self.messages.iter().any(|message| {
            matches!(
                message,
                ChatItem::Proposal {
                    answer: None,
                    resolved: false,
                    ..
                }
            )
        });
        self.active_question_index = self
            .messages
            .iter()
            .rposition(|message| matches!(message, ChatItem::Question { .. }));
    }

    pub(super) fn apply_range(&mut self, range: MessageRange) {
        let digest_matches = range
            .stable_start
            .zip(range.stable_end)
            .zip(range.stable_digest.as_deref())
            .is_some_and(|((start, end), digest)| {
                start <= end
                    && end <= self.messages.len()
                    && message_digest(&self.messages[start..end]).as_deref() == Some(digest)
            });
        let (start, end) = if let Some(before) = range.before {
            let end = before.min(self.total);
            (end.saturating_sub(range.limit.unwrap_or_default()), end)
        } else if let Some(from) = range.from {
            (
                if digest_matches {
                    range.stable_end.unwrap_or(from).min(self.total)
                } else {
                    from.min(self.total)
                },
                self.total,
            )
        } else if let Some(limit) = range.limit {
            (self.total.saturating_sub(limit), self.total)
        } else {
            (0, self.total)
        };

        let stable_start = range.from.unwrap_or(start).min(self.total);
        let stable_end = self.total.saturating_sub(RECENT_TAIL_MESSAGE_COUNT);
        let stable_digest = (range.before.is_none() && stable_start < stable_end)
            .then(|| message_digest(&self.messages[stable_start..stable_end]))
            .flatten();

        self.messages = self.messages[start..end].to_vec();
        self.start = start;
        self.stable_start = stable_digest.as_ref().map(|_| stable_start);
        self.stable_end = stable_digest.as_ref().map(|_| stable_end);
        self.stable_digest = stable_digest;
    }
}

fn message_digest(messages: &[ChatItem]) -> Option<String> {
    let encoded = serde_json::to_vec(messages).ok()?;
    Some(crate::shared::hex::encode(&Sha256::digest(encoded)))
}
