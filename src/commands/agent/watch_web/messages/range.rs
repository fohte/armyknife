use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{ChatItem, ConversationResponse};

const RECENT_TAIL_MESSAGE_COUNT: usize = 20;
const INITIAL_EPOCH_MESSAGE_COUNT: usize = 100;
const MAX_EPOCH_CANDIDATES: usize = 4096;
const MAX_EPOCH_TOKEN_BYTES: usize = 4096;

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct MessageRange {
    limit: Option<usize>,
    before: Option<usize>,
    from: Option<usize>,
    stable_start: Option<usize>,
    stable_end: Option<usize>,
    stable_digest: Option<String>,
    epoch: Option<String>,
    older_cursor: Option<String>,
}

impl MessageRange {
    pub(super) fn initial_tail_window_limit(&self) -> Option<usize> {
        (self.limit == Some(INITIAL_EPOCH_MESSAGE_COUNT)
            && self.before.is_none()
            && self.from.is_none()
            && self.epoch.is_none()
            && self.older_cursor.is_none())
        .then_some(INITIAL_EPOCH_MESSAGE_COUNT)
    }

    pub(super) fn is_valid(&self) -> bool {
        let stable_range_is_valid = match (
            self.stable_start,
            self.stable_end,
            self.stable_digest.as_deref(),
        ) {
            (None, None, None) => true,
            (Some(start), Some(end), Some(_)) => self.from == Some(start) && start <= end,
            _ => false,
        };
        let cursor_is_valid = self
            .epoch
            .as_ref()
            .is_none_or(|epoch| !epoch.is_empty() && epoch.len() <= MAX_EPOCH_TOKEN_BYTES)
            && self
                .older_cursor
                .as_ref()
                .is_none_or(|cursor| Some(cursor) == self.epoch.as_ref());
        let range_is_valid = if self.epoch.is_some() {
            match (
                self.limit,
                self.before,
                self.from,
                self.older_cursor.is_some(),
            ) {
                (None, None, Some(_), _) => stable_range_is_valid,
                (Some(_), Some(_), None, _) => {
                    self.stable_start.is_none()
                        && self.stable_end.is_none()
                        && self.stable_digest.is_none()
                }
                (Some(_), None, None, true) => {
                    self.stable_start.is_none()
                        && self.stable_end.is_none()
                        && self.stable_digest.is_none()
                }
                _ => false,
            }
        } else {
            self.older_cursor.is_none()
                && matches!(
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
                )
                && stable_range_is_valid
        };

        cursor_is_valid && range_is_valid
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MessageEpochCursor {
    version: u8,
    message_count: usize,
    digest: String,
    first_message_digest: String,
    last_message_digest: String,
}

impl MessageEpochCursor {
    fn from_messages(messages: &[ChatItem]) -> Option<Self> {
        let first_message = messages.first()?;
        let last_message = messages.last()?;
        Some(Self {
            version: 1,
            message_count: messages.len(),
            digest: message_digest(messages)?,
            first_message_digest: message_digest(std::slice::from_ref(first_message))?,
            last_message_digest: message_digest(std::slice::from_ref(last_message))?,
        })
    }

    fn encode(&self) -> Option<String> {
        Some(URL_SAFE_NO_PAD.encode(serde_json::to_vec(self).ok()?))
    }

    fn decode(value: &str) -> Option<Self> {
        let decoded = URL_SAFE_NO_PAD.decode(value).ok()?;
        let cursor = serde_json::from_slice::<Self>(&decoded).ok()?;
        (cursor.version == 1
            && cursor.message_count > 0
            && !cursor.digest.is_empty()
            && !cursor.first_message_digest.is_empty()
            && !cursor.last_message_digest.is_empty())
        .then_some(cursor)
    }

    fn find_start(&self, messages: &[ChatItem]) -> Option<usize> {
        let last_start = messages.len().checked_sub(self.message_count)?;
        let mut matched_start = None;
        let mut candidates = 0;
        for start in 0..=last_start {
            let end = start + self.message_count;
            if message_digest(&messages[start..start + 1]).as_deref()
                != Some(self.first_message_digest.as_str())
                || message_digest(&messages[end - 1..end]).as_deref()
                    != Some(self.last_message_digest.as_str())
            {
                continue;
            }
            candidates += 1;
            if candidates > MAX_EPOCH_CANDIDATES
                || message_digest(&messages[start..end]).as_deref() != Some(&self.digest)
            {
                if candidates > MAX_EPOCH_CANDIDATES {
                    return None;
                }
                continue;
            }
            if matched_start.replace(start).is_some() {
                return None;
            }
        }
        matched_start
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
        if range.initial_tail_window_limit().is_some() && self.initial_epoch {
            self.apply_initial_epoch_range();
            return;
        }

        if let Some(epoch) = range.epoch.clone() {
            if let Some(cursor) = MessageEpochCursor::decode(&epoch)
                && let Some(epoch_start) = cursor.find_start(&self.messages)
            {
                if range.older_cursor.is_some() && range.before.is_none() && range.from.is_none() {
                    self.apply_older_epoch_page(
                        epoch_start,
                        range.limit.unwrap_or_default(),
                        &epoch,
                    );
                } else {
                    self.apply_epoch_range(epoch_start, range);
                }
                return;
            }

            self.apply_latest_epoch_reset();
            return;
        }

        self.apply_numeric_range(range);
    }

    fn apply_initial_epoch_range(&mut self) {
        let page_start = self
            .messages
            .len()
            .saturating_sub(INITIAL_EPOCH_MESSAGE_COUNT);
        let page = self.messages[page_start..].to_vec();
        self.set_initial_epoch_page(page);
    }

    fn set_initial_epoch_page(&mut self, page: Vec<ChatItem>) {
        let cursor = MessageEpochCursor::from_messages(&page).and_then(|cursor| cursor.encode());
        self.messages = page;
        self.start = 0;
        self.total = self.messages.len();
        self.active_question_index = active_question_index(&self.messages);
        let (stable_start, stable_end, stable_digest) = Self::stable_range(0, &self.messages);
        self.stable_start = stable_start;
        self.stable_end = stable_end;
        self.stable_digest = stable_digest;
        self.epoch = cursor.clone();
        self.older_cursor = cursor;
    }

    fn apply_epoch_range(&mut self, epoch_start: usize, range: MessageRange) {
        let decisions = std::mem::take(&mut self.decisions);
        let has_unanswered_proposal = self.has_unanswered_proposal;
        self.messages.drain(..epoch_start);
        self.total = self.messages.len();
        self.active_question_index = active_question_index(&self.messages);
        self.decisions = decisions;
        self.has_unanswered_proposal = has_unanswered_proposal;
        self.epoch = range.epoch.clone();
        self.older_cursor = (epoch_start > 0).then(|| range.epoch.clone()).flatten();
        self.apply_numeric_range(range);
    }

    fn apply_older_epoch_page(&mut self, epoch_start: usize, limit: usize, epoch: &str) {
        let page_start = epoch_start.saturating_sub(limit);
        let page = self.messages[page_start..epoch_start].to_vec();
        if page.is_empty() {
            self.apply_empty_older_epoch_page_from_start(epoch_start, epoch);
            return;
        }

        let epoch_messages = self.messages[page_start..].to_vec();
        let new_cursor =
            MessageEpochCursor::from_messages(&page).and_then(|cursor| cursor.encode());
        self.start = 0;
        self.total = epoch_messages.len();
        self.active_question_index = active_question_index(&epoch_messages);
        let (stable_start, stable_end, stable_digest) = Self::stable_range(0, &epoch_messages);
        self.stable_start = stable_start;
        self.stable_end = stable_end;
        self.stable_digest = stable_digest;
        self.messages = page;
        self.epoch = new_cursor.clone();
        self.older_cursor = (page_start > 0).then_some(new_cursor).flatten();
    }

    fn apply_empty_older_epoch_page_from_start(&mut self, epoch_start: usize, epoch: &str) {
        let epoch_messages = self.messages[epoch_start..].to_vec();
        let decisions = std::mem::take(&mut self.decisions);
        let has_unanswered_proposal = self.has_unanswered_proposal;
        self.messages.clear();
        self.start = 0;
        self.total = epoch_messages.len();
        self.active_question_index = active_question_index(&epoch_messages);
        let (stable_start, stable_end, stable_digest) = Self::stable_range(0, &epoch_messages);
        self.stable_start = stable_start;
        self.stable_end = stable_end;
        self.stable_digest = stable_digest;
        self.decisions = decisions;
        self.has_unanswered_proposal = has_unanswered_proposal;
        self.epoch = Some(epoch.to_owned());
        self.older_cursor = None;
    }

    fn apply_latest_epoch_reset(&mut self) {
        let page_start = self
            .messages
            .len()
            .saturating_sub(INITIAL_EPOCH_MESSAGE_COUNT);
        self.set_initial_epoch_page(self.messages[page_start..].to_vec());
        self.epoch_reset = true;
    }

    fn stable_range(
        stable_start: usize,
        messages: &[ChatItem],
    ) -> (Option<usize>, Option<usize>, Option<String>) {
        let stable_end = messages.len().saturating_sub(RECENT_TAIL_MESSAGE_COUNT);
        let stable_digest = (stable_start < stable_end)
            .then(|| message_digest(&messages[stable_start..stable_end]))
            .flatten();
        (
            stable_digest.as_ref().map(|_| stable_start),
            stable_digest.as_ref().map(|_| stable_end),
            stable_digest,
        )
    }

    fn apply_numeric_range(&mut self, range: MessageRange) {
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

fn active_question_index(messages: &[ChatItem]) -> Option<usize> {
    messages
        .iter()
        .rposition(|message| matches!(message, ChatItem::Question { .. }))
}

fn message_digest(messages: &[ChatItem]) -> Option<String> {
    let encoded = serde_json::to_vec(messages).ok()?;
    Some(crate::shared::hex::encode(&Sha256::digest(encoded)))
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::{MessageEpochCursor, MessageRange, active_question_index};
    use crate::commands::agent::watch_web::messages::{ChatItem, ConversationResponse};

    #[rstest]
    #[case::initial_page(MessageRange { limit: Some(100), ..MessageRange::default() }, Some(100))]
    #[case::older_page(MessageRange { limit: Some(100), before: Some(200), ..MessageRange::default() }, None)]
    #[case::stable_refresh(MessageRange { from: Some(0), ..MessageRange::default() }, None)]
    #[case::other_limit(MessageRange { limit: Some(50), ..MessageRange::default() }, None)]
    fn only_the_initial_hundred_message_page_uses_a_tail_window(
        #[case] range: MessageRange,
        #[case] expected: Option<usize>,
    ) {
        assert_eq!(range.initial_tail_window_limit(), expected);
    }

    #[test]
    fn stale_epoch_refresh_resets_to_the_latest_page() {
        let mut messages = (0..101)
            .map(|index| ChatItem::Assistant {
                text: format!("Fixture message {index}."),
            })
            .collect::<Vec<_>>();
        let stale_cursor = MessageEpochCursor::from_messages(&messages[1..])
            .and_then(|cursor| cursor.encode())
            .unwrap();
        messages[100] = ChatItem::Assistant {
            text: "Updated latest fixture message.".to_owned(),
        };

        let mut actual = ConversationResponse {
            messages: messages.clone(),
            ..ConversationResponse::default()
        };
        actual.update_metadata();
        actual.apply_range(MessageRange {
            from: Some(0),
            epoch: Some(stale_cursor.clone()),
            older_cursor: Some(stale_cursor),
            ..MessageRange::default()
        });

        let latest_page = messages[1..].to_vec();
        let new_cursor =
            MessageEpochCursor::from_messages(&latest_page).and_then(|cursor| cursor.encode());
        let (stable_start, stable_end, stable_digest) =
            ConversationResponse::stable_range(0, &latest_page);
        assert_eq!(
            actual,
            ConversationResponse {
                total: latest_page.len(),
                messages: latest_page.clone(),
                active_question_index: active_question_index(&latest_page),
                stable_start,
                stable_end,
                stable_digest,
                epoch: new_cursor.clone(),
                older_cursor: new_cursor,
                epoch_reset: true,
                ..ConversationResponse::default()
            },
        );
    }
}
