pub(super) mod classifier;
pub(super) mod claude;
pub(super) mod codex;
pub(super) mod queue;

use std::collections::BTreeSet;
use std::io::BufRead;
use std::path::Path;

use super::{ChatItem, ParentSession, ToolOperation, UserContentPart};

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

#[derive(Default)]
pub(super) struct ToolSummary {
    actions: usize,
    edited_files: BTreeSet<String>,
    operations: Vec<ToolOperation>,
}

impl ToolSummary {
    pub(super) fn record(&mut self, name: &str, input: Option<&serde_json::Value>) {
        if classifier::is_peer_notify_tool(name, input) {
            return;
        }
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

pub(super) fn append_text(
    items: &mut Vec<ChatItem>,
    text: String,
    is_user: bool,
    parent: Option<&ParentSession>,
) {
    let image_directory = super::super::image_attachments::storage_directory().ok();
    append_text_with_image_directory(items, text, is_user, parent, image_directory.as_deref());
}

fn append_text_with_image_directory(
    items: &mut Vec<ChatItem>,
    text: String,
    is_user: bool,
    parent: Option<&ParentSession>,
    image_directory: Option<&Path>,
) {
    if text.trim().is_empty() {
        return;
    }
    if is_user {
        append_text_with_user_marker_and_image_directory(
            items,
            text,
            parent,
            None,
            Vec::new(),
            image_directory,
        );
    } else {
        items.push(ChatItem::Assistant { text });
    }
}

fn append_text_with_user_marker_and_image_directory(
    items: &mut Vec<ChatItem>,
    text: String,
    parent: Option<&ParentSession>,
    marker: Option<&str>,
    images: Vec<UserContentPart>,
    image_directory: Option<&Path>,
) {
    let classification =
        classifier::classify_inbound(&serde_json::json!({"type":"user"}), &text, marker);
    append_classification(items, classification, images, image_directory, parent);
}

pub(in crate::commands::agent::watch_web::messages::parser) fn append_peer_out(
    items: &mut Vec<ChatItem>,
    peer: classifier::PeerOut,
) {
    items.push(peer_item(
        peer.session_id,
        super::PeerDirection::To,
        None,
        peer.text,
        Vec::new(),
        None,
    ));
}

fn append_classification(
    items: &mut Vec<ChatItem>,
    classification: classifier::MessageKind,
    images: Vec<UserContentPart>,
    image_directory: Option<&Path>,
    parent: Option<&ParentSession>,
) {
    match classification {
        classifier::MessageKind::User { text } if !text.trim().is_empty() || !images.is_empty() => {
            items.push(user_item_with_image_directory(
                text,
                None,
                images,
                image_directory,
            ));
        }
        classifier::MessageKind::PeerIn { session_id, text }
            if !text.trim().is_empty() || !images.is_empty() =>
        {
            let session_title = parent
                .filter(|parent| parent.session_id == session_id)
                .map(|parent| parent.title.clone());
            items.push(peer_item(
                session_id,
                super::PeerDirection::From,
                session_title,
                text,
                images,
                image_directory,
            ));
        }
        classifier::MessageKind::Event { kind, text, failed } => {
            if !text.trim().is_empty() {
                items.push(ChatItem::Event {
                    kind: match kind {
                        classifier::EventKind::Armyknife => super::EventKind::Armyknife,
                        classifier::EventKind::System => super::EventKind::System,
                        classifier::EventKind::Hook => super::EventKind::Hook,
                    },
                    text,
                    failed,
                });
            }
        }
        classifier::MessageKind::User { .. }
        | classifier::MessageKind::PeerIn { .. }
        | classifier::MessageKind::Hidden => {}
    }
}

pub(in crate::commands::agent::watch_web::messages::parser) fn append_record_with_user_marker_and_image_directory(
    items: &mut Vec<ChatItem>,
    record: &serde_json::Value,
    text: String,
    marker: Option<&str>,
    images: Vec<UserContentPart>,
    image_directory: Option<&Path>,
    parent: Option<&ParentSession>,
) {
    let classification = classifier::classify_inbound(record, &text, marker);
    append_classification(items, classification, images, image_directory, parent);
}

pub(in crate::commands::agent::watch_web::messages::parser) fn append_classification_with_image_directory(
    items: &mut Vec<ChatItem>,
    classification: classifier::MessageKind,
    images: Vec<UserContentPart>,
    image_directory: Option<&Path>,
    parent: Option<&ParentSession>,
) {
    append_classification(items, classification, images, image_directory, parent);
}

fn peer_item(
    session_id: String,
    direction: super::PeerDirection,
    session_title: Option<String>,
    text: String,
    images: Vec<UserContentPart>,
    image_directory: Option<&Path>,
) -> ChatItem {
    let (text, content) = user_content_parts_with_image_directory(text, images, image_directory);
    let content = content.unwrap_or_else(|| vec![UserContentPart::Text { text }]);
    ChatItem::Peer {
        direction,
        session_id,
        session_title,
        content,
    }
}

fn user_item_with_image_directory(
    text: String,
    from: Option<String>,
    images: Vec<UserContentPart>,
    image_directory: Option<&Path>,
) -> ChatItem {
    let (text, content) = user_content_parts_with_image_directory(text, images, image_directory);
    match content {
        Some(content) => ChatItem::UserWithContent { content, from },
        None => ChatItem::User { text, from },
    }
}

fn user_content_parts_with_image_directory(
    mut text: String,
    mut images: Vec<UserContentPart>,
    image_directory: Option<&Path>,
) -> (String, Option<Vec<UserContentPart>>) {
    if let Some(directory) = image_directory
        && let Some((visible_text, references)) =
            super::super::image_attachments::extract_references(&text, directory)
    {
        text = visible_text;
        images.extend(references.into_iter().map(|reference| match reference {
            super::super::image_attachments::Reference::Image { source, available } => {
                UserContentPart::Image { source, available }
            }
            super::super::image_attachments::Reference::File { name } => {
                UserContentPart::File { name }
            }
        }));
    }

    if images.is_empty() {
        return (text, None);
    }

    let mut content = Vec::new();
    let mut remaining_images = images.into_iter();
    let mut cursor = 0;
    while let Some((start, end)) = image_placeholder(&text[cursor..]) {
        let Some(image) = remaining_images.next() else {
            break;
        };
        let start = cursor + start;
        let end = cursor + end;
        if start > cursor {
            content.push(UserContentPart::Text {
                text: text[cursor..start].to_owned(),
            });
        }
        content.push(image);
        cursor = end;
    }
    if cursor < text.len() {
        content.push(UserContentPart::Text {
            text: text[cursor..].to_owned(),
        });
    }
    content.extend(remaining_images);
    (text, Some(content))
}

fn image_placeholder(text: &str) -> Option<(usize, usize)> {
    let mut search_from = 0;
    while let Some(offset) = text[search_from..].find("[Image #") {
        let start = search_from + offset;
        let number_start = start + "[Image #".len();
        let number_end = text[number_start..].find(']')? + number_start;
        if !text[number_start..number_end].is_empty()
            && text[number_start..number_end]
                .chars()
                .all(|character| character.is_ascii_digit())
        {
            return Some((start, number_end + 1));
        }
        search_from = start + "[Image #".len();
    }
    None
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use indoc::formatdoc;
    use rstest::{fixture, rstest};

    use super::{
        ChatItem, ToolSummary, UserContentPart, append_text_with_image_directory,
        append_text_with_user_marker_and_image_directory, operation_target,
    };

    #[fixture]
    fn saved_image_directory() -> (tempfile::TempDir, PathBuf) {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(
            directory
                .path()
                .join("00000000-0000-4000-8000-000000000001.png"),
            [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A],
        )
        .unwrap();
        let image_directory = directory.path().canonicalize().unwrap();
        (directory, image_directory)
    }
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
    #[rstest]
    #[case::transcript_user(false)]
    #[case::queued_user(true)]
    fn extracts_saved_images_from_unmarked_user_paths(
        saved_image_directory: (tempfile::TempDir, PathBuf),
        #[case] is_queued: bool,
    ) {
        let (_directory, image_directory) = saved_image_directory;
        let image_path = image_directory.join("00000000-0000-4000-8000-000000000001.png");
        let text = formatdoc! {"
            Inspect this image.

            Attached image(s):
            - {}
        ", image_path.display()};
        let mut items = Vec::new();

        if is_queued {
            append_text_with_user_marker_and_image_directory(
                &mut items,
                text,
                None,
                None,
                Vec::new(),
                Some(&image_directory),
            );
        } else {
            append_text_with_image_directory(&mut items, text, true, None, Some(&image_directory));
        }

        assert_eq!(
            items,
            vec![ChatItem::UserWithContent {
                content: vec![
                    UserContentPart::Text {
                        text: "Inspect this image.".to_string(),
                    },
                    UserContentPart::Image {
                        source: "/api/images/00000000-0000-4000-8000-000000000001.png".to_string(),
                        available: true,
                    },
                ],
                from: None,
            }]
        );
    }

    #[rstest]
    fn extracts_images_and_file_names_from_generic_user_paths(
        saved_image_directory: (tempfile::TempDir, PathBuf),
    ) {
        let (_directory, image_directory) = saved_image_directory;
        let image_name = "00000000-0000-4000-8000-000000000005~image~screenshot.png";
        let file_name = "00000000-0000-4000-8000-000000000006~file~design-mock.html";
        std::fs::write(
            image_directory.join(image_name),
            [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A],
        )
        .unwrap();
        std::fs::write(image_directory.join(file_name), b"<main>Mock</main>").unwrap();
        let text = formatdoc! {"
            Inspect the attachments.

            Attached file(s):
            - {}
            - {}
        ",
            image_directory.join(image_name).display(),
            image_directory.join(file_name).display(),
        };
        let mut items = Vec::new();

        append_text_with_image_directory(&mut items, text, true, None, Some(&image_directory));

        assert_eq!(
            items,
            vec![ChatItem::UserWithContent {
                content: vec![
                    UserContentPart::Text {
                        text: "Inspect the attachments.".to_string(),
                    },
                    UserContentPart::Image {
                        source: format!("/api/images/{image_name}"),
                        available: true,
                    },
                    UserContentPart::File {
                        name: "design-mock.html".to_string(),
                    },
                ],
                from: None,
            }]
        );
    }

    #[test]
    fn retains_the_attachment_listing_when_storage_directory_resolution_fails() {
        let text = formatdoc! {"
            Inspect this image.

            Attached image(s):
            - /unavailable/00000000-0000-4000-8000-000000000001.png
        "};
        let mut items = Vec::new();

        append_text_with_image_directory(&mut items, text.clone(), true, None, None);

        assert_eq!(items, vec![ChatItem::User { text, from: None }]);
    }

    #[test]
    fn preserves_notice_text_quoted_by_the_user_before_the_transport_notice() {
        let marker = "⟦armyknife-user:fixture-marker-cobalt⟧";
        let notice = "This came from another Claude session — this fictional notice is retained.";
        let message = formatdoc! {"
            {marker}
            This message was typed by the human user in armyknife watch-web.
            The user quoted this notice:

            {notice}

            {notice}"
        };
        let expected = formatdoc! {"
            The user quoted this notice:

            {notice}"
        };

        assert_eq!(
            super::classifier::classify_inbound(
                &serde_json::json!({"type":"user"}),
                &message,
                Some(marker),
            ),
            super::classifier::MessageKind::User { text: expected },
        );
    }

    #[test]
    fn tool_targets_preserve_their_full_length() {
        let target = "x".repeat(241);
        let command_input = serde_json::json!({
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
        let changed_paths = serde_json::json!({ "changes": paths });

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
