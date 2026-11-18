use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{ChatItem, ConversationResponse};

const RECENT_TAIL_MESSAGE_COUNT: usize = 20;
const INITIAL_EPOCH_MESSAGE_COUNT: usize = 100;
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
    pub(super) fn requested_epoch(&self) -> Option<&str> {
        self.epoch.as_deref()
    }

    pub(super) fn requires_full_history(&self) -> bool {
        self.older_cursor.is_some()
    }

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
    anchor_count: usize,
    anchor_digests: Vec<String>,
}

impl MessageEpochCursor {
    fn from_messages(messages: &[ChatItem]) -> Option<Self> {
        let stable_prefix_count = messages.len().saturating_sub(RECENT_TAIL_MESSAGE_COUNT);
        let anchor_count = stable_prefix_count.min(RECENT_TAIL_MESSAGE_COUNT);
        if anchor_count == 0 {
            return None;
        }
        Some(Self {
            version: 2,
            anchor_count,
            anchor_digests: message_digests(&messages[..anchor_count])?,
        })
    }

    fn encode(&self) -> Option<String> {
        Some(URL_SAFE_NO_PAD.encode(serde_json::to_vec(self).ok()?))
    }

    fn decode(value: &str) -> Option<Self> {
        let decoded = URL_SAFE_NO_PAD.decode(value).ok()?;
        let cursor = serde_json::from_slice::<Self>(&decoded).ok()?;
        (cursor.version == 2
            && cursor.anchor_count > 0
            && cursor.anchor_count <= RECENT_TAIL_MESSAGE_COUNT
            && cursor.anchor_count == cursor.anchor_digests.len()
            && cursor.anchor_digests.iter().all(|digest| {
                digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit())
            }))
        .then_some(cursor)
    }

    fn find_start(&self, messages: &[ChatItem]) -> Option<usize> {
        let message_digests = message_digests(messages)?;
        let last_start = message_digests.len().checked_sub(self.anchor_count)?;
        let mut matched_start = None;
        for start in 0..=last_start {
            if message_digests[start] != self.anchor_digests[0] {
                continue;
            }
            let end = start + self.anchor_count;
            if message_digests[start..end] != self.anchor_digests {
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

    pub(super) fn epoch_anchor_is_present(&self, epoch: &str) -> bool {
        MessageEpochCursor::decode(epoch)
            .and_then(|cursor| cursor.find_start(&self.messages))
            .is_some()
    }

    fn apply_initial_epoch_range(&mut self) {
        let page_start = self
            .messages
            .len()
            .saturating_sub(INITIAL_EPOCH_MESSAGE_COUNT);
        let page = self.messages[page_start..].to_vec();
        self.set_initial_epoch_page(page, page_start > 0 || self.tail_window_start.is_some());
    }

    fn set_initial_epoch_page(&mut self, page: Vec<ChatItem>, has_older: bool) {
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
        self.older_cursor = has_older.then_some(cursor).flatten();
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
        self.older_cursor = (epoch_start > 0 || self.tail_window_start.is_some())
            .then(|| range.epoch.clone())
            .flatten();
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
        self.set_initial_epoch_page(self.messages[page_start..].to_vec(), page_start > 0);
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

fn message_digests(messages: &[ChatItem]) -> Option<Vec<String>> {
    messages
        .iter()
        .map(|message| message_digest(std::slice::from_ref(message)))
        .collect()
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
    fn full_initial_page_without_history_has_no_older_cursor() {
        let messages = (0..50)
            .map(|index| ChatItem::Assistant {
                text: format!("Fixture message {index}."),
            })
            .collect::<Vec<_>>();
        let epoch = MessageEpochCursor::from_messages(&messages).and_then(|cursor| cursor.encode());
        let (stable_start, stable_end, stable_digest) =
            ConversationResponse::stable_range(0, &messages);
        let mut actual = ConversationResponse {
            messages: messages.clone(),
            initial_epoch: true,
            ..ConversationResponse::default()
        };
        actual.update_metadata();
        actual.apply_range(MessageRange {
            limit: Some(100),
            ..MessageRange::default()
        });

        assert_eq!(
            actual,
            ConversationResponse {
                messages: messages.clone(),
                total: messages.len(),
                stable_start,
                stable_end,
                stable_digest,
                epoch,
                initial_epoch: true,
                ..ConversationResponse::default()
            },
        );
    }

    #[test]
    fn epoch_refresh_keeps_anchor_when_tail_changes_and_messages_append() {
        let mut messages = (0..220)
            .map(|index| ChatItem::Assistant {
                text: format!("Fixture message {index}."),
            })
            .collect::<Vec<_>>();
        let epoch_start = 120;
        let initial_page = messages[epoch_start..].to_vec();
        let epoch = MessageEpochCursor::from_messages(&initial_page)
            .and_then(|cursor| cursor.encode())
            .unwrap();
        let (initial_stable_start, initial_stable_end, initial_stable_digest) =
            ConversationResponse::stable_range(0, &initial_page);
        messages[219] = ChatItem::Assistant {
            text: "Updated fixture tail message.".to_owned(),
        };
        messages.push(ChatItem::Assistant {
            text: "Appended fixture message.".to_owned(),
        });

        let mut actual_refresh = ConversationResponse {
            messages: messages.clone(),
            ..ConversationResponse::default()
        };
        actual_refresh.update_metadata();
        actual_refresh.apply_range(MessageRange {
            from: Some(0),
            stable_start: initial_stable_start,
            stable_end: initial_stable_end,
            stable_digest: initial_stable_digest,
            epoch: Some(epoch.clone()),
            older_cursor: Some(epoch.clone()),
            ..MessageRange::default()
        });

        let refreshed_epoch_messages = messages[epoch_start..].to_vec();
        let refreshed_tail = refreshed_epoch_messages[80..].to_vec();
        let (stable_start, stable_end, stable_digest) =
            ConversationResponse::stable_range(0, &refreshed_epoch_messages);

        let mut actual_older = ConversationResponse {
            messages: messages.clone(),
            ..ConversationResponse::default()
        };
        actual_older.update_metadata();
        actual_older.apply_range(MessageRange {
            limit: Some(100),
            epoch: Some(epoch.clone()),
            older_cursor: Some(epoch.clone()),
            ..MessageRange::default()
        });

        let older_page = messages[20..epoch_start].to_vec();
        let older_epoch_messages = messages[20..].to_vec();
        let older_cursor =
            MessageEpochCursor::from_messages(&older_page).and_then(|cursor| cursor.encode());
        let (older_stable_start, older_stable_end, older_stable_digest) =
            ConversationResponse::stable_range(0, &older_epoch_messages);
        assert_eq!(
            (actual_refresh, actual_older),
            (
                ConversationResponse {
                    start: 80,
                    total: refreshed_epoch_messages.len(),
                    messages: refreshed_tail,
                    active_question_index: active_question_index(&refreshed_epoch_messages),
                    stable_start,
                    stable_end,
                    stable_digest,
                    epoch: Some(epoch.clone()),
                    older_cursor: Some(epoch),
                    ..ConversationResponse::default()
                },
                ConversationResponse {
                    start: 0,
                    total: older_epoch_messages.len(),
                    messages: older_page.clone(),
                    active_question_index: active_question_index(&older_epoch_messages),
                    stable_start: older_stable_start,
                    stable_end: older_stable_end,
                    stable_digest: older_stable_digest,
                    epoch: older_cursor.clone(),
                    older_cursor,
                    ..ConversationResponse::default()
                },
            ),
        );
    }
}
