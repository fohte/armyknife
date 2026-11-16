use std::path::Path;

use super::super::{ChatItem, ToolGroupItem, ToolOperation};

#[derive(Clone, Default)]
pub(in crate::commands::agent::watch_web::messages) struct ToolSummary {
    actions: usize,
    edited_files: Vec<String>,
    operations: Vec<ToolOperation>,
    group_items: Vec<ToolGroupItem>,
}

impl ToolSummary {
    pub(super) fn record(&mut self, name: &str, input: Option<&serde_json::Value>, failed: bool) {
        self.actions += 1;
        let operation = ToolOperation {
            tool: name.to_owned(),
            target: operation_target(name, input),
            failed,
        };
        self.group_items.push(ToolGroupItem::ToolUse {
            tool: operation.tool.clone(),
            target: operation.target.clone(),
            failed: operation.failed,
        });
        self.operations.push(operation);
        if matches!(name, "Edit" | "Write" | "MultiEdit" | "NotebookEdit")
            && let Some(path) = input
                .and_then(|value| value.get("file_path").or_else(|| value.get("path")))
                .and_then(serde_json::Value::as_str)
            && !self.edited_files.iter().any(|file| file == path)
        {
            self.edited_files.push(path.to_owned());
        }
    }

    pub(super) fn record_codex_action(&mut self, action: CodexToolSummaryRecord) {
        self.actions += 1;
        self.group_items.push(ToolGroupItem::ToolUse {
            tool: action.operation.tool.clone(),
            target: action.operation.target.clone(),
            failed: action.operation.failed,
        });
        self.operations.push(action.operation);
        self.record_codex_edited_files(action.edited_files);
    }

    pub(super) fn record_thinking(&mut self, text: String, duration_ms: Option<u64>) {
        self.group_items.push(ToolGroupItem::Thinking {
            text,
            thinking_duration_ms: duration_ms,
        });
    }

    pub(super) fn take_pending_thinking(&mut self) -> Vec<ToolGroupItem> {
        if self.actions > 0 {
            return Vec::new();
        }
        std::mem::take(&mut self.group_items)
    }

    pub(super) fn record_codex_edited_files(&mut self, paths: Vec<String>) {
        for path in paths {
            if !self.edited_files.contains(&path) {
                self.edited_files.push(path);
            }
        }
    }

    pub(super) fn flush_into(&mut self, items: &mut Vec<ChatItem>) {
        if self.actions == 0 {
            items.extend(
                std::mem::take(&mut self.group_items)
                    .into_iter()
                    .filter_map(|item| match item {
                        ToolGroupItem::Thinking {
                            text,
                            thinking_duration_ms,
                        } => Some(ChatItem::Thinking {
                            text,
                            thinking_duration_ms,
                        }),
                        ToolGroupItem::ToolUse { .. } => None,
                    }),
            );
            *self = Self::default();
            return;
        }
        let mut summary_parts = Vec::new();
        if !self.edited_files.is_empty() {
            let files = self
                .edited_files
                .iter()
                .map(|path| {
                    Path::new(path)
                        .file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or(path)
                })
                .collect::<Vec<_>>()
                .join(", ");
            summary_parts.push(format!("Edit {files}"));
        }
        let mut tool_counts = Vec::<(&str, usize)>::new();
        for operation in &self.operations {
            if self.edited_files.is_empty() || !is_file_edit_tool(&operation.tool) {
                if let Some((_, count)) = tool_counts
                    .iter_mut()
                    .find(|(tool, _)| *tool == operation.tool.as_str())
                {
                    *count += 1;
                } else {
                    tool_counts.push((&operation.tool, 1));
                }
            }
        }
        summary_parts.extend(
            tool_counts
                .into_iter()
                .map(|(tool, count)| format!("{tool} ×{count}")),
        );
        let failed_count = self
            .operations
            .iter()
            .filter(|operation| operation.failed)
            .count();
        let summary = if failed_count == 0 {
            summary_parts.join("  ")
        } else {
            format!("{} · {failed_count} failed", summary_parts.join("  "))
        };
        let operations = std::mem::take(&mut self.operations);
        let group_items = std::mem::take(&mut self.group_items);
        if group_items
            .iter()
            .any(|item| matches!(item, ToolGroupItem::Thinking { .. }))
        {
            items.push(ChatItem::ToolSummaryWithThinking {
                summary,
                operations,
                items: group_items,
            });
        } else {
            items.push(ChatItem::ToolSummary {
                summary,
                operations,
            });
        }
        *self = Self::default();
    }
}

#[derive(Clone)]
pub(super) struct CodexToolSummaryRecord {
    operation: ToolOperation,
    edited_files: Vec<String>,
}

pub(super) fn codex_tool_summary_record(
    name: &str,
    detail: Option<&serde_json::Value>,
    failed: bool,
) -> CodexToolSummaryRecord {
    let tool = if matches!(name, "McpToolCall" | "mcpToolCall") {
        detail
            .and_then(|value| value.get("tool").or_else(|| value.get("name")))
            .and_then(serde_json::Value::as_str)
            .unwrap_or(name)
    } else {
        name
    };
    CodexToolSummaryRecord {
        operation: ToolOperation {
            tool: tool.to_owned(),
            target: operation_target(tool, detail),
            failed,
        },
        edited_files: codex_edited_file_paths(name, detail),
    }
}

pub(super) fn codex_edited_file_paths(
    name: &str,
    detail: Option<&serde_json::Value>,
) -> Vec<String> {
    let Some(detail) = detail.filter(|_| {
        matches!(
            name,
            "apply_patch" | "write_file" | "edit_file" | "FileChange" | "fileChange"
        )
    }) else {
        return Vec::new();
    };
    let mut paths = Vec::new();
    if let Some(path) = detail
        .get("file_path")
        .or_else(|| detail.get("path"))
        .and_then(serde_json::Value::as_str)
    {
        paths.push(path.to_owned());
    }
    if let Some(changes) = detail.get("changes") {
        match changes {
            serde_json::Value::Object(changes) => {
                paths.extend(changes.keys().cloned());
            }
            serde_json::Value::Array(changes) => {
                paths.extend(
                    changes
                        .iter()
                        .filter_map(|change| change.get("path").and_then(serde_json::Value::as_str))
                        .map(str::to_owned),
                );
            }
            _ => {}
        }
    }
    let mut unique = Vec::with_capacity(paths.len());
    for path in paths {
        if !unique.contains(&path) {
            unique.push(path);
        }
    }
    unique
}

fn is_file_edit_tool(name: &str) -> bool {
    matches!(
        name,
        "Edit"
            | "Write"
            | "MultiEdit"
            | "NotebookEdit"
            | "apply_patch"
            | "write_file"
            | "edit_file"
            | "FileChange"
            | "fileChange"
    )
}

pub(super) fn operation_target(name: &str, detail: Option<&serde_json::Value>) -> Option<String> {
    let nested_detail = detail.and_then(|value| {
        value
            .get("arguments")
            .or_else(|| value.get("input"))
            .or_else(|| value.get("parameters"))
    });
    if let Some(target) = tool_target(name, detail).or_else(|| tool_target(name, nested_detail)) {
        return Some(target);
    }
    if matches!(name, "exec" | "shell" | "CommandExecution")
        && let Some(target) = detail.and_then(serde_json::Value::as_str)
    {
        return Some(target.to_owned());
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
    Some(paths.join(", "))
}

pub(super) fn tool_target(name: &str, input: Option<&serde_json::Value>) -> Option<String> {
    let input = input?;
    if matches!(name, "Agent" | "Task") {
        let subagent_type = input
            .get("subagent_type")
            .and_then(serde_json::Value::as_str);
        let description = input.get("description").and_then(serde_json::Value::as_str);
        return match (subagent_type, description) {
            (Some(subagent_type), Some(description)) => {
                Some(format!("{subagent_type} · {description}"))
            }
            (Some(subagent_type), None) => Some(subagent_type.to_owned()),
            (None, Some(description)) => Some(description.to_owned()),
            (None, None) => None,
        };
    }
    let target = ["command", "file_path", "path", "pattern", "url", "query"]
        .into_iter()
        .find_map(|key| input.get(key).and_then(serde_json::Value::as_str))?;
    Some(target.to_owned())
}

#[cfg(test)]
mod tests {
    use rstest::rstest;
    use serde_json::json;

    use super::{ToolSummary, operation_target};
    use crate::commands::agent::watch_web::messages::{ChatItem, ToolOperation};

    #[rstest]
    #[case::singular(1, "Read ×1")]
    #[case::plural(2, "Read ×2")]
    fn summary_groups_repeated_tool_calls_by_count(#[case] count: usize, #[case] expected: &str) {
        let mut summary = ToolSummary::default();
        for _ in 0..count {
            summary.record("Read", None, false);
        }
        let mut items = Vec::new();

        summary.flush_into(&mut items);

        assert_eq!(
            items,
            vec![ChatItem::ToolSummary {
                summary: expected.to_string(),
                operations: (0..count)
                    .map(|_| ToolOperation {
                        tool: "Read".to_string(),
                        target: None,
                        failed: false,
                    })
                    .collect(),
            }]
        );
    }

    #[test]
    fn summarizes_edited_files_tool_counts_and_failed_calls() {
        let mut summary = ToolSummary::default();
        summary.record("Edit", Some(&json!({"file_path": "src/queue.rs"})), false);
        summary.record("Edit", Some(&json!({"file_path": "src/mod.rs"})), false);
        for _ in 0..3 {
            summary.record("Read", None, false);
        }
        summary.record("Grep", None, false);
        summary.record("Bash", None, true);
        let mut items = Vec::new();

        summary.flush_into(&mut items);

        assert_eq!(
            items,
            vec![ChatItem::ToolSummary {
                summary: "Edit queue.rs, mod.rs  Read ×3  Grep ×1  Bash ×1 · 1 failed".to_string(),
                operations: vec![
                    ToolOperation {
                        tool: "Edit".to_string(),
                        target: Some("src/queue.rs".to_string()),
                        failed: false,
                    },
                    ToolOperation {
                        tool: "Edit".to_string(),
                        target: Some("src/mod.rs".to_string()),
                        failed: false,
                    },
                    ToolOperation {
                        tool: "Read".to_string(),
                        target: None,
                        failed: false,
                    },
                    ToolOperation {
                        tool: "Read".to_string(),
                        target: None,
                        failed: false,
                    },
                    ToolOperation {
                        tool: "Read".to_string(),
                        target: None,
                        failed: false,
                    },
                    ToolOperation {
                        tool: "Grep".to_string(),
                        target: None,
                        failed: false,
                    },
                    ToolOperation {
                        tool: "Bash".to_string(),
                        target: None,
                        failed: true,
                    },
                ],
            }]
        );
    }

    #[test]
    fn tool_targets_preserve_their_full_length() {
        let target = "x".repeat(241);
        let command_input = json!({
            "command": target,
            "description": "Run the tests",
        });
        let raw_command = serde_json::Value::String(target.clone());
        let paths = (0..40)
            .map(|index| {
                (
                    format!("file-{index:02}-long-path.rs"),
                    serde_json::Value::Null,
                )
            })
            .collect::<serde_json::Map<_, _>>();
        let expected_paths = paths.keys().cloned().collect::<Vec<_>>().join(", ");
        let changed_paths = json!({ "changes": paths });

        assert_eq!(
            vec![
                operation_target("Bash", Some(&command_input)),
                operation_target("exec", Some(&raw_command)),
                operation_target("FileChange", Some(&changed_paths)),
            ],
            vec![Some(target.clone()), Some(target), Some(expected_paths)],
        );
    }
}
