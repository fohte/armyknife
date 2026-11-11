use std::path::Path;

use serde_json::Value;

use super::ChatItem;

pub(super) fn add_descriptions(transcript_path: &Path, messages: &mut [ChatItem]) {
    for message in messages {
        let ChatItem::SubagentReport {
            agent_id,
            description,
            ..
        } = message
        else {
            continue;
        };
        *description = description_for(transcript_path, agent_id);
    }
}

fn description_for(transcript_path: &Path, agent_id: &str) -> Option<String> {
    if agent_id.is_empty()
        || agent_id
            .chars()
            .any(|character| matches!(character, '/' | '\\'))
    {
        return None;
    }
    let metadata_path = transcript_path
        .parent()?
        .join("subagents")
        .join(format!("agent-{agent_id}.meta.json"));
    let metadata = std::fs::read(metadata_path).ok()?;
    let metadata: Value = serde_json::from_slice(&metadata).ok()?;
    metadata
        .get("description")
        .and_then(Value::as_str)
        .filter(|description| !description.trim().is_empty())
        .map(str::to_owned)
}
