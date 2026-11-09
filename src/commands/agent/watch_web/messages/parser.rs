pub(super) mod claude;
pub(super) mod codex;

use std::collections::BTreeSet;
use std::io::BufRead;

use super::{ChatItem, ParentSession};

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

#[derive(Default)]
pub(super) struct ToolSummary {
    actions: usize,
    edited_files: BTreeSet<String>,
}

impl ToolSummary {
    pub(super) fn record(&mut self, name: &str, input: Option<&serde_json::Value>) {
        self.actions += 1;
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
        items.push(ChatItem::ToolSummary { summary });
        *self = Self::default();
    }
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
            }]
        );
    }
}
