pub(super) mod claude;
pub(super) mod codex;

use std::collections::BTreeSet;
use std::io::BufRead;

use super::{ChatItem, ParentSession, ToolOperation};

const PEER_MESSAGE_HEADER_SEPARATOR: &str = concat!("\n", "\n");
const PEER_MESSAGE_END: &str = concat!("\n", "</peer-message>");

pub(super) fn for_each_jsonl_record(
    reader: impl BufRead,
    mut visit: impl FnMut(serde_json::Value),
) -> std::io::Result<()> {
    let mut reader = reader;
    let mut line = Vec::new();
    loop {
        line.clear();
        match reader.read_until(b'\n', &mut line) {
            Ok(0) => break,
            Err(error) => return Err(error),
            Ok(_) => {}
        }
        let Ok(line) = std::str::from_utf8(&line) else {
            continue;
        };
        if let Ok(record) = serde_json::from_str(line) {
            visit(record);
        }
    }
    Ok(())
}

pub(super) fn tool_target(input: Option<&serde_json::Value>) -> Option<String> {
    let input = input?;
    let target = ["command", "file_path", "path", "pattern", "url", "query"]
        .into_iter()
        .find_map(|key| input.get(key).and_then(serde_json::Value::as_str))?;
    Some(target.chars().take(240).collect())
}

#[derive(Default)]
pub(super) struct ToolSummary {
    actions: usize,
    edited_files: BTreeSet<String>,
    operations: Vec<ToolOperation>,
}

impl ToolSummary {
    pub(super) fn record(&mut self, name: &str, input: Option<&serde_json::Value>) {
        self.actions += 1;
        self.operations.push(ToolOperation {
            tool: name.to_owned(),
            target: operation_target(name, input),
        });
        if matches!(name, "Edit" | "Write" | "MultiEdit" | "NotebookEdit")
            && let Some(path) = input
                .and_then(|value| value.get("file_path").or_else(|| value.get("path")))
                .and_then(serde_json::Value::as_str)
        {
            self.edited_files.insert(path.to_owned());
        }
    }

    pub(super) fn record_codex(&mut self, name: &str, detail: Option<&serde_json::Value>) {
        self.actions += 1;
        let tool = if matches!(name, "McpToolCall" | "mcpToolCall") {
            detail
                .and_then(|value| value.get("tool").or_else(|| value.get("name")))
                .and_then(serde_json::Value::as_str)
                .unwrap_or(name)
        } else {
            name
        };
        self.operations.push(ToolOperation {
            tool: tool.to_owned(),
            target: operation_target(tool, detail),
        });
        self.record_codex_files(name, detail);
    }

    pub(super) fn record_codex_files(&mut self, name: &str, detail: Option<&serde_json::Value>) {
        if matches!(
            name,
            "apply_patch" | "write_file" | "edit_file" | "FileChange" | "fileChange"
        ) && let Some(detail) = detail
        {
            if let Some(path) = detail
                .get("file_path")
                .or_else(|| detail.get("path"))
                .and_then(serde_json::Value::as_str)
            {
                self.edited_files.insert(path.to_owned());
            }
            if let Some(changes) = detail.get("changes") {
                match changes {
                    serde_json::Value::Object(changes) => {
                        self.edited_files.extend(changes.keys().cloned());
                    }
                    serde_json::Value::Array(changes) => {
                        self.edited_files
                            .extend(changes.iter().filter_map(|change| {
                                change
                                    .get("path")
                                    .and_then(serde_json::Value::as_str)
                                    .map(str::to_owned)
                            }));
                    }
                    _ => {}
                }
            }
        }
    }

    pub(super) fn flush_into(&mut self, items: &mut Vec<ChatItem>) {
        if self.actions == 0 {
            return;
        }
        let action = if self.actions == 1 {
            "action"
        } else {
            "actions"
        };
        let summary = if self.edited_files.is_empty() {
            format!("{} {action}", self.actions)
        } else {
            let noun = if self.edited_files.len() == 1 {
                "file"
            } else {
                "files"
            };
            format!(
                "{} {action} · {} {noun} edited",
                self.actions,
                self.edited_files.len()
            )
        };
        items.push(ChatItem::ToolSummary {
            summary,
            operations: std::mem::take(&mut self.operations),
        });
        *self = Self::default();
    }
}

fn operation_target(name: &str, detail: Option<&serde_json::Value>) -> Option<String> {
    let nested_detail = detail.and_then(|value| {
        value
            .get("arguments")
            .or_else(|| value.get("input"))
            .or_else(|| value.get("parameters"))
    });
    if let Some(target) = tool_target(detail).or_else(|| tool_target(nested_detail)) {
        return Some(target);
    }
    if matches!(name, "exec" | "shell" | "CommandExecution")
        && let Some(target) = detail.and_then(serde_json::Value::as_str)
    {
        return Some(target.chars().take(240).collect());
    }

    let paths = detail
        .and_then(|value| value.get("changes"))
        .map(|changes| match changes {
            serde_json::Value::Object(changes) => changes.keys().cloned().collect::<Vec<_>>(),
            serde_json::Value::Array(changes) => changes
                .iter()
                .filter_map(|change| {
                    change
                        .get("path")
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_owned)
                })
                .collect(),
            _ => Vec::new(),
        })?;
    if paths.is_empty() {
        return None;
    }
    Some(paths.join(", ").chars().take(240).collect())
}

pub(super) fn append_text(
    items: &mut Vec<ChatItem>,
    text: String,
    is_user: bool,
    parent: Option<&ParentSession>,
) {
    if text.trim().is_empty() {
        return;
    }
    if is_user {
        // A child session can receive direct input as well as peer messages.
        let (text, from) = if let Some((sender_id, body)) = peer_message_parts(&text) {
            (
                body.to_owned(),
                parent
                    .filter(|parent| parent.session_id == sender_id)
                    .map(|parent| parent.title.clone()),
            )
        } else {
            (text, None)
        };
        if text.trim().is_empty() {
            return;
        }
        items.push(ChatItem::User { text, from });
    } else {
        items.push(ChatItem::Assistant { text });
    }
}

pub(super) fn peer_message_item(message: &str, parent: Option<&ParentSession>) -> Option<ChatItem> {
    let (sender_id, body) = peer_message_parts(message)?;
    let parent = parent.filter(|parent| parent.session_id == sender_id)?;
    if body.trim().is_empty() {
        return None;
    }
    Some(ChatItem::User {
        text: body.to_owned(),
        from: Some(parent.title.clone()),
    })
}

pub(super) fn append_text_with_user_marker(
    items: &mut Vec<ChatItem>,
    text: String,
    parent: Option<&ParentSession>,
    marker: Option<&str>,
) {
    if let Some(marker) = marker
        && let Some((_, body)) = peer_message_parts(&text)
        && let Some(body) =
            crate::commands::agent::watch_web_user_marker::strip_message(marker, body)
    {
        if !body.trim().is_empty() {
            items.push(ChatItem::User {
                text: body.to_owned(),
                from: None,
            });
        }
        return;
    }
    append_text(items, text, true, parent);
}

pub(super) fn peer_message_item_with_user_marker(
    message: &str,
    parent: Option<&ParentSession>,
    marker: Option<&str>,
) -> Option<ChatItem> {
    if let Some(marker) = marker
        && let Some((_, body)) = peer_message_parts(message)
        && let Some(body) =
            crate::commands::agent::watch_web_user_marker::strip_message(marker, body)
    {
        return (!body.trim().is_empty()).then(|| ChatItem::User {
            text: body.to_owned(),
            from: None,
        });
    }
    peer_message_item(message, parent)
}

fn peer_message_parts(message: &str) -> Option<(&str, &str)> {
    let envelope = message.split_once("<peer-message>")?.1;
    let end = envelope.rfind(PEER_MESSAGE_END)?;
    let envelope = envelope[..end].strip_prefix('\n')?;
    let (header, body) = envelope.split_once(PEER_MESSAGE_HEADER_SEPARATOR)?;
    let mut header_lines = header.lines();
    let sender_id = header_lines.next()?.strip_prefix("- From session_id: ")?;
    if sender_id.is_empty()
        || header_lines
            .next()
            .is_some_and(|line| !line.starts_with("- Sender engine: "))
        || header_lines.next().is_some()
    {
        return None;
    }
    Some((sender_id, body))
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::{ChatItem, ToolSummary};

    #[rstest]
    #[case::singular(1, "1 action")]
    #[case::plural(2, "2 actions")]
    fn action_count_uses_the_matching_noun(#[case] count: usize, #[case] expected: &str) {
        let mut summary = ToolSummary::default();
        for _ in 0..count {
            summary.record("Read", None);
        }
        let mut items = Vec::new();

        summary.flush_into(&mut items);

        assert_eq!(
            items,
            vec![ChatItem::ToolSummary {
                summary: expected.to_string(),
                operations: (0..count)
                    .map(|_| super::super::ToolOperation {
                        tool: "Read".to_string(),
                        target: None,
                    })
                    .collect(),
            }]
        );
    }
}
