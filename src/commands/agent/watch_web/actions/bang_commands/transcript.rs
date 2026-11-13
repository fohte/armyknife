use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_json::Value;

pub(super) fn transcript_contains_commands(
    path: &Path,
    expected_message: &str,
    expected_commands: &[String],
) -> Result<bool> {
    let reader = open_transcript(path)?;
    for line in reader.lines() {
        let line = line?;
        let Ok(record) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if let Some(message) = assistant_text(&record)
            && message == expected_message
            && commands_are_candidates(&message, expected_commands)
        {
            return Ok(true);
        }
    }
    Ok(false)
}

fn commands_are_candidates(message: &str, commands: &[String]) -> bool {
    let mut remaining = HashMap::<String, usize>::new();
    for candidate in bang_commands(message) {
        *remaining.entry(candidate).or_default() += 1;
    }
    commands.iter().all(|command| {
        let Some(available) = remaining.get_mut(command) else {
            return false;
        };
        if *available == 0 {
            return false;
        }
        *available -= 1;
        true
    })
}

fn open_transcript(path: &Path) -> Result<Box<dyn BufRead>> {
    let is_compressed = path.extension().and_then(|extension| extension.to_str()) == Some("zst");
    if is_compressed {
        let file =
            File::open(path).with_context(|| format!("open transcript {}", path.display()))?;
        return Ok(Box::new(BufReader::new(zstd::stream::read::Decoder::new(
            file,
        )?)));
    }
    match File::open(path) {
        Ok(file) => Ok(Box::new(BufReader::new(file))),
        Err(_) => {
            let compressed_path = compressed_path(path);
            let compressed = File::open(&compressed_path)
                .with_context(|| format!("open transcript {}", compressed_path.display()))?;
            Ok(Box::new(BufReader::new(zstd::stream::read::Decoder::new(
                compressed,
            )?)))
        }
    }
}

fn compressed_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(".zst");
    PathBuf::from(name)
}

fn assistant_text(record: &Value) -> Option<String> {
    let content = if record.get("type").and_then(Value::as_str) == Some("assistant") {
        record.get("message")?.get("content")?
    } else if record.get("type").and_then(Value::as_str) == Some("response_item")
        && record.pointer("/payload/type")?.as_str() == Some("message")
        && record.pointer("/payload/role")?.as_str() == Some("assistant")
    {
        record.pointer("/payload/content")?
    } else {
        return None;
    };
    match content {
        Value::String(text) => Some(text.clone()),
        Value::Array(blocks) => Some(
            blocks
                .iter()
                .filter(|block| {
                    matches!(
                        block.get("type").and_then(Value::as_str),
                        Some("text" | "input_text" | "output_text")
                    )
                })
                .filter_map(|block| block.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n"),
        ),
        _ => None,
    }
}

fn bang_commands(message: &str) -> Vec<String> {
    let mut commands = Vec::new();
    let mut fence: Option<(char, usize, usize)> = None;
    let mut in_indented_code = false;

    for line in message.lines() {
        if let Some((fence_character, fence_length, indent)) = fence {
            if is_closing_fence(line, fence_character, fence_length) {
                fence = None;
                continue;
            }
            add_block_command(strip_fence_indent(line, indent), &mut commands);
            continue;
        }

        if let Some(marker) = fence_marker(line) {
            fence = Some(marker);
            in_indented_code = false;
            continue;
        }

        if let Some(code_line) = indented_code_line(line) {
            in_indented_code = true;
            add_block_command(code_line, &mut commands);
        } else if line.trim().is_empty() && in_indented_code {
            continue;
        } else {
            in_indented_code = false;
            commands.extend(inline_bang_commands(line));
        }
    }
    commands
}

fn fence_marker(line: &str) -> Option<(char, usize, usize)> {
    let indent = line.bytes().take_while(|byte| *byte == b' ').count();
    if indent > 3 {
        return None;
    }
    let marker = line.get(indent..)?;
    let character = marker.chars().next()?;
    if character != '`' && character != '~' {
        return None;
    }
    let length = marker
        .chars()
        .take_while(|value| *value == character)
        .count();
    (length >= 3).then_some((character, length, indent))
}

fn is_closing_fence(line: &str, character: char, minimum_length: usize) -> bool {
    let Some((marker_character, marker_length, indent)) = fence_marker(line) else {
        return false;
    };
    if marker_character != character || marker_length < minimum_length {
        return false;
    }
    line.get(indent + marker_length..)
        .is_some_and(|trailing| trailing.trim().is_empty())
}

fn strip_fence_indent(line: &str, indent: usize) -> &str {
    let spaces = line
        .bytes()
        .take(indent)
        .take_while(|byte| *byte == b' ')
        .count();
    &line[spaces..]
}

fn indented_code_line(line: &str) -> Option<&str> {
    if let Some(code_line) = line.strip_prefix('\t') {
        return Some(code_line);
    }
    (line.bytes().take_while(|byte| *byte == b' ').count() >= 4)
        .then(|| line.get(4..))
        .flatten()
}

fn add_block_command(line: &str, commands: &mut Vec<String>) {
    if let Some(command) = line.strip_prefix("! ") {
        let command = command.trim();
        if !command.is_empty() {
            commands.push(command.to_string());
        }
    }
}

fn inline_bang_commands(line: &str) -> Vec<String> {
    let mut commands = Vec::new();
    let mut index = 0;
    let bytes = line.as_bytes();
    while index < bytes.len() {
        if bytes[index] != b'`' {
            index += 1;
            continue;
        }
        let start = index;
        while index < bytes.len() && bytes[index] == b'`' {
            index += 1;
        }
        let delimiter_length = index - start;
        let mut close = index;
        let mut found = None;
        while close < bytes.len() {
            if bytes[close] != b'`' {
                close += 1;
                continue;
            }
            let close_start = close;
            while close < bytes.len() && bytes[close] == b'`' {
                close += 1;
            }
            if close - close_start == delimiter_length {
                found = Some(close_start);
                break;
            }
        }
        if let Some(end) = found {
            let code = line.get(index..end).unwrap_or_default();
            if let Some(command) = code.strip_prefix("! ") {
                let command = command.trim();
                if !command.is_empty() {
                    commands.push(command.to_string());
                }
            }
            index += delimiter_length;
        }
    }
    commands
}

#[cfg(test)]
mod tests {
    use indoc::indoc;
    use rstest::rstest;

    use super::{assistant_text, bang_commands, commands_are_candidates};

    #[rstest]
    #[case::fenced_commands(indoc! {"\
        ```sh
        ! printf first
        ! printf second
        ```"}, vec!["printf first", "printf second"])]
    #[case::indented_commands(indoc! {"
        prose
            ! printf first

            ! printf second
    "}, vec!["printf first", "printf second"])]
    #[case::indented_fence(indoc! {"
        prose
          ```sh
          ! printf result
          ```
    "}, vec!["printf result"])]
    #[case::inline_command("Run `! printf result` now.", vec!["printf result"])]
    #[case::other_code_is_ignored("Run `printf result` and use ! plain text.", vec![])]
    fn detects_only_commands_in_markdown_code(#[case] message: &str, #[case] expected: Vec<&str>) {
        assert_eq!(bang_commands(message), expected);
    }

    #[rstest]
    #[case::claude(serde_json::json!({"type":"assistant","message":{"content":[{"type":"text","text":"hello"}]}}), Some("hello".to_string()))]
    #[case::codex(serde_json::json!({"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"hello"}]}}), Some("hello".to_string()))]
    #[case::user(serde_json::json!({"type":"user","message":{"content":"hello"}}), None)]
    fn extracts_assistant_message(
        #[case] record: serde_json::Value,
        #[case] expected: Option<String>,
    ) {
        assert_eq!(assistant_text(&record), expected);
    }

    #[rstest]
    #[case::authored_command("Run `! echo safe`.", vec!["echo safe"], true)]
    #[case::indented_command("    ! echo safe", vec!["echo safe"], true)]
    #[case::unlisted_command("Run `! echo safe`.", vec!["echo unsafe"], false)]
    #[case::duplicate_request("Run `! echo safe`.", vec!["echo safe", "echo safe"], false)]
    fn validates_requested_command_occurrences(
        #[case] message: &str,
        #[case] commands: Vec<&str>,
        #[case] expected: bool,
    ) {
        let commands = commands.into_iter().map(str::to_string).collect::<Vec<_>>();
        assert_eq!(commands_are_candidates(message, &commands), expected);
    }
}
