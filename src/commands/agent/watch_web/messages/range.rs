use serde::Deserialize;
use sha2::{Digest, Sha256};

use super::{ChatItem, ConversationResponse};

const UNVERIFIED_TAIL_MESSAGES: usize = 20;

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct MessageRange {
    limit: Option<usize>,
    before: Option<usize>,
    from: Option<usize>,
    tail_start: Option<usize>,
    tail_end: Option<usize>,
    tail_digest: Option<String>,
}

impl MessageRange {
    pub(super) fn is_valid(&self) -> bool {
        matches!(
            (
                self.limit,
                self.before,
                self.from,
                self.tail_start,
                self.tail_end,
                self.tail_digest.as_deref(),
            ),
            (None, None, None, None, None, None)
                | (Some(_), None, None, None, None, None)
                | (Some(_), Some(_), None, None, None, None)
                | (None, None, Some(_), None, None, None)
                | (None, None, Some(_), Some(_), Some(_), Some(_))
        ) && self
            .tail_start
            .zip(self.tail_end)
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
            .tail_start
            .zip(range.tail_end)
            .zip(range.tail_digest.as_deref())
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
                    range.tail_end.unwrap_or(from).min(self.total)
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

        let digest_start = range.from.unwrap_or(start).min(self.total);
        let digest_end = self.total.saturating_sub(UNVERIFIED_TAIL_MESSAGES);
        let tail_digest = (range.before.is_none() && digest_start < digest_end)
            .then(|| message_digest(&self.messages[digest_start..digest_end]))
            .flatten();

        self.messages = self.messages[start..end].to_vec();
        self.start = start;
        self.tail_start = tail_digest.as_ref().map(|_| digest_start);
        self.tail_end = tail_digest.as_ref().map(|_| digest_end);
        self.tail_digest = tail_digest;
    }
}

fn message_digest(messages: &[ChatItem]) -> Option<String> {
    let encoded = serde_json::to_vec(messages).ok()?;
    Some(crate::shared::hex::encode(&Sha256::digest(encoded)))
}
