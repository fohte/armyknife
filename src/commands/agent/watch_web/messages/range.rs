use serde::Deserialize;

use super::{ChatItem, ConversationResponse};

#[derive(Debug, Clone, Copy, Default, Deserialize)]
pub(super) struct MessageRange {
    limit: Option<usize>,
    before: Option<usize>,
    from: Option<usize>,
}

impl MessageRange {
    pub(super) fn is_valid(self) -> bool {
        matches!(
            (self.limit, self.before, self.from),
            (None, None, None)
                | (Some(_), None, None)
                | (Some(_), Some(_), None)
                | (None, None, Some(_))
        )
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
            .rposition(|message| matches!(message, ChatItem::Question { answer: None, .. }));
    }

    pub(super) fn apply_range(&mut self, range: MessageRange) {
        let (start, end) = if let Some(before) = range.before {
            let end = before.min(self.total);
            (end.saturating_sub(range.limit.unwrap_or_default()), end)
        } else if let Some(from) = range.from {
            (from.min(self.total), self.total)
        } else if let Some(limit) = range.limit {
            (self.total.saturating_sub(limit), self.total)
        } else {
            (0, self.total)
        };

        self.messages = self.messages[start..end].to_vec();
        self.start = start;
    }
}
