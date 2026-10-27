use std::io::{BufRead, BufReader};
use std::path::Path;

use anyhow::{Context, Result};
use serde_json::Value;

pub(super) fn model_for_tool_use(
    transcript_path: &Path,
    tool_use_id: &str,
    tool_name: &str,
    command: &str,
) -> Result<String> {
    let file =
        std::fs::File::open(transcript_path).with_context(|| "Claude transcript is unavailable")?;
    let mut found: Option<String> = None;
    let mut last_model: Option<String> = None;
    for line in BufReader::new(file).lines() {
        let line = line.context("failed to read Claude transcript")?;
        let Ok(entry) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if entry.get("type").and_then(Value::as_str) != Some("assistant") {
            continue;
        }
        let Some(message) = entry.get("message") else {
            continue;
        };
        // Claude Code has not written the in-flight tool use to the transcript yet when
        // PreToolUse runs, so the preceding assistant entry is the only evidence of the model.
        // "<synthetic>" marks client-generated entries (e.g. interrupt notices), not a model.
        if let Some(model) = message
            .get("model")
            .and_then(Value::as_str)
            .filter(|model| !model.is_empty() && *model != "<synthetic>")
        {
            last_model = Some(model.to_owned());
        }
        let Some(content) = message.get("content").and_then(Value::as_array) else {
            continue;
        };
        for block in content {
            if block.get("type").and_then(Value::as_str) != Some("tool_use")
                || block.get("id").and_then(Value::as_str) != Some(tool_use_id)
            {
                continue;
            }
            if block.get("name").and_then(Value::as_str) != Some(tool_name)
                || block.pointer("/input/command").and_then(Value::as_str) != Some(command)
            {
                anyhow::bail!("Claude tool-use identifier does not match the hook command");
            }
            let model = message
                .get("model")
                .and_then(Value::as_str)
                .filter(|model| !model.is_empty())
                .ok_or_else(|| anyhow::anyhow!("matching Claude tool use has no runtime model"))?;
            if found.replace(model.to_owned()).is_some() {
                anyhow::bail!("Claude tool-use identifier is ambiguous in its transcript");
            }
        }
    }
    found
        .or(last_model)
        .ok_or_else(|| anyhow::anyhow!("Claude transcript has no assistant model"))
}
