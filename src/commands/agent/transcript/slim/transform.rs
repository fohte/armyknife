use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{BufRead, BufReader, Seek, SeekFrom, Write};
use std::path::Path;

use anyhow::{Context, Result};
use serde_json::{Value, json};
use tempfile::NamedTempFile;

use super::TranscriptAnalysis;

pub(super) fn analyze_transcript(
    source: &mut File,
    snapshot_len: u64,
) -> Result<TranscriptAnalysis> {
    source
        .seek(SeekFrom::Start(0))
        .context("rewind transcript for analysis")?;
    let mut reader = BufReader::new(source);
    let mut analysis = TranscriptAnalysis {
        boundary_offset: None,
        preserved_uuids: HashSet::new(),
        read_paths_by_tool_use: HashMap::new(),
    };
    let mut line = Vec::new();
    let mut offset = 0_u64;
    while offset < snapshot_len {
        line.clear();
        let read = reader
            .read_until(b'\n', &mut line)
            .context("read transcript during analysis")?;
        if read == 0 {
            break;
        }
        let line_offset = offset;
        offset += read as u64;
        if let Ok(record) = serde_json::from_slice::<Value>(&line) {
            collect_read_paths(&record, &mut analysis);
            if is_compact_boundary(&record) {
                analysis.boundary_offset = Some(line_offset);
                analysis.preserved_uuids = preserved_uuids(&record);
            }
        }
    }
    Ok(analysis)
}

fn collect_read_paths(record: &Value, analysis: &mut TranscriptAnalysis) {
    if record.get("type").and_then(Value::as_str) != Some("assistant") {
        return;
    }
    if let Some(content) = record.pointer("/message/content").and_then(Value::as_array) {
        for block in content {
            if block.get("type").and_then(Value::as_str) != Some("tool_use")
                || block.get("name").and_then(Value::as_str) != Some("Read")
            {
                continue;
            }
            let Some(path) = block.pointer("/input/file_path").and_then(Value::as_str) else {
                continue;
            };
            if let Some(id) = block.get("id").and_then(Value::as_str) {
                analysis
                    .read_paths_by_tool_use
                    .insert(id.to_owned(), path.to_owned());
            }
        }
    }
}

pub(super) fn write_slimmed_prefix(
    source: &mut File,
    snapshot_len: u64,
    boundary_offset: u64,
    analysis: &TranscriptAnalysis,
    output: &mut NamedTempFile,
) -> Result<u64> {
    source
        .seek(SeekFrom::Start(0))
        .context("rewind transcript before slimming")?;
    let mut reader = BufReader::new(source);
    let mut line = Vec::new();
    let mut offset = 0_u64;
    while offset < snapshot_len {
        line.clear();
        let read = reader
            .read_until(b'\n', &mut line)
            .context("read transcript while slimming")?;
        if read == 0 {
            break;
        }
        if line.last() != Some(&b'\n') {
            break;
        }
        let line_offset = offset;
        offset += read as u64;
        if line_offset >= boundary_offset {
            output
                .write_all(&line)
                .context("write untouched transcript line")?;
            continue;
        }
        let Ok(mut record) = serde_json::from_slice::<Value>(&line) else {
            output
                .write_all(&line)
                .context("write invalid transcript line")?;
            continue;
        };
        if record
            .get("uuid")
            .and_then(Value::as_str)
            .is_some_and(|uuid| analysis.preserved_uuids.contains(uuid))
        {
            output
                .write_all(&line)
                .context("write preserved transcript line")?;
            continue;
        }
        match slim_record(&mut record, analysis) {
            RecordAction::Drop => {}
            RecordAction::KeepUnchanged => {
                output.write_all(&line).context("write transcript line")?;
            }
            RecordAction::KeepChanged => {
                serde_json::to_writer(&mut *output, &record)
                    .context("serialize slimmed transcript line")?;
                output
                    .write_all(b"\n")
                    .context("terminate transcript line")?;
            }
        }
    }
    Ok(offset)
}

#[derive(Debug, PartialEq, Eq)]
enum RecordAction {
    Drop,
    KeepUnchanged,
    KeepChanged,
}

fn slim_record(record: &mut Value, analysis: &TranscriptAnalysis) -> RecordAction {
    let Some(entry_type) = record.get("type").and_then(Value::as_str) else {
        return RecordAction::KeepUnchanged;
    };
    if entry_type.starts_with("api-request") || entry_type.starts_with("api_request") {
        return RecordAction::Drop;
    }

    if is_discardable_attachment(record) {
        if let Some(object) = record.as_object_mut()
            && let Some(attachment) = object.get_mut("attachment").and_then(Value::as_object_mut)
        {
            let attachment_type = attachment.get("type").cloned().unwrap_or(Value::Null);
            attachment.clear();
            attachment.insert("type".to_owned(), attachment_type);
            return RecordAction::KeepChanged;
        }
        return RecordAction::KeepUnchanged;
    }

    let image_changed = sanitize_images(record, analysis) == RecordAction::KeepChanged;
    let mut changed = image_changed;
    if let Some(tool_result) = record.get_mut("toolUseResult")
        && !tool_result.is_null()
    {
        if let Some(result) = tool_result.as_object_mut() {
            let allowed = [
                "task_id",
                "taskId",
                "background_task_id",
                "backgroundTaskId",
                "shell_id",
                "shellId",
                "agentId",
                "isAsync",
                "status",
                "answers",
                "questions",
            ];
            let before = result.len();
            result.retain(|key, _| allowed.contains(&key.as_str()));
            changed |= before != result.len();
        } else {
            *tool_result = json!({});
            changed = true;
        }
    }
    if changed {
        RecordAction::KeepChanged
    } else {
        RecordAction::KeepUnchanged
    }
}

fn is_discardable_attachment(record: &Value) -> bool {
    if record.get("type").and_then(Value::as_str) != Some("attachment") {
        return false;
    }
    let attachment_type = record
        .pointer("/attachment/type")
        .or_else(|| record.get("subtype"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    matches!(
        attachment_type,
        "invoked_skills" | "prompt_snapshot" | "instructions" | "hook_success"
    ) || attachment_type.starts_with("deferred_tools_")
}

fn sanitize_images(record: &mut Value, analysis: &TranscriptAnalysis) -> RecordAction {
    let mut changed = false;
    sanitize_value(record, analysis, None, &mut changed);
    if changed {
        RecordAction::KeepChanged
    } else {
        RecordAction::KeepUnchanged
    }
}

fn sanitize_value(
    value: &mut Value,
    analysis: &TranscriptAnalysis,
    path: Option<&str>,
    changed: &mut bool,
) {
    if let Some(object) = value.as_object_mut() {
        if object.get("type").and_then(Value::as_str) == Some("image")
            && object
                .get("source")
                .and_then(|source| source.get("type"))
                .and_then(Value::as_str)
                == Some("base64")
        {
            *value = json!({
                "type": "text",
                "text": image_placeholder(path),
            });
            *changed = true;
            return;
        }

        let local_path = object
            .get("tool_use_id")
            .and_then(Value::as_str)
            .and_then(|id| analysis.read_paths_by_tool_use.get(id).cloned())
            .or_else(|| path.map(str::to_owned));
        let file_path = object
            .get("file")
            .and_then(|file| file.get("filePath"))
            .and_then(Value::as_str)
            .map(str::to_owned);
        let image_file_path = image_file_path(object).map(str::to_owned);
        if object.get("base64").and_then(Value::as_str).is_some()
            && (object.get("type").and_then(Value::as_str) == Some("image")
                || image_file_path.is_some())
        {
            let source_path = local_path.as_deref().or(image_file_path.as_deref());
            object.insert("base64".to_owned(), json!(image_placeholder(source_path)));
            *changed = true;
        }
        for (key, child) in object.iter_mut() {
            let child_path = if key == "file" {
                file_path.as_deref().or(local_path.as_deref())
            } else {
                local_path.as_deref()
            };
            sanitize_value(child, analysis, child_path, changed);
        }
    } else if let Some(array) = value.as_array_mut() {
        for child in array {
            sanitize_value(child, analysis, path, changed);
        }
    } else if let Some(text) = value.as_str()
        && let Some(shortened) = shorten_data_uri_payloads(text, path)
    {
        *value = Value::String(shortened);
        *changed = true;
    }
}

fn image_file_path(object: &serde_json::Map<String, Value>) -> Option<&str> {
    let file_path = object.get("filePath").and_then(Value::as_str)?;
    let extension = Path::new(file_path).extension()?.to_str()?;
    [
        "avif", "bmp", "gif", "heic", "heif", "ico", "jpeg", "jpg", "png", "tif", "tiff", "webp",
    ]
    .iter()
    .any(|candidate| extension.eq_ignore_ascii_case(candidate))
    .then_some(file_path)
}

fn shorten_data_uri_payloads(text: &str, path: Option<&str>) -> Option<String> {
    const PREFIX: &str = "data:image/";
    const BASE64_SEPARATOR: &str = ";base64,";

    let bytes = text.as_bytes();
    let mut output = String::with_capacity(text.len());
    let mut copied_to = 0;
    let mut search_from = 0;
    let mut changed = false;
    while let Some(relative_start) = text[search_from..].find(PREFIX) {
        let start = search_from + relative_start;
        let Some(separator_offset) = text[start..].find(BASE64_SEPARATOR) else {
            search_from = start + PREFIX.len();
            continue;
        };
        let payload_start = start + separator_offset + BASE64_SEPARATOR.len();
        let payload_end = bytes[payload_start..]
            .iter()
            .position(|byte| !is_base64_byte(*byte))
            .map_or(bytes.len(), |offset| payload_start + offset);
        if payload_end == payload_start {
            search_from = start + PREFIX.len();
            continue;
        }

        output.push_str(&text[copied_to..start]);
        output.push_str(&image_placeholder(path));
        copied_to = payload_end;
        search_from = payload_end;
        changed = true;
    }

    if !changed {
        return None;
    }
    output.push_str(&text[copied_to..]);
    Some(output)
}

fn is_base64_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'/' | b'=')
}

fn image_placeholder(path: Option<&str>) -> String {
    match path {
        Some(path) => format!("[Image data omitted; source: {path}]"),
        None => "[Image data omitted]".to_owned(),
    }
}

fn is_compact_boundary(record: &Value) -> bool {
    record.get("type").and_then(Value::as_str) == Some("system")
        && record.get("subtype").and_then(Value::as_str) == Some("compact_boundary")
}

fn preserved_uuids(record: &Value) -> HashSet<String> {
    record
        .pointer("/compactMetadata/preservedMessages/uuids")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};

    use rstest::{fixture, rstest};
    use serde_json::{Value, json};

    use super::{RecordAction, TranscriptAnalysis, sanitize_images};

    #[fixture]
    fn analysis() -> TranscriptAnalysis {
        TranscriptAnalysis {
            boundary_offset: None,
            preserved_uuids: HashSet::new(),
            read_paths_by_tool_use: HashMap::new(),
        }
    }

    #[rstest]
    #[case::data_uri_followed_by_prompt_text(
        r#"{"type":"user","message":{"content":"data:image/png;base64,fixturepayload Keep this instruction."}}"#,
        r#"{"type":"user","message":{"content":"[Image data omitted] Keep this instruction."}}"#,
    )]
    #[case::unrelated_base64_object_keeps_its_text(
        r#"{"type":"user","message":{"content":[{"type":"metadata","base64":"fixture-value","text":"Keep this metadata text."}]}}"#,
        r#"{"type":"user","message":{"content":[{"type":"metadata","base64":"fixture-value","text":"Keep this metadata text."}]}}"#,
    )]
    #[case::image_object_keeps_its_text(
        r#"{"type":"image","base64":"fixturepayload","text":"Keep this image note."}"#,
        r#"{"type":"image","base64":"[Image data omitted]","text":"Keep this image note."}"#
    )]
    #[case::image_file_keeps_existing_text(
        r#"{"type":"user","file":{"filePath":"/fixture/source.png","base64":"fixture-payload","text":"Keep this file note."}}"#,
        r#"{"type":"user","file":{"filePath":"/fixture/source.png","base64":"[Image data omitted; source: /fixture/source.png]","text":"Keep this file note."}}"#,
    )]
    fn shortens_image_values_without_rewriting_other_text(
        analysis: TranscriptAnalysis,
        #[case] input: &str,
        #[case] expected: &str,
    ) {
        let mut record = serde_json::from_str::<Value>(input).expect("fixture record should parse");

        sanitize_images(&mut record, &analysis);

        assert_eq!(
            record,
            serde_json::from_str::<Value>(expected).expect("expected record should parse")
        );
    }

    #[rstest]
    fn scopes_tool_result_file_paths_to_the_file_payload(analysis: TranscriptAnalysis) {
        let mut record = json!({
            "type": "user",
            "toolUseResult": {
                "file": {
                    "filePath": "/fixture/source.png",
                    "base64": "fixture-file-payload",
                },
                "answers": {
                    "image": "data:image/png;base64,fixtureanswerimage",
                },
            },
        });

        let action = sanitize_images(&mut record, &analysis);

        assert_eq!(
            (action, record),
            (
                RecordAction::KeepChanged,
                json!({
                    "type": "user",
                    "toolUseResult": {
                        "file": {
                            "filePath": "/fixture/source.png",
                            "base64": "[Image data omitted; source: /fixture/source.png]",
                        },
                        "answers": {
                            "image": "[Image data omitted]",
                        },
                    },
                }),
            ),
        );
    }
}
