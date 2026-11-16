use std::fs::File;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::Path;

use anyhow::{Context, Result};
use serde::Deserialize;

const INITIAL_READ_SIZE: usize = 8 * 1024;
const MAX_READ_SIZE: usize = 64 * 1024;
const MAX_LINES_TO_SCAN: usize = 20;

#[derive(Deserialize)]
struct TranscriptEntry {
    #[serde(rename = "type")]
    entry_type: Option<String>,
    message: Option<AssistantMessage>,
}

#[derive(Deserialize)]
struct AssistantMessage {
    usage: Option<AssistantUsage>,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct AssistantUsage {
    input_tokens: u64,
    output_tokens: u64,
    cache_creation_input_tokens: u64,
    cache_read_input_tokens: u64,
}

impl AssistantUsage {
    fn total(&self) -> u64 {
        self.input_tokens
            .saturating_add(self.output_tokens)
            .saturating_add(self.cache_creation_input_tokens)
            .saturating_add(self.cache_read_input_tokens)
    }
}

pub(super) fn read_total_tokens(path: &Path) -> Result<u64> {
    let file =
        File::open(path).with_context(|| format!("opening transcript {}", path.display()))?;
    if let Some(tokens) = read_last_usage_reverse(&file) {
        return Ok(tokens);
    }

    read_last_usage_forward(&file)
}

fn read_last_usage_reverse(file: &File) -> Option<u64> {
    let file_size = file.metadata().ok()?.len();
    if file_size < INITIAL_READ_SIZE as u64 {
        return None;
    }

    let mut read_size = INITIAL_READ_SIZE;
    while read_size <= MAX_READ_SIZE {
        let actual_read = std::cmp::min(read_size as u64, file_size) as usize;
        if let Some(tokens) = read_last_usage_window(file, actual_read, file_size) {
            return Some(tokens);
        }
        read_size *= 2;
    }
    None
}

fn read_last_usage_window(file: &File, read_size: usize, file_size: u64) -> Option<u64> {
    let mut reader = BufReader::new(file);
    reader.seek(SeekFrom::End(-(read_size as i64))).ok()?;
    let mut buffer = vec![0_u8; read_size];
    reader.read_exact(&mut buffer).ok()?;
    let content = String::from_utf8_lossy(&buffer);
    let complete_content = if read_size as u64 >= file_size {
        &content[..]
    } else {
        &content[content.find('\n')? + 1..]
    };

    complete_content
        .lines()
        .rev()
        .take(MAX_LINES_TO_SCAN)
        .find_map(usage_total_from_line)
}

fn read_last_usage_forward(file: &File) -> Result<u64> {
    let mut last_total = 0;
    for line in BufReader::new(file).lines() {
        if let Some(total) = usage_total_from_line(&line.context("reading transcript line")?) {
            last_total = total;
        }
    }
    Ok(last_total)
}

fn usage_total_from_line(line: &str) -> Option<u64> {
    let entry: TranscriptEntry = serde_json::from_str(line).ok()?;
    if entry.entry_type.as_deref() != Some("assistant") {
        return None;
    }
    entry.message?.usage.map(|usage| usage.total())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use indoc::indoc;
    use tempfile::tempdir;

    use super::read_total_tokens;

    #[test]
    fn reads_the_latest_assistant_usage_and_skips_invalid_lines() {
        let directory = tempdir().expect("temporary directory should be created");
        let transcript = directory.path().join("fixture-transcript.jsonl");
        fs::write(
            &transcript,
            indoc! {r#"
                {"type":"assistant","message":{"usage":{"input_tokens":8,"output_tokens":3,"cache_creation_input_tokens":5,"cache_read_input_tokens":13}}}
                not json
                {"type":"user","message":{"usage":{"input_tokens":200}}}
                {"type":"assistant","message":{"usage":{"input_tokens":21,"output_tokens":2}}}
            "#},
        )
        .expect("fixture transcript should be written");

        assert_eq!(read_total_tokens(&transcript).unwrap(), 23);
    }

    #[test]
    fn finds_latest_usage_before_a_long_transcript_tail() {
        let directory = tempdir().expect("temporary directory should be created");
        let transcript = directory.path().join("fixture-transcript.jsonl");
        let usage = indoc! {r#"
            {"type":"assistant","message":{"usage":{"input_tokens":34}}}
        "#};
        let tail = indoc! {r#"
            {"type":"user","message":{"content":"fixture prompt"}}
        "#}
        .repeat(24);
        fs::write(&transcript, format!("{usage}{tail}"))
            .expect("fixture transcript should be written");

        assert_eq!(read_total_tokens(&transcript).unwrap(), 34);
    }
}
