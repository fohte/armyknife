pub(super) mod claude;
pub(super) mod codex;

use std::collections::BTreeSet;
use std::io::BufRead;

use super::ChatItem;

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
        if matches!(
            name,
            "apply_patch" | "write_file" | "edit_file" | "fileChange"
        ) && let Some(detail) = detail
        {
            if let Some(path) = detail
                .get("file_path")
                .or_else(|| detail.get("path"))
                .and_then(serde_json::Value::as_str)
            {
                self.edited_files.insert(path.to_owned());
            }
            if let Some(changes) = detail.get("changes").and_then(serde_json::Value::as_array) {
                self.edited_files
                    .extend(changes.iter().filter_map(|change| {
                        change
                            .get("path")
                            .and_then(serde_json::Value::as_str)
                            .map(str::to_owned)
                    }));
            }
        }
    }

    pub(super) fn flush_into(&mut self, items: &mut Vec<ChatItem>) {
        if self.actions == 0 {
            return;
        }
        let summary = if self.edited_files.is_empty() {
            format!("{} actions", self.actions)
        } else {
            let noun = if self.edited_files.len() == 1 {
                "file"
            } else {
                "files"
            };
            format!(
                "{} actions · {} {noun} edited",
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
    from: Option<&str>,
) {
    if text.trim().is_empty() {
        return;
    }
    if is_user {
        items.push(ChatItem::User {
            text,
            from: from.map(str::to_owned),
        });
    } else {
        items.push(ChatItem::Assistant { text });
    }
}
