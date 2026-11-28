use serde_json::Value;

use super::super::transcript_branch::is_inactive_branch;
use super::ClaudeParserState;

impl ClaudeParserState {
    pub(super) fn is_queued_peer_delivery(&mut self, record: &Value, uuid: Option<&str>) -> bool {
        let Some(uuid) = uuid.filter(|_| is_queued_peer_delivery_record(record)) else {
            return false;
        };
        if !self.forward_parent_uuids.contains(uuid) {
            return false;
        }
        self.transcript_children.get(uuid).is_some_and(|children| {
            children
                .iter()
                .any(|child_uuid| is_inactive_branch(child_uuid, &mut self.transcript_nodes))
        })
    }
}

fn is_queued_peer_delivery_record(record: &Value) -> bool {
    record.get("type").and_then(Value::as_str) == Some("user")
        && record.get("isMeta").and_then(Value::as_bool) == Some(true)
        && record
            .get("origin")
            .and_then(|origin| origin.get("kind"))
            .and_then(Value::as_str)
            == Some("peer")
        && record.get("message").is_some_and(|message| {
            let text = super::super::user_input::user_text(message);
            super::super::super::super::queued_prompt::id_from_content(&text).is_some()
        })
}
