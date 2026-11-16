use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

use serde::Deserialize;

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

pub(super) fn read_total_tokens(path: &Path) -> u64 {
    let Ok(file) = File::open(path) else {
        return 0;
    };

    let mut last_total = 0;
    for line in BufReader::new(file).lines().map_while(Result::ok) {
        let Ok(entry) = serde_json::from_str::<TranscriptEntry>(&line) else {
            continue;
        };
        if entry.entry_type.as_deref() == Some("assistant")
            && let Some(usage) = entry.message.and_then(|message| message.usage)
        {
            last_total = usage.total();
        }
    }
    last_total
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

        assert_eq!(read_total_tokens(&transcript), 23);
    }
}
