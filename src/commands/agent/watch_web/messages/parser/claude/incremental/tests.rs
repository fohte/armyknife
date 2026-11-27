use std::io::{self, BufRead, BufReader, Cursor, Read, Seek, SeekFrom};

use chrono::{DateTime, TimeZone, Utc};
use indoc::indoc;
use rstest::rstest;

use super::super::super::super::ChatItem;
use super::super::ParserAdditions;
use super::{IncrementalAppend, IncrementalClaudeParser};
use crate::commands::agent::store::{AgentTodo, WatchWebQueuedPrompt};
use crate::commands::agent::types::Engine;
use crate::commands::agent::watch_web::messages::TranscriptInput;

use crate::commands::agent::watch_web::messages::cache::MessageCache;
use crate::commands::agent::watch_web::messages::incremental::{
    parse_initial_or_full, parse_session_transcript,
};
use crate::commands::agent::watch_web::messages::items::{EventKind, ToolOperation};
use crate::commands::agent::watch_web::messages::questions::{QuestionChoice, QuestionPrompt};

mod benchmark;

fn fixed_time() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).single().unwrap()
}

fn tail_window_parser(
    mut reader: impl BufRead + Seek,
    message_limit: usize,
    pending_task_ids: &[String],
) -> io::Result<Option<IncrementalClaudeParser>> {
    let file_length = reader.seek(SeekFrom::End(0))?;
    reader.seek(SeekFrom::Start(0))?;
    IncrementalClaudeParser::new_tail_window_with_length(
        reader,
        None,
        message_limit,
        pending_task_ids,
        file_length,
    )
}

fn empty_additions() -> ParserAdditions<'static> {
    ParserAdditions {
        session_id: "fixture-session",
        parent: None,
        pending_tool: None,
        user_marker: None,
        queued_prompts: &[],
        peer_out_messages: &[],
        proposals: &[],
        todos: &[],
        additional_cards: &[],
    }
}

#[rstest::fixture]
fn oversized_transcript() -> String {
    let padding = "x".repeat(5 * 1024 * 1024);
    indoc::formatdoc! {r#"
        {{"type":"user","uuid":"fixture-request","parentUuid":null,"timestamp":"2026-01-01T00:00:01Z","message":{{"content":"Fixture request."}}}}
        {{"type":"system","uuid":"fixture-padding","parentUuid":null,"padding":"{padding}"}}
    "#}
}

#[rstest::fixture]
fn saved_queued_prompt() -> WatchWebQueuedPrompt {
    WatchWebQueuedPrompt {
        id: 7,
        message_id: Some("fixture-message-cyan".to_owned()),
        content: "Delivered request.".to_owned(),
        action_event_kind: None,
        attachments: Vec::new(),
        resolved: false,
        sent_at: fixed_time(),
    }
}

const MAX_PREFIX_SEARCH_BYTES: u64 = 256 * 1024 * 1024;

struct PrefixBeyondSearchLimit {
    header: Vec<u8>,
    boundary_record: Vec<u8>,
    suffix: Vec<u8>,
    padding_record: Vec<u8>,
    prefix_length: u64,
}

impl PrefixBeyondSearchLimit {
    fn reader(&self) -> BufReader<VirtualTranscript> {
        BufReader::new(VirtualTranscript {
            header: self.header.clone(),
            boundary_record: self.boundary_record.clone(),
            suffix: self.suffix.clone(),
            padding_record: self.padding_record.clone(),
            prefix_length: self.prefix_length,
            position: 0,
        })
    }

    fn reference_transcript(&self) -> Vec<u8> {
        [
            self.header.as_slice(),
            self.boundary_record.as_slice(),
            self.suffix.as_slice(),
        ]
        .concat()
    }
}

#[rstest::fixture]
fn prefix_beyond_search_limit() -> PrefixBeyondSearchLimit {
    let marker = "⟦armyknife-user:fixture-marker-cyan⟧";
    let delivery = indoc::formatdoc! {"
        {marker}
        Delivered fixture request.
        <!-- armyknife-queued-prompt-id:fixture-message-cyan -->
    "};
    let header = [
        serde_json::json!({
            "type": "user",
            "uuid": "fixture-root",
            "parentUuid": null,
            "timestamp": "2026-01-01T00:00:00Z",
            "message": {"content": "Fixture root."},
        }),
        serde_json::json!({
            "type": "user",
            "uuid": "fixture-delivery",
            "parentUuid": "fixture-root",
            "timestamp": "2026-01-01T00:00:01Z",
            "origin": {"kind": "peer"},
            "message": {"content": delivery},
        }),
        serde_json::json!({
            "type": "attachment",
            "uuid": "fixture-skill-baseline",
            "parentUuid": "fixture-delivery",
            "timestamp": "2026-01-01T00:00:02Z",
            "attachment": {"type": "skill_listing", "isInitial": true, "content": []},
        }),
    ]
    .into_iter()
    .map(|record| record.to_string())
    .collect::<Vec<_>>()
    .join("\n")
        + "\n";
    let boundary_record = serde_json::json!({
        "type": "system",
        "uuid": "fixture-compact-boundary",
        "parentUuid": null,
        "logicalParentUuid": "fixture-skill-baseline",
        "timestamp": "2026-01-01T00:00:02Z",
        "subtype": "compact_boundary",
    })
    .to_string()
        + "\n";
    let mut suffix = Vec::new();
    let mut parent_uuid = "fixture-compact-boundary".to_owned();
    for index in 0..101 {
        let uuid = format!("fixture-message-{index}");
        let mut record = serde_json::json!({
            "type": "user",
            "uuid": uuid,
            "parentUuid": parent_uuid,
            "message": {"content": format!("Fixture request {index}.")},
        });
        if index != 100 {
            record["timestamp"] = serde_json::json!("2026-01-01T00:00:03Z");
        }
        suffix.extend_from_slice(record.to_string().as_bytes());
        if index != 100 {
            suffix.push(b'\n');
        }
        parent_uuid = format!("fixture-message-{index}");
    }
    let padding_payload = "x".repeat(1024 * 1024);
    let padding_record =
        format!("{{\"type\":\"fixture-padding\",\"padding\":\"{padding_payload}\"}}\n")
            .into_bytes();
    let header = header.into_bytes();
    let boundary_record = boundary_record.into_bytes();
    let minimum_prefix_length = MAX_PREFIX_SEARCH_BYTES + 4 * 1024 * 1024;
    let padding_records = minimum_prefix_length
        .saturating_sub((header.len() + boundary_record.len()) as u64)
        .div_ceil(padding_record.len() as u64);
    let prefix_length = header.len() as u64
        + padding_records * padding_record.len() as u64
        + boundary_record.len() as u64;

    PrefixBeyondSearchLimit {
        header,
        boundary_record,
        suffix,
        padding_record,
        prefix_length,
    }
}

struct VirtualTranscript {
    header: Vec<u8>,
    boundary_record: Vec<u8>,
    suffix: Vec<u8>,
    padding_record: Vec<u8>,
    prefix_length: u64,
    position: u64,
}

impl Read for VirtualTranscript {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        let total_length = self.prefix_length + self.suffix.len() as u64;
        let padding_end = self.prefix_length - self.boundary_record.len() as u64;
        let read_length = output
            .len()
            .min(total_length.saturating_sub(self.position) as usize);
        let mut written = 0;
        while written < read_length {
            let position = self.position + written as u64;
            if position < self.header.len() as u64 {
                let source_start = position as usize;
                let length = (read_length - written).min(self.header.len() - source_start);
                output[written..written + length]
                    .copy_from_slice(&self.header[source_start..source_start + length]);
                written += length;
            } else if position < padding_end {
                let padding_position = (position - self.header.len() as u64) as usize;
                let padding_offset = padding_position % self.padding_record.len();
                let length = (read_length - written)
                    .min(self.padding_record.len() - padding_offset)
                    .min((padding_end - position) as usize);
                output[written..written + length]
                    .copy_from_slice(&self.padding_record[padding_offset..padding_offset + length]);
                written += length;
            } else if position < self.prefix_length {
                let boundary_offset = (position - padding_end) as usize;
                let length =
                    (read_length - written).min(self.boundary_record.len() - boundary_offset);
                output[written..written + length].copy_from_slice(
                    &self.boundary_record[boundary_offset..boundary_offset + length],
                );
                written += length;
            } else {
                let source_start = (position - self.prefix_length) as usize;
                let length = (read_length - written).min(self.suffix.len() - source_start);
                output[written..written + length]
                    .copy_from_slice(&self.suffix[source_start..source_start + length]);
                written += length;
            }
        }
        self.position += written as u64;
        Ok(written)
    }
}

impl Seek for VirtualTranscript {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        let total_length = self.prefix_length + self.suffix.len() as u64;
        let position = match position {
            SeekFrom::Start(position) => i128::from(position),
            SeekFrom::Current(offset) => i128::from(self.position) + i128::from(offset),
            SeekFrom::End(offset) => i128::from(total_length) + i128::from(offset),
        };
        if !(0..=i128::from(u64::MAX)).contains(&position) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "fixture seek position is outside the transcript",
            ));
        }
        self.position = position as u64;
        Ok(self.position)
    }
}

fn queued_prompt_additions<'a>(queued_prompt: &'a WatchWebQueuedPrompt) -> ParserAdditions<'a> {
    ParserAdditions {
        user_marker: Some("⟦armyknife-user:fixture-marker-cobalt⟧"),
        queued_prompts: std::slice::from_ref(queued_prompt),
        ..empty_additions()
    }
}

const ROOT_AND_CURRENT: &str = indoc! {r#"
    {"type":"user","uuid":"fixture-root","parentUuid":null,"message":{"content":"Initial request."}}
    {"type":"assistant","uuid":"fixture-current","parentUuid":"fixture-root","message":{"content":[{"type":"text","text":"Current response."}]}}
"#};

fn incremental_parser(bytes: &[u8]) -> IncrementalClaudeParser {
    IncrementalClaudeParser::new(Cursor::new(bytes), None).unwrap()
}

fn parse_all(bytes: &[u8], additions: ParserAdditions<'_>) -> super::ParsedTranscript {
    IncrementalClaudeParser::new(Cursor::new(bytes), None)
        .unwrap()
        .render_with(fixed_time(), additions)
        .unwrap()
}

fn last_messages(mut parsed: super::ParsedTranscript, limit: usize) -> super::ParsedTranscript {
    parsed.0 = parsed
        .0
        .into_iter()
        .rev()
        .take(limit)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    parsed
}

fn last_message_and_state(mut parsed: super::ParsedTranscript) -> super::ParsedTranscript {
    parsed.0 = parsed
        .0
        .into_iter()
        .rev()
        .take(1)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    parsed
}

fn incremental_result(
    prefix: &[u8],
    suffix: &[u8],
) -> (IncrementalAppend, super::ParsedTranscript) {
    let mut parser = incremental_parser(prefix);
    let mut transcript = prefix.to_vec();
    transcript.extend_from_slice(suffix);
    let append = parser.append_bytes(suffix, fixed_time()).unwrap();
    let parser = match append {
        IncrementalAppend::Applied => parser,
        IncrementalAppend::RestartRequired => incremental_parser(&transcript),
    };
    (
        append,
        parser.render_with(fixed_time(), empty_additions()).unwrap(),
    )
}

fn incremental_three_chunk_result(
    first: &[u8],
    second: &[u8],
    third: &[u8],
) -> Option<super::ParsedTranscript> {
    let mut parser = incremental_parser(first);
    for chunk in [second, third] {
        if parser.append_bytes(chunk, fixed_time()).unwrap() == IncrementalAppend::RestartRequired {
            return None;
        }
    }
    Some(parser.render_with(fixed_time(), empty_additions()).unwrap())
}

fn incremental_applied_result(prefix: &[u8], suffix: &[u8]) -> Option<super::ParsedTranscript> {
    let mut parser = incremental_parser(prefix);
    (parser.append_bytes(suffix, fixed_time()).unwrap() == IncrementalAppend::Applied)
        .then(|| parser.render_with(fixed_time(), empty_additions()).unwrap())
}

fn assert_incremental_matches_full_parse_at_every_split_point(
    transcript: &str,
    expected: &super::ParsedTranscript,
    expected_append: Option<IncrementalAppend>,
    context: &str,
) {
    let transcript_bytes = transcript.as_bytes();
    for split in 0..=transcript_bytes.len() {
        if !transcript.is_char_boundary(split) {
            continue;
        }
        let actual = incremental_result(&transcript_bytes[..split], &transcript_bytes[split..]);
        if let Some(expected_append) = expected_append {
            assert_eq!(
                actual,
                (expected_append, expected.clone()),
                "{context}: transcript split at byte {split}",
            );
        } else {
            assert_eq!(
                actual.1,
                expected.clone(),
                "{context}: transcript split at byte {split}",
            );
        }
    }
}

struct TranscriptFixtureRng(u64);

impl TranscriptFixtureRng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn index(&mut self, length: usize) -> usize {
        (self.next() % length as u64) as usize
    }
}

#[derive(Clone)]
struct FixtureToolUse {
    id: String,
    owner_uuid: String,
}

fn generated_transcript(seed: u64, include_attachments: bool) -> Vec<serde_json::Value> {
    let mut rng = TranscriptFixtureRng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    for _ in 0..4 {
        rng.next();
    }
    let record_count = 3 + rng.index(8);
    let mut records = Vec::with_capacity(record_count);
    let mut known_uuids = Vec::<String>::new();
    let mut result_uuids = Vec::<String>::new();
    let mut pending_tool_uses = Vec::<FixtureToolUse>::new();
    let mut completed_tool_uses = Vec::<FixtureToolUse>::new();

    for index in 0..record_count {
        let uuid = format!("fixture-{seed}-record-{index}");
        let latest_parent = known_uuids.last().cloned();
        let arbitrary_parent =
            (!known_uuids.is_empty()).then(|| known_uuids[rng.index(known_uuids.len())].clone());
        let parent_uuid = if rng.index(10) < 6 {
            latest_parent.clone()
        } else {
            arbitrary_parent.clone()
        };
        let kind = rng.index(if include_attachments { 100 } else { 94 });

        if kind < 30 && (!pending_tool_uses.is_empty() || !completed_tool_uses.is_empty()) {
            let tool_use = if !pending_tool_uses.is_empty() && rng.index(8) != 0 {
                pending_tool_uses.remove(rng.index(pending_tool_uses.len()))
            } else if !completed_tool_uses.is_empty() {
                completed_tool_uses[rng.index(completed_tool_uses.len())].clone()
            } else {
                pending_tool_uses.remove(rng.index(pending_tool_uses.len()))
            };
            let parent_uuid = match rng.index(6) {
                0 | 1 => Some(tool_use.owner_uuid.clone()),
                2 if !result_uuids.is_empty() => {
                    Some(result_uuids[rng.index(result_uuids.len())].clone())
                }
                3 => latest_parent.clone(),
                4 => arbitrary_parent.clone(),
                _ => Some(tool_use.owner_uuid.clone()),
            };
            let mut content = vec![serde_json::json!({
                "type": "tool_result",
                "tool_use_id": tool_use.id.clone(),
                "content": "Fixture output.",
            })];
            if !pending_tool_uses.is_empty() && rng.index(5) == 0 {
                let second_tool_use = pending_tool_uses.remove(rng.index(pending_tool_uses.len()));
                content.push(serde_json::json!({
                    "type": "tool_result",
                    "tool_use_id": second_tool_use.id.clone(),
                    "content": "Second fixture output.",
                }));
                completed_tool_uses.push(second_tool_use);
            }
            let mut record = serde_json::json!({
                "type": "user",
                "uuid": uuid.clone(),
                "parentUuid": parent_uuid,
                "message": {"content": content},
            });
            if rng.index(4) == 0 {
                record["sourceToolAssistantUUID"] = serde_json::json!(tool_use.owner_uuid.clone());
            }
            records.push(record);
            completed_tool_uses.push(tool_use);
            result_uuids.push(uuid.clone());
            known_uuids.push(uuid);
        } else if kind < 52 {
            let tool_count = 1 + rng.index(3);
            let mut content = Vec::with_capacity(tool_count + 1);
            if rng.index(3) == 0 {
                content.push(serde_json::json!({
                    "type": "text",
                    "text": format!("Fixture context {index}."),
                }));
            }
            for tool_index in 0..tool_count {
                let id = format!("fixture-{seed}-tool-{index}-{tool_index}");
                content.push(serde_json::json!({
                    "type": "tool_use",
                    "id": id,
                    "name": "Read",
                    "input": {"file_path": format!("/fixture/generated-{index}-{tool_index}.md")},
                }));
                pending_tool_uses.push(FixtureToolUse {
                    id: format!("fixture-{seed}-tool-{index}-{tool_index}"),
                    owner_uuid: uuid.clone(),
                });
            }
            records.push(serde_json::json!({
                "type": "assistant",
                "uuid": uuid.clone(),
                "parentUuid": parent_uuid,
                "message": {"content": content},
            }));
            known_uuids.push(uuid);
        } else if kind < 67 {
            records.push(serde_json::json!({
                "type": "assistant",
                "uuid": uuid.clone(),
                "parentUuid": parent_uuid,
                "message": {"content": [{"type": "text", "text": format!("Fixture response {index}.")}]},
            }));
            known_uuids.push(uuid);
        } else if kind < 79 {
            records.push(serde_json::json!({
                "type": "user",
                "uuid": uuid.clone(),
                "parentUuid": parent_uuid,
                "message": {"content": format!("Fixture request {index}.")},
            }));
            known_uuids.push(uuid);
        } else if kind < 89 {
            records.push(serde_json::json!({
                "type": "user",
                "parentUuid": if rng.index(2) == 0 { latest_parent.clone() } else { arbitrary_parent.clone() },
                "origin": {"kind": "task-notification"},
                "message": {"content": format!("<task-notification><task-id>fixture-task-{}</task-id><status>completed</status></task-notification>", rng.index(2))},
            }));
        } else if kind < 94 {
            records.push(serde_json::json!({
                "type": "user",
                "uuid": uuid.clone(),
                "parentUuid": parent_uuid,
                "origin": {"kind": "task-notification"},
                "message": {"content": "<task-notification><task-id>fixture-task</task-id><status>completed</status></task-notification>"},
            }));
            known_uuids.push(uuid);
        } else {
            records.push(serde_json::json!({
                "type": "attachment",
                "uuid": uuid.clone(),
                "parentUuid": parent_uuid,
                "attachment": {"type": "hook_success", "hookName": "PostToolUse"},
            }));
            known_uuids.push(uuid);
        }
    }

    records
}

struct TranscriptFixtureSplitMix(u64);

impl TranscriptFixtureSplitMix {
    fn index(&mut self, length: usize) -> usize {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut value = self.0;
        value = (value ^ (value >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        value = (value ^ (value >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        ((value ^ (value >> 31)) % length as u64) as usize
    }
}

fn generated_transcript_with_tool_bursts(
    seed: u64,
    include_attachments: bool,
) -> Vec<serde_json::Value> {
    use serde_json::json;

    let mut rng = TranscriptFixtureSplitMix(seed ^ 0xDEAD_BEEF);
    let record_count = 3 + rng.index(8);
    let mut records = Vec::new();
    let mut uuids = Vec::<String>::new();
    let mut pending = Vec::<(String, String)>::new();
    let mut result_uuids = Vec::<String>::new();
    let mut next_id = 0;

    while records.len() < record_count {
        let latest = uuids.last().cloned();
        let pick_uuid = |rng: &mut TranscriptFixtureSplitMix, uuids: &[String]| {
            (!uuids.is_empty()).then(|| uuids[rng.index(uuids.len())].clone())
        };
        let kind = rng.index(if include_attachments { 100 } else { 87 });
        let uuid = format!("fixture-v2-record-{next_id}");
        next_id += 1;
        if kind < 25 {
            let burst = 1 + rng.index(3);
            let mut parent_uuid = if rng.index(4) == 0 {
                pick_uuid(&mut rng, &uuids)
            } else {
                latest.clone()
            };
            for tool_index in 0..burst {
                let child_uuid = format!("{uuid}-tool-{tool_index}");
                let tool_id = format!("fixture-v2-tool-{next_id}-{tool_index}");
                records.push(json!({
                    "type": "assistant",
                    "uuid": child_uuid,
                    "parentUuid": parent_uuid,
                    "message": {"content": [{
                        "type": "tool_use",
                        "id": tool_id,
                        "name": "Read",
                        "input": {"file_path": format!("/fixture/v2-{next_id}-{tool_index}")},
                    }]},
                }));
                pending.push((tool_id, child_uuid.clone()));
                uuids.push(child_uuid.clone());
                parent_uuid = Some(child_uuid);
            }
        } else if kind < 50 && !pending.is_empty() {
            let selected = if rng.index(2) == 0 {
                0
            } else {
                rng.index(pending.len())
            };
            let (tool_id, owner_uuid) = pending.remove(selected);
            let parent_uuid = match rng.index(5) {
                0 | 1 => Some(owner_uuid.clone()),
                2 if !result_uuids.is_empty() => {
                    Some(result_uuids[rng.index(result_uuids.len())].clone())
                }
                3 => latest.clone(),
                _ => pick_uuid(&mut rng, &uuids),
            };
            records.push(json!({
                "type": "user",
                "uuid": uuid.clone(),
                "parentUuid": parent_uuid,
                "sourceToolAssistantUUID": owner_uuid,
                "message": {"content": [{
                    "type": "tool_result",
                    "tool_use_id": tool_id,
                    "content": "Fixture tool output.",
                }]},
            }));
            result_uuids.push(uuid.clone());
            uuids.push(uuid);
        } else if kind < 62 {
            let parent_uuid = if rng.index(3) == 0 {
                pick_uuid(&mut rng, &uuids)
            } else {
                latest.clone()
            };
            records.push(json!({
                "type": "assistant",
                "uuid": uuid.clone(),
                "parentUuid": parent_uuid,
                "message": {"content": [{"type": "text", "text": format!("Fixture response {next_id}.")}]},
            }));
            uuids.push(uuid);
        } else if kind < 72 {
            let parent_uuid = if rng.index(2) == 0 {
                pick_uuid(&mut rng, &uuids)
            } else {
                latest.clone()
            };
            records.push(json!({
                "type": "user",
                "uuid": uuid.clone(),
                "parentUuid": parent_uuid,
                "message": {"content": format!("Fixture request {next_id}.")},
            }));
            uuids.push(uuid);
        } else if kind < 82 {
            records.push(json!({
                "type": "user",
                "parentUuid": latest,
                "origin": {"kind": "task-notification"},
                "message": {"content": "<task-notification><task-id>fixture-task</task-id><status>completed</status></task-notification>"},
            }));
        } else if include_attachments && kind < 90 {
            let parent_uuid = if rng.index(4) == 0 {
                pick_uuid(&mut rng, &uuids)
            } else {
                latest.clone()
            };
            records.push(json!({
                "type": "attachment",
                "uuid": uuid.clone(),
                "parentUuid": parent_uuid,
                "attachment": {"type": "hook_success", "hookName": "PostToolUse"},
            }));
            uuids.push(uuid);
        } else if include_attachments && kind < 95 {
            records.push(json!({
                "type": "attachment",
                "uuid": uuid.clone(),
                "parentUuid": latest,
                "attachment": {
                    "type": "queued_command",
                    "origin": {"kind": "task-notification"},
                    "prompt": "<task-notification><task-id>fixture-task</task-id><status>completed</status></task-notification>",
                },
            }));
            uuids.push(uuid);
        } else {
            records.push(json!({
                "type": "system",
                "uuid": uuid.clone(),
                "parentUuid": latest,
                "subtype": "compact_boundary",
            }));
            uuids.push(uuid);
        }
    }

    records.truncate(record_count.max(3));
    records
}

fn serialize_fixture_transcript(records: &[serde_json::Value]) -> String {
    let mut transcript = records
        .iter()
        .map(serde_json::Value::to_string)
        .collect::<Vec<_>>()
        .join("\n");
    if !records.is_empty() {
        transcript.push('\n');
    }
    transcript
}

fn fixture_record_boundaries(records: &[serde_json::Value]) -> Vec<usize> {
    let mut boundaries = vec![0];
    for record in records {
        let next = boundaries.last().copied().unwrap_or_default() + record.to_string().len() + 1;
        boundaries.push(next);
    }
    boundaries
}

fn assert_append_matches_full_parse_at_record_boundary(
    prefix: &str,
    suffix: &str,
    expected_append: Option<IncrementalAppend>,
    context: &str,
) {
    let transcript = format!("{prefix}{suffix}");
    let expected = parse_all(transcript.as_bytes(), empty_additions());
    let actual = incremental_result(prefix.as_bytes(), suffix.as_bytes());
    if let Some(expected_append) = expected_append {
        assert_eq!(
            actual,
            (expected_append, expected.clone()),
            "{context}: append at the record boundary",
        );
    } else {
        assert_eq!(
            actual.1,
            expected.clone(),
            "{context}: append at the record boundary",
        );
    }
    assert_incremental_matches_full_parse_at_every_split_point(
        &transcript,
        &expected,
        None,
        context,
    );
}

const PARALLEL_TOOLS_WITH_SYSTEM_AFTER_RESULT: &str = indoc! {r#"
    {"type":"user","uuid":"root","parentUuid":null,"timestamp":"2026-01-01T00:00:01Z","message":{"content":"Run two fixture tools."}}
    {"type":"assistant","uuid":"tool-call","parentUuid":"root","timestamp":"2026-01-01T00:00:02Z","message":{"content":[{"type":"tool_use","id":"call-a","name":"Read","input":{"file_path":"/fixture/a"}},{"type":"tool_use","id":"call-b","name":"Read","input":{"file_path":"/fixture/b"}}]}}
    {"type":"user","uuid":"result-a","parentUuid":"tool-call","timestamp":"2026-01-01T00:00:03Z","message":{"content":[{"type":"tool_result","tool_use_id":"call-a","content":"first result"}]}}
    {"type":"system","uuid":"system-step","parentUuid":"result-a","timestamp":"2026-01-01T00:00:04Z","subtype":"compact_boundary"}
"#};

const PARALLEL_TOOL_RESULT_BRANCH_PREFIX: &str = indoc! {r#"
    {"type":"user","uuid":"fixture-root","parentUuid":null,"timestamp":"2026-01-01T00:00:01Z","message":{"content":"Run two fixture tools."}}
    {"type":"assistant","uuid":"fixture-first-call","parentUuid":"fixture-root","timestamp":"2026-01-01T00:00:02Z","message":{"content":[{"type":"tool_use","id":"fixture-call-a","name":"Read","input":{"file_path":"/fixture/a"}}]}}
    {"type":"assistant","uuid":"fixture-second-call","parentUuid":"fixture-first-call","timestamp":"2026-01-01T00:00:03Z","message":{"content":[{"type":"tool_use","id":"fixture-call-b","name":"Read","input":{"file_path":"/fixture/b"}}]}}
    {"type":"user","uuid":"fixture-result-a","parentUuid":"fixture-first-call","timestamp":"2026-01-01T00:00:04Z","message":{"content":[{"type":"tool_result","tool_use_id":"fixture-call-a","content":"first result"}]}}
    {"type":"assistant","uuid":"fixture-followup","parentUuid":"fixture-result-a","timestamp":"2026-01-01T00:00:05Z","message":{"content":[{"type":"text","text":"The first tool result is ready."}]}}
"#};

const PARALLEL_TOOL_USE_RESULT_BRANCH_PREFIX: &str = indoc! {r#"
    {"type":"user","uuid":"fixture-root","parentUuid":null,"timestamp":"2026-01-01T00:00:01Z","message":{"content":"Run two fixture tools."}}
    {"type":"assistant","uuid":"fixture-first-call","parentUuid":"fixture-root","timestamp":"2026-01-01T00:00:02Z","message":{"content":[{"type":"tool_use","id":"fixture-call-a","name":"Read","input":{"file_path":"/fixture/a"}}]}}
    {"type":"assistant","uuid":"fixture-second-call","parentUuid":"fixture-first-call","timestamp":"2026-01-01T00:00:03Z","message":{"content":[{"type":"tool_use","id":"fixture-call-b","name":"Read","input":{"file_path":"/fixture/b"}}]}}
    {"type":"user","uuid":"fixture-result-a","parentUuid":"fixture-first-call","timestamp":"2026-01-01T00:00:04Z","message":{"content":[{"type":"tool_result","tool_use_id":"fixture-call-a","content":"first result"}]}}
    {"type":"assistant","uuid":"fixture-next-call","parentUuid":"fixture-result-a","timestamp":"2026-01-01T00:00:05Z","message":{"content":[{"type":"tool_use","id":"fixture-call-c","name":"Read","input":{"file_path":"/fixture/c"}}]}}
"#};

const PARALLEL_TOOL_USE_RESULT_BRANCH_SUFFIX: &str = indoc! {r#"
    {"type":"user","uuid":"fixture-result-b","parentUuid":"fixture-second-call","timestamp":"2026-01-01T00:00:06Z","message":{"content":[{"type":"tool_result","tool_use_id":"fixture-call-b","content":"second result"}]}}
"#};

const PARALLEL_TOOL_RESULT_BRANCH_SUFFIX: &str = indoc! {r#"
    {"type":"user","uuid":"fixture-result-b","parentUuid":"fixture-second-call","timestamp":"2026-01-01T00:00:06Z","message":{"content":[{"type":"tool_result","tool_use_id":"fixture-call-b","content":"second result"}]}}
"#};

const PARALLEL_TOOL_RESULT_WITH_FOLLOWUP_SUFFIX: &str = indoc! {r#"
    {"type":"user","uuid":"fixture-result-b","parentUuid":"fixture-second-call","timestamp":"2026-01-01T00:00:06Z","message":{"content":[{"type":"tool_result","tool_use_id":"fixture-call-b","content":"second result"}]}}
    {"type":"assistant","uuid":"fixture-latest-followup","parentUuid":"fixture-followup","timestamp":"2026-01-01T00:00:07Z","message":{"content":[{"type":"text","text":"The active branch continues."}]}}
"#};

const COMPACT_BRANCH_PENDING_TOOL_PREFIX: &str = indoc! {r#"
    {"type":"user","uuid":"fixture-root","parentUuid":null,"timestamp":"2026-01-01T00:00:01Z","message":{"content":"Run a fixture tool."}}
    {"type":"system","uuid":"fixture-compact-boundary","parentUuid":"fixture-root","timestamp":"2026-01-01T00:00:02Z","subtype":"compact_boundary"}
    {"type":"assistant","uuid":"fixture-post-compact-call","parentUuid":"fixture-compact-boundary","timestamp":"2026-01-01T00:00:03Z","message":{"content":[{"type":"tool_use","id":"fixture-post-compact-call-id","name":"Read","input":{"file_path":"/fixture/post-compact.md"}}]}}
"#};

const COMPACT_BRANCH_PENDING_TOOL_SUFFIX: &str = indoc! {r#"
    {"type":"user","uuid":"fixture-post-compact-result","parentUuid":"fixture-post-compact-call","timestamp":"2026-01-01T00:00:04Z","message":{"content":[{"type":"tool_result","tool_use_id":"fixture-post-compact-call-id","content":"fixture output"}]}}
"#};

const INITIAL_PENDING_SIBLING_TRANSCRIPT: &str = indoc! {r#"
    {"type":"user","uuid":"root","parentUuid":null,"message":{"content":"The original request."}}
    {"type":"assistant","uuid":"current","parentUuid":"root","message":{"content":[{"type":"text","text":"The current branch response."}]}}
    {"type":"assistant","uuid":"replacement","parentUuid":"root","message":{"content":[{"type":"text","text":"The replacement response."}]}}
"#};

#[rstest]
#[case::tool_use_then_appended_tool_result(
    indoc! {r#"
        {"type":"assistant","uuid":"tool-call","parentUuid":null,"timestamp":"2026-01-01T00:00:01Z","message":{"content":[{"type":"tool_use","id":"call-a","name":"Bash","input":{"command":"echo fixture"}}]}}
    "#},
    indoc! {r#"
        {"type":"user","uuid":"tool-result","parentUuid":"tool-call","timestamp":"2026-01-01T00:00:02Z","message":{"content":[{"type":"tool_result","tool_use_id":"call-a","content":"fixture output"}]}}
    "#},
    IncrementalAppend::Applied,
)]
#[case::duplicate_uuid_requires_reparse(
    ROOT_AND_CURRENT,
    indoc! {r#"
        {"type":"assistant","uuid":"fixture-current","parentUuid":"fixture-current","message":{"content":[{"type":"text","text":"A duplicate fixture record."}]}}
    "#},
    IncrementalAppend::RestartRequired,
)]
#[case::uuidless_tool_result_resolves_a_pending_tool_use(
    indoc! {r#"
        {"type":"assistant","message":{"content":[{"type":"tool_use","id":"call-a","name":"Read","input":{"file_path":"/fixture/a"}}]}}
    "#},
    indoc! {r#"
        {"type":"user","sourceToolAssistantUUID":"fixture-assistant","toolUseResult":{},"message":{"content":[{"type":"tool_result","tool_use_id":"call-a","content":"fixture output"}]}}
    "#},
    IncrementalAppend::RestartRequired,
)]
#[case::tool_result_after_an_appended_hook_attachment(
    indoc! {r#"
        {"type":"assistant","uuid":"tool-call","parentUuid":null,"timestamp":"2026-01-01T00:00:01Z","message":{"content":[{"type":"tool_use","id":"call-a","name":"Bash","input":{"command":"echo fixture"}}]}}
    "#},
    indoc! {r#"
        {"type":"attachment","uuid":"hook-attachment","parentUuid":"tool-call","timestamp":"2026-01-01T00:00:02Z","attachment":{"type":"hook_success"}}
        {"type":"user","uuid":"tool-result","parentUuid":"tool-call","timestamp":"2026-01-01T00:00:03Z","message":{"content":[{"type":"tool_result","tool_use_id":"call-a","content":"fixture output"}]}}
    "#},
    IncrementalAppend::RestartRequired,
)]
#[case::tool_result_appended_after_a_hook_attachment_in_the_initial_transcript(
    indoc! {r#"
        {"type":"assistant","uuid":"tool-call","parentUuid":null,"timestamp":"2026-01-01T00:00:01Z","message":{"content":[{"type":"tool_use","id":"call-a","name":"Bash","input":{"command":"echo fixture"}}]}}
        {"type":"attachment","uuid":"hook-attachment","parentUuid":"tool-call","timestamp":"2026-01-01T00:00:02Z","attachment":{"type":"hook_success"}}
    "#},
    indoc! {r#"
        {"type":"user","uuid":"tool-result","parentUuid":"tool-call","timestamp":"2026-01-01T00:00:03Z","message":{"content":[{"type":"tool_result","tool_use_id":"call-a","content":"fixture output"}]}}
    "#},
    IncrementalAppend::RestartRequired,
)]
#[case::parallel_tool_result_continues_after_an_attachment_preserves_its_parent(
    indoc! {r#"
        {"type":"assistant","uuid":"parallel-calls","parentUuid":null,"timestamp":"2026-01-01T00:00:01Z","message":{"content":[{"type":"tool_use","id":"call-a","name":"Read","input":{"file_path":"/fixture/a"}},{"type":"tool_use","id":"call-b","name":"Read","input":{"file_path":"/fixture/b"}}]}}
        {"type":"user","uuid":"result-a","parentUuid":"parallel-calls","timestamp":"2026-01-01T00:00:02Z","message":{"content":[{"type":"tool_result","tool_use_id":"call-a","content":"first result"}]}}
    "#},
    indoc! {r#"
        {"type":"attachment","uuid":"hook-attachment","parentUuid":"result-a","timestamp":"2026-01-01T00:00:03Z","attachment":{"type":"hook_success"}}
        {"type":"user","uuid":"result-b","parentUuid":"parallel-calls","timestamp":"2026-01-01T00:00:04Z","message":{"content":[{"type":"tool_result","tool_use_id":"call-b","content":"second result"}]}}
    "#},
    IncrementalAppend::RestartRequired,
)]
#[case::partial_json_at_delta_end(
    indoc! {r#"
        {"type":"user","uuid":"message-one","parentUuid":null,"timestamp":"2026-01-01T00:00:01Z","message":{"content":"A question."}}
    "#},
    r#"{"type":"assistant","uuid":"message-two","parentUuid":"message-one","timestamp":"2026-01-01T00:00:02Z","message":{"content":[{"type":"text","text":"unfinished"#,
    IncrementalAppend::Applied,
)]
#[case::initial_final_line_has_no_newline(
    r#"{"type":"user","uuid":"message-one","parentUuid":null,"timestamp":"2026-01-01T00:00:01Z","message":{"content":"A question."}}"#,
    "",
    IncrementalAppend::Applied,
)]
#[case::initial_pending_sibling_completes_on_append(
    INITIAL_PENDING_SIBLING_TRANSCRIPT.trim_end_matches('\n'),
    "\n",
    IncrementalAppend::RestartRequired,
)]
#[case::queue_operation_crosses_append_boundary(
    indoc! {r#"
        {"type":"user","uuid":"message-one","parentUuid":null,"timestamp":"2026-01-01T00:00:01Z","message":{"content":"A question."}}
        {"type":"queue-operation","operation":"enqueue","content":"Queued
    "#}.trim_end_matches('\n'),
    " request.\"}\n",
    IncrementalAppend::RestartRequired,
)]
#[case::ask_user_question_answer_arrives_later(
    indoc! {r#"
        {"type":"assistant","uuid":"question","parentUuid":null,"timestamp":"2026-01-01T00:00:01Z","message":{"content":[{"type":"tool_use","id":"ask-1","name":"AskUserQuestion","input":{"questions":[{"question":"Choose a fixture option","options":[{"label":"First option","description":"The first fixture choice."},{"label":"Second option","description":"The second fixture choice."}]}]}}]}}
    "#},
    indoc! {r#"
        {"type":"user","uuid":"answer","parentUuid":"question","timestamp":"2026-01-01T00:00:02Z","message":{"content":[{"type":"tool_result","tool_use_id":"ask-1","content":"Your questions have been answered: \"Choose a fixture option\"=\"Second option\""}]}}
    "#},
    IncrementalAppend::Applied,
)]
#[case::background_task_notification_completes_later(
    indoc! {r#"
        {"type":"assistant","uuid":"task-call","parentUuid":null,"timestamp":"2026-01-01T00:00:01Z","message":{"content":[{"type":"tool_use","id":"task-call-id","name":"Bash","input":{"command":"run fixture task","run_in_background":true}}]}}
    "#},
    indoc! {r#"
        {"type":"user","uuid":"task-started","parentUuid":"task-call","timestamp":"2026-01-01T00:00:02Z","toolUseResult":{"task_id":"fixture-task"},"message":{"content":[{"type":"tool_result","tool_use_id":"task-call-id","content":"Background task started (id: fixture-task)"}]}}
        {"type":"user","uuid":"task-finished","parentUuid":"task-started","timestamp":"2026-01-01T00:00:03Z","origin":{"kind":"task-notification"},"message":{"content":"<task-notification><task-id>fixture-task</task-id><status>completed</status><summary>Fixture task completed</summary></task-notification>"}}
    "#},
    IncrementalAppend::RestartRequired,
)]
#[case::compaction_boundary_is_appended(
    indoc! {r#"
        {"type":"user","uuid":"message-one","parentUuid":null,"timestamp":"2026-01-01T00:00:01Z","message":{"content":"A question."}}
    "#},
    indoc! {r#"
        {"type":"system","uuid":"compact","parentUuid":null,"logicalParentUuid":"message-one","timestamp":"2026-01-01T00:00:02Z","subtype":"compact_boundary"}
    "#},
    IncrementalAppend::RestartRequired,
)]
#[case::assistant_continues_after_an_appended_attachment(
    indoc! {r#"
        {"type":"user","uuid":"fixture-request","parentUuid":null,"timestamp":"2026-01-01T00:00:01Z","message":{"content":"A fixture request."}}
        {"type":"assistant","uuid":"fixture-response","parentUuid":"fixture-request","timestamp":"2026-01-01T00:00:02Z","message":{"content":[{"type":"text","text":"A fixture response."}]}}
    "#},
    indoc! {r#"
        {"type":"attachment","uuid":"fixture-reminder","parentUuid":"fixture-response","timestamp":"2026-01-01T00:00:03Z","attachment":{"type":"silent_turn_reminder"}}
        {"type":"assistant","uuid":"fixture-followup","parentUuid":"fixture-reminder","timestamp":"2026-01-01T00:00:04Z","message":{"content":[{"type":"text","text":"A fixture follow-up."}]}}
    "#},
    IncrementalAppend::RestartRequired,
)]
#[case::assistant_continues_after_an_initial_attachment(
    indoc! {r#"
        {"type":"user","uuid":"fixture-request","parentUuid":null,"timestamp":"2026-01-01T00:00:01Z","message":{"content":"A fixture request."}}
        {"type":"assistant","uuid":"fixture-response","parentUuid":"fixture-request","timestamp":"2026-01-01T00:00:02Z","message":{"content":[{"type":"text","text":"A fixture response."}]}}
        {"type":"attachment","uuid":"fixture-reminder","parentUuid":"fixture-response","timestamp":"2026-01-01T00:00:03Z","attachment":{"type":"silent_turn_reminder"}}
    "#},
    indoc! {r#"
        {"type":"assistant","uuid":"fixture-followup","parentUuid":"fixture-reminder","timestamp":"2026-01-01T00:00:04Z","message":{"content":[{"type":"text","text":"A fixture follow-up."}]}}
    "#},
    IncrementalAppend::RestartRequired,
)]
#[case::parallel_tool_results_share_the_same_parent(
    indoc! {r#"
        {"type":"assistant","uuid":"parallel-calls","parentUuid":null,"timestamp":"2026-01-01T00:00:01Z","message":{"content":[{"type":"tool_use","id":"call-a","name":"Read","input":{"file_path":"/fixture/a"}},{"type":"tool_use","id":"call-b","name":"Read","input":{"file_path":"/fixture/b"}}]}}
    "#},
    indoc! {r#"
        {"type":"user","uuid":"result-a","parentUuid":"parallel-calls","timestamp":"2026-01-01T00:00:02Z","message":{"content":[{"type":"tool_result","tool_use_id":"call-a","content":"first result"}]}}
        {"type":"user","uuid":"result-b","parentUuid":"parallel-calls","timestamp":"2026-01-01T00:00:03Z","message":{"content":[{"type":"tool_result","tool_use_id":"call-b","content":"second result"}]}}
        {"type":"assistant","uuid":"parallel-answer","parentUuid":"parallel-calls","timestamp":"2026-01-01T00:00:04Z","message":{"content":[{"type":"text","text":"Parallel work is complete."}]}}
    "#},
    IncrementalAppend::RestartRequired,
)]
#[case::user_message_branching_from_an_active_ancestor_restarts(
    indoc! {r#"
        {"type":"user","uuid":"root","parentUuid":null,"timestamp":"2026-01-01T00:00:01Z","message":{"content":"The original request."}}
        {"type":"assistant","uuid":"current","parentUuid":"root","timestamp":"2026-01-01T00:00:02Z","message":{"content":[{"type":"text","text":"The current branch response."}]}}
    "#},
    indoc! {r#"
        {"type":"user","uuid":"replacement","parentUuid":"root","timestamp":"2026-01-01T00:00:03Z","message":{"content":"The replacement request."}}
    "#},
    IncrementalAppend::RestartRequired,
)]
#[case::assistant_message_branching_without_tool_results_restarts(
    indoc! {r#"
        {"type":"user","uuid":"root","parentUuid":null,"timestamp":"2026-01-01T00:00:01Z","message":{"content":"The original request."}}
        {"type":"assistant","uuid":"current","parentUuid":"root","timestamp":"2026-01-01T00:00:02Z","message":{"content":[{"type":"text","text":"The current branch response."}]}}
    "#},
    indoc! {r#"
        {"type":"assistant","uuid":"replacement","parentUuid":"root","timestamp":"2026-01-01T00:00:03Z","message":{"content":[{"type":"text","text":"The replacement response."}]}}
    "#},
    IncrementalAppend::RestartRequired,
)]
#[case::unterminated_assistant_fork_restarts_without_render_error(
    indoc! {r#"
        {"type":"user","uuid":"root","parentUuid":null,"timestamp":"2026-01-01T00:00:01Z","message":{"content":"The original request."}}
        {"type":"assistant","uuid":"current","parentUuid":"root","timestamp":"2026-01-01T00:00:02Z","message":{"content":[{"type":"text","text":"The current branch response."}]}}
    "#},
    r#"{"type":"assistant","uuid":"replacement","parentUuid":"root","timestamp":"2026-01-01T00:00:03Z","message":{"content":[{"type":"text","text":"The replacement response."}]}}"#,
    IncrementalAppend::RestartRequired,
)]
#[case::parallel_tool_result_after_a_new_assistant_continues_incrementally(
    PARALLEL_TOOL_RESULT_BRANCH_PREFIX,
    PARALLEL_TOOL_RESULT_BRANCH_SUFFIX,
    IncrementalAppend::Applied
)]
#[case::parallel_tool_result_with_meta_marker_restarts(
    PARALLEL_TOOL_RESULT_BRANCH_PREFIX,
    indoc! {r#"
        {"type":"user","uuid":"result-b","parentUuid":"fixture-second-call","isMeta":true,"message":{"content":[{"type":"tool_result","tool_use_id":"fixture-call-b","content":"second result"}]}}
    "#},
    IncrementalAppend::RestartRequired
)]
#[case::parallel_tool_result_after_a_meta_marked_first_result_restarts(
    indoc! {r#"
        {"type":"user","uuid":"fixture-root","parentUuid":null,"message":{"content":"Run two fixture tools."}}
        {"type":"assistant","uuid":"fixture-first-call","parentUuid":"fixture-root","message":{"content":[{"type":"tool_use","id":"fixture-call-a","name":"Read","input":{"file_path":"/fixture/a"}}]}}
        {"type":"assistant","uuid":"fixture-second-call","parentUuid":"fixture-first-call","message":{"content":[{"type":"tool_use","id":"fixture-call-b","name":"Read","input":{"file_path":"/fixture/b"}}]}}
        {"type":"user","uuid":"fixture-result-a","parentUuid":"fixture-first-call","isMeta":true,"message":{"content":[{"type":"tool_result","tool_use_id":"fixture-call-a","content":"first result"}]}}
        {"type":"assistant","uuid":"fixture-followup","parentUuid":"fixture-result-a","message":{"content":[{"type":"text","text":"The first tool result is ready."}]}}
    "#},
    PARALLEL_TOOL_RESULT_BRANCH_SUFFIX,
    IncrementalAppend::RestartRequired
)]
#[case::assistant_after_system_invalidates_tool_result_parent(
    PARALLEL_TOOLS_WITH_SYSTEM_AFTER_RESULT,
    indoc! {r#"
        {"type":"assistant","uuid":"replacement","parentUuid":"tool-call","timestamp":"2026-01-01T00:00:05Z","message":{"content":[{"type":"text","text":"The replacement branch is active."}]}}
    "#},
    IncrementalAppend::RestartRequired,
)]
#[case::parallel_tool_result_after_a_compact_boundary_restarts_for_branch_parity(
    PARALLEL_TOOLS_WITH_SYSTEM_AFTER_RESULT,
    indoc! {r#"
        {"type":"user","uuid":"result-b","parentUuid":"tool-call","timestamp":"2026-01-01T00:00:05Z","message":{"content":[{"type":"tool_result","tool_use_id":"call-b","content":"second result"}]}}
    "#},
    IncrementalAppend::RestartRequired,
)]
#[case::parallel_tool_result_after_a_compact_boundary_descendant_restarts_for_branch_parity(
    PARALLEL_TOOLS_WITH_SYSTEM_AFTER_RESULT,
    indoc! {r#"
        {"type":"assistant","uuid":"post-boundary-assistant","parentUuid":"system-step","timestamp":"2026-01-01T00:00:05Z","message":{"content":[{"type":"text","text":"The compact-boundary branch continues."}]}}
        {"type":"user","uuid":"result-b","parentUuid":"tool-call","timestamp":"2026-01-01T00:00:06Z","message":{"content":[{"type":"tool_result","tool_use_id":"call-b","content":"second result"}]}}
    "#},
    IncrementalAppend::RestartRequired,
)]
#[case::task_notification_with_non_latest_parent_restarts(
    indoc! {r#"
        {"type":"assistant","uuid":"task-call","parentUuid":null,"timestamp":"2026-01-01T00:00:01Z","message":{"content":[{"type":"tool_use","id":"task-call-id","name":"Bash","input":{"command":"run fixture task","run_in_background":true}}]}}
        {"type":"user","uuid":"task-started","parentUuid":"task-call","timestamp":"2026-01-01T00:00:02Z","toolUseResult":{"task_id":"fixture-task"},"message":{"content":[{"type":"tool_result","tool_use_id":"task-call-id","content":"Background task started (id: fixture-task)"}]}}
        {"type":"assistant","uuid":"followup","parentUuid":"task-started","timestamp":"2026-01-01T00:00:03Z","message":{"content":[{"type":"text","text":"A later assistant turn is now the tip."}]}}
    "#},
    indoc! {r#"
        {"type":"user","uuid":"task-finished","parentUuid":"task-started","timestamp":"2026-01-01T00:00:04Z","origin":{"kind":"task-notification"},"message":{"content":"<task-notification><task-id>fixture-task</task-id><status>completed</status><summary>Fixture task completed</summary></task-notification>"}}
    "#},
    IncrementalAppend::RestartRequired,
)]
#[case::tool_result_with_missing_parent_requires_restart(
    indoc! {r#"
        {"type":"user","uuid":"root","parentUuid":null,"timestamp":"2026-01-01T00:00:01Z","message":{"content":"Run a fixture tool."}}
        {"type":"assistant","uuid":"tool-call","parentUuid":"root","timestamp":"2026-01-01T00:00:02Z","message":{"content":[{"type":"tool_use","id":"call-a","name":"Read","input":{"file_path":"/fixture/a"}}]}}
    "#},
    indoc! {r#"
        {"type":"user","uuid":"orphan-result","parentUuid":"missing-parent","timestamp":"2026-01-01T00:00:03Z","message":{"content":[{"type":"tool_result","tool_use_id":"call-a","content":"orphan output"}]}}
    "#},
    IncrementalAppend::RestartRequired,
)]
#[case::tool_result_with_unknown_tool_id_requires_restart(
    indoc! {r#"
        {"type":"user","uuid":"root","parentUuid":null,"timestamp":"2026-01-01T00:00:01Z","message":{"content":"Run a fixture tool."}}
        {"type":"assistant","uuid":"tool-call","parentUuid":"root","timestamp":"2026-01-01T00:00:02Z","message":{"content":[{"type":"tool_use","id":"call-a","name":"Read","input":{"file_path":"/fixture/a"}}]}}
    "#},
    indoc! {r#"
        {"type":"user","uuid":"unknown-result","parentUuid":"tool-call","timestamp":"2026-01-01T00:00:03Z","message":{"content":[{"type":"tool_result","tool_use_id":"unknown-call","content":"orphan output"}]}}
    "#},
    IncrementalAppend::RestartRequired,
)]
fn appended_transcript_matches_a_full_parse(
    #[case] prefix: &str,
    #[case] suffix: &str,
    #[case] expected_append: IncrementalAppend,
) {
    let mut transcript = prefix.as_bytes().to_vec();
    transcript.extend_from_slice(suffix.as_bytes());
    assert_eq!(
        incremental_result(prefix.as_bytes(), suffix.as_bytes()),
        (expected_append, parse_all(&transcript, empty_additions()),),
    );
}

#[test]
fn parallel_tool_result_matches_a_full_parse_at_every_split_point() {
    let transcript = format!(
        "{}{}",
        PARALLEL_TOOL_USE_RESULT_BRANCH_PREFIX, PARALLEL_TOOL_USE_RESULT_BRANCH_SUFFIX
    );
    let expected = parse_all(transcript.as_bytes(), empty_additions());

    assert_incremental_matches_full_parse_at_every_split_point(
        &transcript,
        &expected,
        Some(IncrementalAppend::Applied),
        "parallel tool result",
    );
}

#[test]
fn parallel_tool_result_after_an_assistant_followup_is_applied_at_every_split_point() {
    let transcript = format!(
        "{}{}",
        PARALLEL_TOOL_RESULT_BRANCH_PREFIX, PARALLEL_TOOL_RESULT_BRANCH_SUFFIX
    );
    let expected = parse_all(transcript.as_bytes(), empty_additions());

    assert_incremental_matches_full_parse_at_every_split_point(
        &transcript,
        &expected,
        Some(IncrementalAppend::Applied),
        "parallel result after an assistant text follow-up",
    );
}

#[test]
fn human_origin_user_and_followup_append_without_restart() {
    let transcript = format!(
        "{}{}",
        ROOT_AND_CURRENT,
        indoc! {r#"
            {"type":"user","uuid":"fixture-human-request","parentUuid":"fixture-current","origin":{"kind":"human"},"message":{"content":"A human follow-up request."}}
            {"type":"assistant","uuid":"fixture-human-response","parentUuid":"fixture-human-request","message":{"content":[{"type":"text","text":"A fixture response."}]}}
        "#},
    );
    let expected = parse_all(transcript.as_bytes(), empty_additions());

    assert_incremental_matches_full_parse_at_every_split_point(
        &transcript,
        &expected,
        Some(IncrementalAppend::Applied),
        "human-origin user message and assistant follow-up",
    );
}

#[test]
fn parallel_tool_result_after_an_active_followup_matches_a_full_parse_at_every_split_point() {
    let transcript = format!(
        "{}{}",
        PARALLEL_TOOL_RESULT_BRANCH_PREFIX, PARALLEL_TOOL_RESULT_WITH_FOLLOWUP_SUFFIX
    );
    let expected = (
        vec![
            ChatItem::User {
                rewind_id: Some("fixture-root".to_owned()),
                text: "Run two fixture tools.".to_owned(),
                timestamp: Some("2026-01-01T00:00:01+00:00".to_owned()),
                from: None,
                queued_prompt_id: None,
            },
            ChatItem::ToolSummary {
                summary: "Read ×1".to_owned(),
                operations: vec![ToolOperation {
                    tool: "Read".to_owned(),
                    target: Some("/fixture/a".to_owned()),
                    failed: false,
                }],
            },
            ChatItem::Assistant {
                text: "The first tool result is ready.".to_owned(),
            },
            ChatItem::Assistant {
                text: "The active branch continues.".to_owned(),
            },
        ],
        Vec::new(),
        Vec::new(),
        Vec::new(),
    );

    assert_incremental_matches_full_parse_at_every_split_point(
        &transcript,
        &expected,
        None,
        "parallel tool result after a follow-up",
    );
}

#[test]
fn late_parallel_result_after_a_rewound_tool_result_restarts_at_every_split_point() {
    let transcript = indoc! {r#"
        {"type":"assistant","uuid":"fixture-owner-a","parentUuid":null,"message":{"content":[{"type":"tool_use","id":"fixture-tool-a0","name":"Read","input":{"file_path":"/fixture/a0"}},{"type":"tool_use","id":"fixture-tool-a1","name":"Read","input":{"file_path":"/fixture/a1"}},{"type":"tool_use","id":"fixture-tool-a2","name":"Read","input":{"file_path":"/fixture/a2"}}]}}
        {"type":"assistant","uuid":"fixture-owner-b","parentUuid":"fixture-owner-a","message":{"content":[{"type":"tool_use","id":"fixture-tool-b0","name":"Read","input":{"file_path":"/fixture/b0"}},{"type":"tool_use","id":"fixture-tool-b1","name":"Read","input":{"file_path":"/fixture/b1"}},{"type":"tool_use","id":"fixture-tool-b2","name":"Read","input":{"file_path":"/fixture/b2"}}]}}
        {"type":"user","uuid":"fixture-result-a","parentUuid":"fixture-owner-a","message":{"content":[{"type":"tool_result","tool_use_id":"fixture-tool-a2","content":"fixture result"}]}}
        {"type":"user","uuid":"fixture-result-rewind","parentUuid":"fixture-result-a","message":{"content":[{"type":"tool_result","tool_use_id":"fixture-tool-a0","content":"fixture result"},{"type":"tool_result","tool_use_id":"fixture-tool-a1","content":"fixture result"}]}}
        {"type":"assistant","uuid":"fixture-response-after-rewind","parentUuid":"fixture-result-a","message":{"content":[{"type":"text","text":"Fixture response."}]}}
        {"type":"user","uuid":"fixture-result-b","parentUuid":"fixture-owner-b","sourceToolAssistantUUID":"fixture-owner-b","message":{"content":[{"type":"tool_result","tool_use_id":"fixture-tool-b1","content":"fixture result"}]}}
    "#};
    let expected = parse_all(transcript.as_bytes(), empty_additions());

    assert_incremental_matches_full_parse_at_every_split_point(
        transcript,
        &expected,
        None,
        "late parallel result after a rewound tool result",
    );

    let prefix = transcript.lines().take(5).collect::<Vec<_>>().join("\n") + "\n";
    let suffix = &transcript[prefix.len()..];
    assert_eq!(
        incremental_result(prefix.as_bytes(), suffix.as_bytes()).0,
        IncrementalAppend::RestartRequired,
    );
}

#[test]
fn assistant_after_sibling_hook_attachments_restarts_at_every_split_point() {
    let transcript = indoc! {r#"
        {"type":"assistant","uuid":"fixture-first-response","parentUuid":null,"message":{"content":[{"type":"text","text":"Fixture response one."}]}}
        {"type":"assistant","uuid":"fixture-active-response","parentUuid":"fixture-first-response","message":{"content":[{"type":"text","text":"Fixture response two."}]}}
        {"type":"attachment","uuid":"fixture-hook-active","parentUuid":"fixture-active-response","attachment":{"type":"hook_success","hookName":"PostToolUse"}}
        {"type":"attachment","uuid":"fixture-hook-sibling","parentUuid":"fixture-first-response","attachment":{"type":"hook_success","hookName":"PostToolUse"}}
        {"type":"assistant","uuid":"fixture-tool-after-hook","parentUuid":"fixture-hook-active","message":{"content":[{"type":"tool_use","id":"fixture-hook-tool","name":"Read","input":{"file_path":"/fixture/hook-target"}}]}}
    "#};
    let expected = parse_all(transcript.as_bytes(), empty_additions());

    assert_incremental_matches_full_parse_at_every_split_point(
        transcript,
        &expected,
        None,
        "assistant following sibling hook attachments",
    );

    let prefix = transcript.lines().take(4).collect::<Vec<_>>().join("\n") + "\n";
    let suffix = &transcript[prefix.len()..];
    assert_eq!(
        incremental_result(prefix.as_bytes(), suffix.as_bytes()).0,
        IncrementalAppend::RestartRequired,
    );
}

#[test]
fn compact_boundary_tool_result_matches_a_full_parse_at_every_split_point() {
    let transcript = format!(
        "{}{}",
        COMPACT_BRANCH_PENDING_TOOL_PREFIX, COMPACT_BRANCH_PENDING_TOOL_SUFFIX
    );
    let expected = (
        vec![
            ChatItem::User {
                rewind_id: Some("fixture-root".to_owned()),
                text: "Run a fixture tool.".to_owned(),
                timestamp: Some("2026-01-01T00:00:01+00:00".to_owned()),
                from: None,
                queued_prompt_id: None,
            },
            ChatItem::Event {
                kind: EventKind::CompactBoundary,
                text: "Context compacted".to_owned(),
                timestamp: Some("2026-01-01T00:00:02+00:00".to_owned()),
                failed: false,
            },
            ChatItem::ToolSummary {
                summary: "Read ×1".to_owned(),
                operations: vec![ToolOperation {
                    tool: "Read".to_owned(),
                    target: Some("/fixture/post-compact.md".to_owned()),
                    failed: false,
                }],
            },
        ],
        Vec::new(),
        Vec::new(),
        Vec::new(),
    );

    assert_incremental_matches_full_parse_at_every_split_point(
        &transcript,
        &expected,
        None,
        "compact-boundary tool result",
    );
}

#[test]
fn full_parse_keeps_a_pending_tool_result_with_a_missing_parent_when_owner_is_active() {
    let transcript = indoc! {r#"
        {"type":"system","uuid":"fixture-compact-boundary","parentUuid":null,"logicalParentUuid":"fixture-pruned-parent","subtype":"compact_boundary"}
        {"type":"assistant","uuid":"fixture-tool-owner","parentUuid":"fixture-compact-boundary","message":{"content":[{"type":"tool_use","id":"fixture-call-id","name":"Read","input":{"file_path":"/fixture/active.md"}}]}}
        {"type":"user","uuid":"fixture-result","parentUuid":"fixture-pruned-parent","sourceToolAssistantUUID":"fixture-tool-owner","message":{"content":[{"type":"tool_result","tool_use_id":"fixture-call-id","content":"fixture output"}]}}
        {"type":"assistant","uuid":"fixture-current","parentUuid":"fixture-tool-owner","message":{"content":[{"type":"text","text":"Current response."}]}}
    "#};

    assert_eq!(
        parse_all(transcript.as_bytes(), empty_additions()),
        (
            vec![
                ChatItem::Event {
                    kind: EventKind::CompactBoundary,
                    text: "Context compacted".to_owned(),
                    timestamp: None,
                    failed: false,
                },
                ChatItem::ToolSummary {
                    summary: "Read ×1".to_owned(),
                    operations: vec![ToolOperation {
                        tool: "Read".to_owned(),
                        target: Some("/fixture/active.md".to_owned()),
                        failed: false,
                    }],
                },
                ChatItem::Assistant {
                    text: "Current response.".to_owned(),
                },
            ],
            Vec::new(),
            Vec::new(),
            Vec::new(),
        ),
    );
}

#[test]
fn chained_tool_results_match_a_full_parse_at_every_split_point() {
    let transcript = indoc! {r#"
        {"type":"user","uuid":"fixture-root","parentUuid":null,"timestamp":"2026-01-01T00:00:01Z","message":{"content":"Run fixture tools."}}
        {"type":"assistant","uuid":"fixture-tool-owner","parentUuid":"fixture-root","timestamp":"2026-01-01T00:00:02Z","message":{"content":[{"type":"tool_use","id":"fixture-call-a","name":"Read","input":{"file_path":"/fixture/first.md"}},{"type":"tool_use","id":"fixture-call-b","name":"Read","input":{"file_path":"/fixture/second.md"}},{"type":"tool_use","id":"fixture-question-b","name":"AskUserQuestion","input":{"questions":[{"question":"Choose a fixture option","options":[{"label":"First option","description":"The first fixture choice."},{"label":"Second option","description":"The second fixture choice."}]}]}}]}}
        {"type":"user","uuid":"fixture-result-a","parentUuid":"fixture-tool-owner","timestamp":"2026-01-01T00:00:03Z","message":{"content":[{"type":"tool_result","tool_use_id":"fixture-call-a","content":"first result"}]}}
        {"type":"user","uuid":"fixture-result-b","parentUuid":"fixture-result-a","timestamp":"2026-01-01T00:00:04Z","message":{"content":[{"type":"tool_result","tool_use_id":"fixture-call-b","content":"second result"},{"type":"tool_result","tool_use_id":"fixture-question-b","content":"Your questions have been answered: \"Choose a fixture option\"=\"Second option\""}]}}
    "#};
    let transcript_text = transcript.trim_end_matches('\n');
    let expected = (
        vec![
            ChatItem::User {
                rewind_id: Some("fixture-root".to_owned()),
                text: "Run fixture tools.".to_owned(),
                timestamp: Some("2026-01-01T00:00:01+00:00".to_owned()),
                from: None,
                queued_prompt_id: None,
            },
            ChatItem::Question {
                request_id: None,
                cancelled: false,
                questions: vec![QuestionPrompt {
                    header: None,
                    question: "Choose a fixture option".to_owned(),
                    choices: vec![
                        QuestionChoice {
                            label: "First option".to_owned(),
                            description: Some("The first fixture choice.".to_owned()),
                        },
                        QuestionChoice {
                            label: "Second option".to_owned(),
                            description: Some("The second fixture choice.".to_owned()),
                        },
                    ],
                    multi_select: false,
                    answer: Some("Second option".to_owned()),
                }],
            },
            ChatItem::ToolSummary {
                summary: "Read ×2".to_owned(),
                operations: vec![
                    ToolOperation {
                        tool: "Read".to_owned(),
                        target: Some("/fixture/first.md".to_owned()),
                        failed: false,
                    },
                    ToolOperation {
                        tool: "Read".to_owned(),
                        target: Some("/fixture/second.md".to_owned()),
                        failed: false,
                    },
                ],
            },
        ],
        Vec::new(),
        Vec::new(),
        Vec::new(),
    );

    assert_incremental_matches_full_parse_at_every_split_point(
        transcript_text,
        &expected,
        Some(IncrementalAppend::Applied),
        "chained tool results",
    );
}

#[rstest]
#[case::user_prompt(indoc! {r#"
    {"type":"user","uuid":"fixture-queued-prompt","parentUuid":"fixture-main-followup","message":{"content":"A later fixture request."}}
"#})]
#[case::task_notification_without_uuid(indoc! {r#"
    {"type":"user","parentUuid":"fixture-main-followup","origin":{"kind":"task-notification"},"message":{"content":"<task-notification><task-id>fixture-task</task-id><status>completed</status><summary>Fixture task completed</summary></task-notification>"}}
"#})]
#[case::task_notification_attachment_with_uuid(indoc! {r#"
    {"type":"attachment","uuid":"fixture-notification-attachment","parentUuid":"fixture-main-followup","attachment":{"type":"queued_command","origin":{"kind":"task-notification"},"prompt":"<task-notification><task-id>fixture-task</task-id><status>completed</status><summary>Fixture task completed</summary></task-notification>"}}
"#})]
#[case::task_notification_attachment_without_uuid(indoc! {r#"
    {"type":"attachment","parentUuid":"fixture-main-followup","attachment":{"type":"queued_command","origin":{"kind":"task-notification"},"prompt":"<task-notification><task-id>fixture-task</task-id><status>completed</status><summary>Fixture task completed</summary></task-notification>"}}
"#})]
fn assistant_after_a_prompt_or_task_notification_restarts_an_old_side_branch(
    #[case] boundary_record: &str,
) {
    let prefix = format!(
        "{}{}",
        indoc! {r#"
        {"type":"user","uuid":"fixture-root","parentUuid":null,"message":{"content":"Run two fixture tools."}}
        {"type":"assistant","uuid":"fixture-main-call","parentUuid":"fixture-root","message":{"content":[{"type":"tool_use","id":"fixture-main-use","name":"Read","input":{"file_path":"/fixture/main.md"}}]}}
        {"type":"assistant","uuid":"fixture-side-call","parentUuid":"fixture-main-call","message":{"content":[{"type":"tool_use","id":"fixture-side-use","name":"Read","input":{"file_path":"/fixture/side.md"}}]}}
        {"type":"user","uuid":"fixture-main-result","parentUuid":"fixture-main-call","message":{"content":[{"type":"tool_result","tool_use_id":"fixture-main-use","content":"main output"}]}}
        {"type":"assistant","uuid":"fixture-main-followup","parentUuid":"fixture-main-result","message":{"content":[{"type":"text","text":"The main branch continued."}]}}
    "#},
        boundary_record,
    );
    let suffix = indoc! {r#"
        {"type":"assistant","uuid":"fixture-side-followup","parentUuid":"fixture-side-call","message":{"content":[{"type":"text","text":"The side branch continued."}]}}
    "#};
    let transcript = format!("{prefix}{suffix}");
    let expected = parse_all(transcript.as_bytes(), empty_additions());

    assert_incremental_matches_full_parse_at_every_split_point(
        &transcript,
        &expected,
        None,
        "prompt or notification boundary",
    );
    assert_eq!(
        incremental_result(prefix.as_bytes(), suffix.as_bytes()).0,
        IncrementalAppend::RestartRequired,
    );
}

#[test]
fn assistant_after_a_task_notification_and_its_followup_restarts_an_old_side_branch() {
    let prefix = indoc! {r#"
        {"type":"user","uuid":"fixture-root","parentUuid":null,"message":{"content":"Run two fixture tools."}}
        {"type":"assistant","uuid":"fixture-main-call","parentUuid":"fixture-root","message":{"content":[{"type":"tool_use","id":"fixture-main-use","name":"Read","input":{"file_path":"/fixture/main.md"}}]}}
        {"type":"assistant","uuid":"fixture-side-call","parentUuid":"fixture-main-call","message":{"content":[{"type":"tool_use","id":"fixture-side-use","name":"Read","input":{"file_path":"/fixture/side.md"}}]}}
        {"type":"user","uuid":"fixture-main-result","parentUuid":"fixture-main-call","message":{"content":[{"type":"tool_result","tool_use_id":"fixture-main-use","content":"main output"}]}}
        {"type":"assistant","uuid":"fixture-main-followup","parentUuid":"fixture-main-result","message":{"content":[{"type":"text","text":"The main branch continued."}]}}
        {"type":"user","parentUuid":"fixture-main-followup","origin":{"kind":"task-notification"},"message":{"content":"<task-notification><task-id>fixture-task</task-id><status>completed</status><summary>Fixture task completed</summary></task-notification>"}}
        {"type":"assistant","uuid":"fixture-main-followup-2","parentUuid":"fixture-main-followup","message":{"content":[{"type":"text","text":"The main branch continued again."}]}}
    "#};
    let suffix = indoc! {r#"
        {"type":"assistant","uuid":"fixture-side-followup","parentUuid":"fixture-side-call","message":{"content":[{"type":"text","text":"The side branch continued."}]}}
    "#};
    let transcript = format!("{prefix}{suffix}");
    let expected = parse_all(transcript.as_bytes(), empty_additions());

    assert_incremental_matches_full_parse_at_every_split_point(
        &transcript,
        &expected,
        None,
        "task notification followed by assistant continuation",
    );
    assert_eq!(
        incremental_result(prefix.as_bytes(), suffix.as_bytes()).0,
        IncrementalAppend::RestartRequired,
    );
}

#[test]
fn generated_transcripts_match_full_parse_at_record_byte_and_three_chunk_splits() {
    const SAMPLE_COUNT: u64 = 1_750;
    const SEED_COUNT: u64 = 60_000;

    for sample_index in 0..SAMPLE_COUNT {
        let seed = sample_index * (SEED_COUNT - 1) / (SAMPLE_COUNT - 1);
        for include_attachments in [false, true] {
            for (generator, records) in [
                (
                    "mixed_tool_and_notification_records",
                    generated_transcript(seed, include_attachments),
                ),
                (
                    "parallel_tool_burst_records",
                    generated_transcript_with_tool_bursts(seed, include_attachments),
                ),
            ] {
                assert_generated_transcript_matches_full_parse(
                    &records,
                    seed,
                    sample_index,
                    generator,
                    include_attachments,
                );
            }
        }
    }
}

#[rstest]
#[case::rewound_hook_branch_does_not_keep_parallel_tool_results(
    indoc! {r#"
        {"type":"user","uuid":"fixture-root","parentUuid":null,"message":{"content":"First request."}}
        {"type":"assistant","uuid":"fixture-old-response","parentUuid":"fixture-root","message":{"content":[{"type":"text","text":"Old response."}]}}
        {"type":"attachment","uuid":"fixture-hook","parentUuid":"fixture-old-response","attachment":{"type":"hook_success","hookName":"PostToolUse"}}
        {"type":"assistant","uuid":"fixture-hook-response","parentUuid":"fixture-hook","message":{"content":[{"type":"text","text":"Old hook response."}]}}
        {"type":"user","uuid":"fixture-replacement","parentUuid":"fixture-root","message":{"content":"Replacement request."}}
        {"type":"assistant","uuid":"fixture-call-a","parentUuid":"fixture-replacement","message":{"content":[{"type":"tool_use","id":"fixture-tool-a","name":"Read","input":{"file_path":"/fixture/a"}}]}}
        {"type":"assistant","uuid":"fixture-call-b","parentUuid":"fixture-call-a","message":{"content":[{"type":"tool_use","id":"fixture-tool-b","name":"Read","input":{"file_path":"/fixture/b"}}]}}
        {"type":"assistant","uuid":"fixture-call-c","parentUuid":"fixture-call-b","message":{"content":[{"type":"tool_use","id":"fixture-tool-c","name":"Read","input":{"file_path":"/fixture/c"}}]}}
        {"type":"user","uuid":"fixture-result-c","parentUuid":"fixture-call-c","sourceToolAssistantUUID":"fixture-call-c","message":{"content":[{"type":"tool_result","tool_use_id":"fixture-tool-c","content":"Fixture result."}]}}
        {"type":"assistant","uuid":"fixture-call-d","parentUuid":"fixture-call-b","message":{"content":[{"type":"tool_use","id":"fixture-tool-d","name":"Read","input":{"file_path":"/fixture/d"}}]}}
    "#},
    (
        vec![
            ChatItem::User {
                text: "First request.".to_owned(),
                rewind_id: Some("fixture-root".to_owned()),
                queued_prompt_id: None,
                timestamp: None,
                from: None,
            },
            ChatItem::User {
                text: "Replacement request.".to_owned(),
                rewind_id: Some("fixture-replacement".to_owned()),
                queued_prompt_id: None,
                timestamp: None,
                from: None,
            },
            ChatItem::ToolInProgress {
                tool: "Read".to_owned(),
                target: Some("/fixture/d".to_owned()),
            },
        ],
        Vec::new(),
        Vec::new(),
        Vec::new(),
    ),
)]
#[case::uuid_task_notification_keeps_its_tool_result(
    indoc! {r#"
        {"type":"assistant","uuid":"fixture-root-call","parentUuid":null,"message":{"content":[{"type":"tool_use","id":"fixture-root-tool","name":"Read","input":{"file_path":"/fixture/root"}}]}}
        {"type":"assistant","uuid":"fixture-root-followup","parentUuid":"fixture-root-call","message":{"content":[{"type":"text","text":"Fixture text."}]}}
        {"type":"assistant","uuid":"fixture-side-call","parentUuid":"fixture-root-followup","message":{"content":[{"type":"tool_use","id":"fixture-side-a","name":"Read","input":{"file_path":"/fixture/side-a"}},{"type":"tool_use","id":"fixture-side-b","name":"Read","input":{"file_path":"/fixture/side-b"}}]}}
        {"type":"user","uuid":"fixture-notification","parentUuid":"fixture-root-call","origin":{"kind":"task-notification"},"message":{"content":"<task-notification><task-id>fixture-task</task-id><status>completed</status></task-notification>"}}
        {"type":"user","uuid":"fixture-root-result","parentUuid":"fixture-notification","message":{"content":[{"type":"tool_result","tool_use_id":"fixture-root-tool","content":"Fixture result."}]}}
        {"type":"user","uuid":"fixture-prompt","parentUuid":"fixture-root-result","message":{"content":"Next request."}}
        {"type":"assistant","uuid":"fixture-next-call","parentUuid":"fixture-prompt","message":{"content":[{"type":"tool_use","id":"fixture-next-a","name":"Read","input":{"file_path":"/fixture/next-a"}},{"type":"tool_use","id":"fixture-next-b","name":"Read","input":{"file_path":"/fixture/next-b"}}]}}
    "#},
    (
        vec![
            ChatItem::Event {
                kind: EventKind::System,
                text: "Background task fixture-task completed".to_owned(),
                failed: false,
                timestamp: None,
            },
            ChatItem::ToolSummary {
                summary: "Read ×1".to_owned(),
                operations: vec![ToolOperation {
                    tool: "Read".to_owned(),
                    target: Some("/fixture/root".to_owned()),
                    failed: false,
                }],
            },
            ChatItem::User {
                text: "Next request.".to_owned(),
                rewind_id: Some("fixture-prompt".to_owned()),
                queued_prompt_id: None,
                timestamp: None,
                from: None,
            },
            ChatItem::ToolInProgress {
                tool: "Read".to_owned(),
                target: Some("/fixture/next-b".to_owned()),
            },
        ],
        Vec::new(),
        Vec::new(),
        Vec::new(),
    ),
)]
#[case::active_branch_pending_tool_survives_an_older_result(
    indoc! {r#"
        {"type":"user","uuid":"fixture-notification","parentUuid":null,"origin":{"kind":"task-notification"},"message":{"content":"<task-notification><task-id>fixture-task</task-id><status>completed</status></task-notification>"}}
        {"type":"assistant","uuid":"fixture-first-call","parentUuid":"fixture-notification","message":{"content":[{"type":"tool_use","id":"fixture-first-a","name":"Read","input":{"file_path":"/fixture/first-a"}},{"type":"tool_use","id":"fixture-first-b","name":"Read","input":{"file_path":"/fixture/first-b"}}]}}
        {"type":"user","uuid":"fixture-first-result","parentUuid":"fixture-first-call","sourceToolAssistantUUID":"fixture-first-call","message":{"content":[{"type":"tool_result","tool_use_id":"fixture-first-a","content":"Fixture result."}]}}
        {"type":"user","uuid":"fixture-second-result","parentUuid":"fixture-notification","message":{"content":[{"type":"tool_result","tool_use_id":"fixture-first-b","content":"Fixture result."}]}}
        {"type":"assistant","uuid":"fixture-side-call","parentUuid":"fixture-notification","message":{"content":[{"type":"tool_use","id":"fixture-side-a","name":"Read","input":{"file_path":"/fixture/side-a"}},{"type":"tool_use","id":"fixture-side-b","name":"Read","input":{"file_path":"/fixture/side-b"}}]}}
        {"type":"user","uuid":"fixture-duplicate-result","parentUuid":"fixture-first-call","message":{"content":[{"type":"tool_result","tool_use_id":"fixture-first-b","content":"Fixture result."}]}}
        {"type":"assistant","uuid":"fixture-active-call","parentUuid":"fixture-notification","message":{"content":[{"type":"text","text":"Active branch."},{"type":"tool_use","id":"fixture-active-a","name":"Read","input":{"file_path":"/fixture/active-a"}},{"type":"tool_use","id":"fixture-active-b","name":"Read","input":{"file_path":"/fixture/active-b"}},{"type":"tool_use","id":"fixture-active-c","name":"Read","input":{"file_path":"/fixture/active-c"}}]}}
        {"type":"assistant","uuid":"fixture-nested-call","parentUuid":"fixture-active-call","message":{"content":[{"type":"tool_use","id":"fixture-nested-a","name":"Read","input":{"file_path":"/fixture/nested-a"}},{"type":"tool_use","id":"fixture-nested-b","name":"Read","input":{"file_path":"/fixture/nested-b"}}]}}
        {"type":"user","uuid":"fixture-active-result","parentUuid":"fixture-active-call","sourceToolAssistantUUID":"fixture-active-call","message":{"content":[{"type":"tool_result","tool_use_id":"fixture-active-a","content":"Fixture result."}]}}
        {"type":"assistant","uuid":"fixture-active-followup","parentUuid":"fixture-active-result","message":{"content":[{"type":"text","text":"Active follow-up."}]}}
    "#},
    (
        vec![
            ChatItem::Event {
                kind: EventKind::System,
                text: "Background task fixture-task completed".to_owned(),
                failed: false,
                timestamp: None,
            },
            ChatItem::Assistant {
                text: "Active branch.".to_owned(),
            },
            ChatItem::ToolSummary {
                summary: "Read ×1".to_owned(),
                operations: vec![ToolOperation {
                    tool: "Read".to_owned(),
                    target: Some("/fixture/active-a".to_owned()),
                    failed: false,
                }],
            },
            ChatItem::Assistant {
                text: "Active follow-up.".to_owned(),
            },
            ChatItem::ToolInProgress {
                tool: "Read".to_owned(),
                target: Some("/fixture/active-c".to_owned()),
            },
        ],
        Vec::new(),
        Vec::new(),
        Vec::new(),
    ),
)]
#[case::rewound_send_user_file_result_is_hidden(
    indoc! {r#"
        {"type":"user","uuid":"fixture-root","parentUuid":null,"message":{"content":"Root request."}}
        {"type":"assistant","uuid":"fixture-active-tool","parentUuid":"fixture-root","message":{"content":[{"type":"tool_use","id":"fixture-active-call","name":"Read","input":{"file_path":"/fixture/active.md"}}]}}
        {"type":"assistant","uuid":"fixture-stale-file-call","parentUuid":"fixture-active-tool","message":{"content":[{"type":"tool_use","id":"fixture-stale-file-use","name":"SendUserFile","input":{"files":["/fixture/hidden.png"],"caption":"Stale file."}}]}}
        {"type":"user","uuid":"fixture-replacement","parentUuid":"fixture-active-tool","message":{"content":"Replacement request."}}
        {"type":"assistant","uuid":"fixture-response","parentUuid":"fixture-replacement","message":{"content":[{"type":"text","text":"Current response."}]}}
        {"type":"user","parentUuid":"fixture-stale-file-call","message":{"content":[{"type":"tool_result","tool_use_id":"fixture-stale-file-use","content":"File sent."}]}}
    "#},
    (
        vec![
            ChatItem::User {
                text: "Root request.".to_owned(),
                rewind_id: Some("fixture-root".to_owned()),
                timestamp: None,
                from: None,
                queued_prompt_id: None,
            },
            ChatItem::User {
                text: "Replacement request.".to_owned(),
                rewind_id: Some("fixture-replacement".to_owned()),
                timestamp: None,
                from: None,
                queued_prompt_id: None,
            },
            ChatItem::Assistant {
                text: "Current response.".to_owned(),
            },
            ChatItem::ToolInProgress {
                tool: "Read".to_owned(),
                target: Some("/fixture/active.md".to_owned()),
            },
        ],
        Vec::new(),
        Vec::new(),
        Vec::new(),
    ),
)]
#[case::forward_parent_membership_matches_master(
    indoc! {r#"
        {"type":"user","uuid":"fixture-root","parentUuid":null,"message":{"content":"Root request."}}
        {"type":"assistant","uuid":"fixture-early-child","parentUuid":"fixture-future-parent","message":{"content":[{"type":"text","text":"Discard the earlier child."}]}}
        {"type":"assistant","uuid":"fixture-future-parent","parentUuid":"fixture-root","message":{"content":[{"type":"text","text":"Keep the forward parent."}]}}
        {"type":"assistant","uuid":"fixture-current-child","parentUuid":"fixture-future-parent","message":{"content":[{"type":"text","text":"Keep the current response."}]}}
    "#},
    (
        vec![
            ChatItem::User {
                text: "Root request.".to_owned(),
                rewind_id: Some("fixture-root".to_owned()),
                timestamp: None,
                from: None,
                queued_prompt_id: None,
            },
            ChatItem::Assistant {
                text: "Keep the forward parent.".to_owned(),
            },
            ChatItem::Assistant {
                text: "Keep the current response.".to_owned(),
            },
        ],
        Vec::new(),
        Vec::new(),
        Vec::new(),
    ),
)]
fn full_parse_preserves_master_branch_output(
    #[case] transcript: &str,
    #[case] expected: super::ParsedTranscript,
) {
    assert_eq!(
        parse_all(transcript.as_bytes(), empty_additions()),
        expected,
    );
}

fn assert_generated_transcript_matches_full_parse(
    records: &[serde_json::Value],
    seed: u64,
    sample_index: u64,
    generator: &str,
    include_attachments: bool,
) {
    let transcript = serialize_fixture_transcript(records);
    let bytes = transcript.as_bytes();
    let expected = parse_all(bytes, empty_additions());
    let boundaries = fixture_record_boundaries(records);
    let context = format!("{generator}, seed {seed}, attachments {include_attachments}");

    for (record_split, &split) in boundaries.iter().enumerate() {
        if let Some(actual) = incremental_applied_result(&bytes[..split], &bytes[split..]) {
            assert_eq!(
                actual,
                expected.clone(),
                "{context}: record-boundary split after record {record_split}\n{transcript}",
            );
        }
    }

    if sample_index.is_multiple_of(100) {
        for split in 0..=bytes.len() {
            if !transcript.is_char_boundary(split) {
                continue;
            }
            if let Some(actual) = incremental_applied_result(&bytes[..split], &bytes[split..]) {
                assert_eq!(
                    actual,
                    expected.clone(),
                    "{context}: byte split at offset {split}\n{transcript}",
                );
            }
        }
    }

    if sample_index.is_multiple_of(10) {
        for first_split in 0..boundaries.len().saturating_sub(2) {
            for second_split in first_split + 1..boundaries.len() - 1 {
                let first = boundaries[first_split];
                let second = boundaries[second_split];
                let Some(actual) = incremental_three_chunk_result(
                    &bytes[..first],
                    &bytes[first..second],
                    &bytes[second..],
                ) else {
                    continue;
                };
                assert_eq!(
                    actual,
                    expected.clone(),
                    "{context}: three-chunk split after records {first_split} and {second_split}\n{transcript}",
                );
            }
        }
    }
}

#[rstest]
#[case::late_result_after_a_pending_tool_use_child(
    indoc! {r#"
        {"type":"assistant","uuid":"fixture-owner-a","parentUuid":null,"message":{"content":[{"type":"tool_use","id":"fixture-call-a","name":"Read","input":{"file_path":"/fixture/a.md"}}]}}
        {"type":"user","uuid":"fixture-result-a","parentUuid":"fixture-owner-a","message":{"content":[{"type":"tool_result","tool_use_id":"fixture-call-a","content":"first result"}]}}
        {"type":"assistant","uuid":"fixture-owner-b","parentUuid":"fixture-owner-a","message":{"content":[{"type":"tool_use","id":"fixture-call-b","name":"Read","input":{"file_path":"/fixture/b.md"}}]}}
        {"type":"assistant","uuid":"fixture-child-call","parentUuid":"fixture-owner-b","message":{"content":[{"type":"tool_use","id":"fixture-call-c","name":"Read","input":{"file_path":"/fixture/c.md"}}]}}
        {"type":"assistant","uuid":"fixture-followup","parentUuid":"fixture-result-a","message":{"content":[{"type":"text","text":"The first result is ready."}]}}
    "#},
    indoc! {r#"
        {"type":"user","uuid":"fixture-result-b","parentUuid":"fixture-owner-b","message":{"content":[{"type":"tool_result","tool_use_id":"fixture-call-b","content":"second result"}]}}
    "#},
    "late result after pending tool-use child",
    Some(IncrementalAppend::RestartRequired),
)]
#[case::result_sibling_after_a_result_followup(
    indoc! {r#"
        {"type":"assistant","uuid":"fixture-owner","parentUuid":null,"message":{"content":[{"type":"tool_use","id":"fixture-call-a","name":"Read","input":{"file_path":"/fixture/a.md"}},{"type":"tool_use","id":"fixture-call-b","name":"Read","input":{"file_path":"/fixture/b.md"}}]}}
        {"type":"user","uuid":"fixture-result-a","parentUuid":"fixture-owner","message":{"content":[{"type":"tool_result","tool_use_id":"fixture-call-a","content":"first result"}]}}
        {"type":"assistant","uuid":"fixture-followup","parentUuid":"fixture-result-a","message":{"content":[{"type":"text","text":"The first result is ready."}]}}
    "#},
    indoc! {r#"
        {"type":"user","uuid":"fixture-result-b","parentUuid":"fixture-result-a","message":{"content":[{"type":"tool_result","tool_use_id":"fixture-call-b","content":"second result"}]}}
    "#},
    "result sibling after a result follow-up",
    Some(IncrementalAppend::RestartRequired),
)]
#[case::text_branch_followed_by_another_owner_result(
    indoc! {r#"
        {"type":"assistant","uuid":"fixture-owner","parentUuid":null,"message":{"content":[{"type":"tool_use","id":"fixture-call-a","name":"Read","input":{"file_path":"/fixture/a.md"}},{"type":"tool_use","id":"fixture-call-b","name":"Read","input":{"file_path":"/fixture/b.md"}}]}}
        {"type":"user","uuid":"fixture-result-b","parentUuid":"fixture-owner","message":{"content":[{"type":"tool_result","tool_use_id":"fixture-call-b","content":"second result"}]}}
        {"type":"assistant","uuid":"fixture-followup-one","parentUuid":"fixture-result-b","message":{"content":[{"type":"text","text":"First follow-up."}]}}
        {"type":"assistant","uuid":"fixture-followup-two","parentUuid":"fixture-followup-one","message":{"content":[{"type":"text","text":"Second follow-up."}]}}
    "#},
    indoc! {r#"
        {"type":"user","uuid":"fixture-result-a","parentUuid":"fixture-owner","message":{"content":[{"type":"tool_result","tool_use_id":"fixture-call-a","content":"first result"}]}}
    "#},
    "late result after a text branch",
    Some(IncrementalAppend::RestartRequired),
)]
#[case::result_chain_crosses_a_task_notification_boundary(
    indoc! {r#"
        {"type":"assistant","uuid":"fixture-owner","parentUuid":null,"message":{"content":[{"type":"tool_use","id":"fixture-call-a","name":"Read","input":{"file_path":"/fixture/a.md"}},{"type":"tool_use","id":"fixture-call-b","name":"Read","input":{"file_path":"/fixture/b.md"}}]}}
        {"type":"user","parentUuid":"fixture-owner","origin":{"kind":"task-notification"},"message":{"content":"<task-notification><task-id>fixture-task</task-id><status>completed</status></task-notification>"}}
        {"type":"assistant","uuid":"fixture-followup-call","parentUuid":"fixture-owner","message":{"content":[{"type":"tool_use","id":"fixture-call-c","name":"Read","input":{"file_path":"/fixture/c.md"}}]}}
        {"type":"user","uuid":"fixture-prompt","parentUuid":"fixture-followup-call","message":{"content":"A later fixture request."}}
        {"type":"assistant","uuid":"fixture-prompt-followup","parentUuid":"fixture-prompt","message":{"content":[{"type":"text","text":"The later request is being handled."}]}}
        {"type":"user","uuid":"fixture-followup-result","parentUuid":"fixture-followup-call","message":{"content":[{"type":"tool_result","tool_use_id":"fixture-call-c","content":"third result"}]}}
        {"type":"user","uuid":"fixture-original-result","parentUuid":"fixture-owner","message":{"content":[{"type":"tool_result","tool_use_id":"fixture-call-a","content":"first result"}]}}
    "#},
    indoc! {r#"
        {"type":"user","parentUuid":"fixture-original-result","origin":{"kind":"task-notification"},"message":{"content":"<task-notification><task-id>fixture-task</task-id><status>completed</status></task-notification>"}}
    "#},
    "late result after notification boundary",
    None,
)]
#[case::hook_attachment_keeps_a_rewound_prompt_out_of_an_append(
    indoc! {r#"
        {"type":"user","uuid":"fixture-prompt-one","parentUuid":"fixture-old-root","message":{"content":"First fixture request."}}
        {"type":"user","uuid":"fixture-prompt-two","parentUuid":"fixture-prompt-one","message":{"content":"Second fixture request."}}
        {"type":"assistant","uuid":"fixture-tool-call","parentUuid":"fixture-prompt-two","message":{"content":[{"type":"tool_use","id":"fixture-call","name":"Read","input":{"file_path":"/fixture/request.md"}}]}}
        {"type":"assistant","uuid":"fixture-tool-followup","parentUuid":"fixture-tool-call","message":{"content":[{"type":"text","text":"The tool call is continuing."}]}}
        {"type":"attachment","uuid":"fixture-hook","parentUuid":"fixture-prompt-one","attachment":{"type":"hook_success","hookName":"PostToolUse"}}
    "#},
    indoc! {r#"
        {"type":"assistant","uuid":"fixture-late-followup","parentUuid":"fixture-tool-followup","message":{"content":[{"type":"text","text":"The earlier branch continued."}]}}
    "#},
    "assistant continuation after a rewinding hook attachment",
    Some(IncrementalAppend::RestartRequired),
)]
#[case::result_chain_crosses_tool_owners(
    indoc! {r#"
        {"type":"assistant","uuid":"fixture-owner-a","parentUuid":null,"message":{"content":[{"type":"tool_use","id":"fixture-call-a","name":"Read","input":{"file_path":"/fixture/a.md"}}]}}
        {"type":"assistant","uuid":"fixture-owner-b","parentUuid":"fixture-owner-a","message":{"content":[{"type":"tool_use","id":"fixture-call-b","name":"Read","input":{"file_path":"/fixture/b.md"}},{"type":"tool_use","id":"fixture-call-c","name":"Read","input":{"file_path":"/fixture/c.md"}}]}}
        {"type":"user","uuid":"fixture-result-a","parentUuid":"fixture-owner-a","sourceToolAssistantUUID":"fixture-owner-a","message":{"content":[{"type":"tool_result","tool_use_id":"fixture-call-a","content":"first result"}]}}
    "#},
    indoc! {r#"
        {"type":"user","uuid":"fixture-result-b","parentUuid":"fixture-result-a","message":{"content":[{"type":"tool_result","tool_use_id":"fixture-call-b","content":"second result"}]}}
    "#},
    "result parent belongs to another tool owner",
    Some(IncrementalAppend::Applied),
)]
fn branch_ambiguities_match_full_parse_across_all_byte_splits(
    #[case] prefix: &str,
    #[case] suffix: &str,
    #[case] context: &str,
    #[case] expected_append: Option<IncrementalAppend>,
) {
    assert_append_matches_full_parse_at_record_boundary(prefix, suffix, expected_append, context);
}

#[test]
fn late_parallel_result_restarts_when_it_would_replace_a_user_branch() {
    let prefix = indoc! {r#"
        {"type":"user","uuid":"fixture-root","parentUuid":null,"message":{"content":"Run two fixture tools."}}
        {"type":"assistant","uuid":"fixture-tool-owner","parentUuid":"fixture-root","message":{"content":[{"type":"tool_use","id":"fixture-call-a","name":"Read","input":{"file_path":"/fixture/a.md"}},{"type":"tool_use","id":"fixture-call-b","name":"Read","input":{"file_path":"/fixture/b.md"}}]}}
        {"type":"user","uuid":"fixture-result-a","parentUuid":"fixture-tool-owner","message":{"content":[{"type":"tool_result","tool_use_id":"fixture-call-a","content":"first result"}]}}
        {"type":"assistant","uuid":"fixture-followup","parentUuid":"fixture-result-a","message":{"content":[{"type":"text","text":"The first result is ready."}]}}
        {"type":"user","uuid":"fixture-replacement","parentUuid":"fixture-tool-owner","message":{"content":"A replacement fixture request."}}
        {"type":"assistant","uuid":"fixture-replacement-followup","parentUuid":"fixture-replacement","message":{"content":[{"type":"text","text":"The replacement branch continues."}]}}
    "#};
    let suffix = indoc! {r#"
        {"type":"user","uuid":"fixture-result-b","parentUuid":"fixture-tool-owner","message":{"content":[{"type":"tool_result","tool_use_id":"fixture-call-b","content":"second result"}]}}
    "#};
    let transcript = format!("{prefix}{suffix}");
    let expected = parse_all(transcript.as_bytes(), empty_additions());

    assert_incremental_matches_full_parse_at_every_split_point(
        &transcript,
        &expected,
        None,
        "late parallel result after a user branch",
    );
    assert_eq!(
        incremental_result(prefix.as_bytes(), suffix.as_bytes()).0,
        IncrementalAppend::RestartRequired,
    );
}

#[test]
fn old_tool_use_owner_does_not_keep_a_sibling_assistant_active_after_a_prompt() {
    let transcript = indoc! {r#"
        {"type":"user","uuid":"fixture-root","parentUuid":null,"message":{"content":"Run the original request."}}
        {"type":"assistant","uuid":"fixture-tool-owner","parentUuid":"fixture-root","message":{"content":[{"type":"tool_use","id":"fixture-original-use","name":"Read","input":{"file_path":"/fixture/original.md"}}]}}
        {"type":"user","uuid":"fixture-replacement","parentUuid":"fixture-tool-owner","message":{"content":"Use the replacement request."}}
        {"type":"assistant","uuid":"fixture-replacement-response","parentUuid":"fixture-replacement","message":{"content":[{"type":"text","text":"The replacement response."}]}}
        {"type":"assistant","uuid":"fixture-stale-tool","parentUuid":"fixture-tool-owner","message":{"content":[{"type":"tool_use","id":"fixture-stale-use","name":"Write","input":{"file_path":"/fixture/stale.md","content":"stale"}}]}}
        {"type":"assistant","uuid":"fixture-replacement-followup","parentUuid":"fixture-replacement-response","message":{"content":[{"type":"text","text":"The replacement branch continued."}]}}
    "#};
    let expected: super::ParsedTranscript = (
        vec![
            ChatItem::User {
                rewind_id: Some("fixture-root".to_owned()),
                text: "Run the original request.".to_owned(),
                timestamp: None,
                from: None,
                queued_prompt_id: None,
            },
            ChatItem::User {
                rewind_id: Some("fixture-replacement".to_owned()),
                text: "Use the replacement request.".to_owned(),
                timestamp: None,
                from: None,
                queued_prompt_id: None,
            },
            ChatItem::Assistant {
                text: "The replacement response.".to_owned(),
            },
            ChatItem::Assistant {
                text: "The replacement branch continued.".to_owned(),
            },
            ChatItem::ToolInProgress {
                tool: "Read".to_owned(),
                target: Some("/fixture/original.md".to_owned()),
            },
        ],
        Vec::new(),
        Vec::new(),
        Vec::new(),
    );

    assert_eq!(
        parse_all(transcript.as_bytes(), empty_additions()),
        expected
    );
    assert_incremental_matches_full_parse_at_every_split_point(
        transcript,
        &expected,
        None,
        "stale tool-use owner after a user prompt",
    );
}

#[test]
fn assistant_from_a_side_branch_restarts_after_another_branch_updates_the_tip() {
    let prefix = indoc! {r#"
        {"type":"user","uuid":"fixture-root","parentUuid":null,"message":{"content":"Run two fixture tools."}}
        {"type":"assistant","uuid":"fixture-main-call","parentUuid":"fixture-root","message":{"content":[{"type":"tool_use","id":"fixture-main-use","name":"Read","input":{"file_path":"/fixture/main.md"}}]}}
        {"type":"assistant","uuid":"fixture-side-call","parentUuid":"fixture-main-call","message":{"content":[{"type":"tool_use","id":"fixture-side-use","name":"Read","input":{"file_path":"/fixture/side.md"}}]}}
        {"type":"user","uuid":"fixture-main-result","parentUuid":"fixture-main-call","message":{"content":[{"type":"tool_result","tool_use_id":"fixture-main-use","content":"main output"}]}}
        {"type":"assistant","uuid":"fixture-main-followup","parentUuid":"fixture-main-result","message":{"content":[{"type":"text","text":"The main branch continued."}]}}
    "#};
    let suffix = indoc! {r#"
        {"type":"assistant","uuid":"fixture-side-followup","parentUuid":"fixture-side-call","message":{"content":[{"type":"text","text":"The side branch continued."}]}}
    "#};
    let transcript = format!("{prefix}{suffix}");
    assert_eq!(
        incremental_result(prefix.as_bytes(), suffix.as_bytes()),
        (
            IncrementalAppend::RestartRequired,
            parse_all(transcript.as_bytes(), empty_additions())
        ),
    );
}

#[test]
fn chained_tool_results_from_a_rewound_branch_match_master_output() {
    let transcript = indoc! {r#"
        {"type":"user","uuid":"fixture-root","parentUuid":null,"timestamp":"2026-01-01T00:00:01Z","message":{"content":"Run fixture tools."}}
        {"type":"assistant","uuid":"fixture-tool-owner","parentUuid":"fixture-root","timestamp":"2026-01-01T00:00:02Z","message":{"content":[{"type":"tool_use","id":"fixture-call-a","name":"Read","input":{"file_path":"/fixture/first.md"}},{"type":"tool_use","id":"fixture-call-b","name":"Read","input":{"file_path":"/fixture/second.md"}},{"type":"tool_use","id":"fixture-question-b","name":"AskUserQuestion","input":{"questions":[{"question":"Choose a fixture option","options":[{"label":"First option","description":"The first fixture choice."},{"label":"Second option","description":"The second fixture choice."}]}]}}]}}
        {"type":"user","uuid":"fixture-result-a","parentUuid":"fixture-tool-owner","timestamp":"2026-01-01T00:00:03Z","message":{"content":[{"type":"tool_result","tool_use_id":"fixture-call-a","content":"first result"}]}}
        {"type":"user","uuid":"fixture-result-b","parentUuid":"fixture-result-a","timestamp":"2026-01-01T00:00:04Z","message":{"content":[{"type":"tool_result","tool_use_id":"fixture-call-b","content":"second result"},{"type":"tool_result","tool_use_id":"fixture-question-b","content":"Your questions have been answered: \"Choose a fixture option\"=\"Second option\""}]}}
        {"type":"user","uuid":"fixture-replacement","parentUuid":"fixture-tool-owner","timestamp":"2026-01-01T00:00:05Z","message":{"content":"The replacement request."}}
        {"type":"assistant","uuid":"fixture-current","parentUuid":"fixture-replacement","timestamp":"2026-01-01T00:00:06Z","message":{"content":[{"type":"text","text":"The replacement response."}]}}
    "#};

    assert_eq!(
        parse_all(transcript.as_bytes(), empty_additions()),
        (
            vec![
                ChatItem::User {
                    rewind_id: Some("fixture-root".to_owned()),
                    text: "Run fixture tools.".to_owned(),
                    timestamp: Some("2026-01-01T00:00:01+00:00".to_owned()),
                    from: None,
                    queued_prompt_id: None,
                },
                ChatItem::Question {
                    request_id: None,
                    cancelled: false,
                    questions: vec![QuestionPrompt {
                        header: None,
                        question: "Choose a fixture option".to_owned(),
                        choices: vec![
                            QuestionChoice {
                                label: "First option".to_owned(),
                                description: Some("The first fixture choice.".to_owned()),
                            },
                            QuestionChoice {
                                label: "Second option".to_owned(),
                                description: Some("The second fixture choice.".to_owned()),
                            },
                        ],
                        multi_select: false,
                        answer: Some("Second option".to_owned()),
                    }],
                },
                ChatItem::ToolSummary {
                    summary: "Read ×2".to_owned(),
                    operations: vec![
                        ToolOperation {
                            tool: "Read".to_owned(),
                            target: Some("/fixture/first.md".to_owned()),
                            failed: false,
                        },
                        ToolOperation {
                            tool: "Read".to_owned(),
                            target: Some("/fixture/second.md".to_owned()),
                            failed: false,
                        },
                    ],
                },
                ChatItem::User {
                    rewind_id: Some("fixture-replacement".to_owned()),
                    text: "The replacement request.".to_owned(),
                    timestamp: Some("2026-01-01T00:00:05+00:00".to_owned()),
                    from: None,
                    queued_prompt_id: None,
                },
                ChatItem::Assistant {
                    text: "The replacement response.".to_owned(),
                },
            ],
            Vec::new(),
            Vec::new(),
            Vec::new(),
        ),
    );
}

#[test]
fn assistant_appended_after_an_attachment_in_a_separate_update_matches_a_full_parse() {
    let prefix = indoc! {r#"
        {"type":"user","uuid":"fixture-request","parentUuid":null,"timestamp":"2026-01-01T00:00:01Z","message":{"content":"A fixture request."}}
        {"type":"assistant","uuid":"fixture-response","parentUuid":"fixture-request","timestamp":"2026-01-01T00:00:02Z","message":{"content":[{"type":"text","text":"A fixture response."}]}}
    "#};
    let attachment = indoc! {r#"
        {"type":"attachment","uuid":"fixture-reminder","parentUuid":"fixture-response","timestamp":"2026-01-01T00:00:03Z","attachment":{"type":"silent_turn_reminder"}}
    "#};
    let assistant = indoc! {r#"
        {"type":"assistant","uuid":"fixture-followup","parentUuid":"fixture-reminder","timestamp":"2026-01-01T00:00:04Z","message":{"content":[{"type":"text","text":"A fixture follow-up."}]}}
    "#};
    let mut transcript = prefix.as_bytes().to_vec();
    transcript.extend_from_slice(attachment.as_bytes());
    transcript.extend_from_slice(assistant.as_bytes());
    let mut parser = incremental_parser(prefix.as_bytes());
    let attachment_append = parser
        .append_bytes(attachment.as_bytes(), fixed_time())
        .unwrap();
    if attachment_append == IncrementalAppend::RestartRequired {
        let mut reparsed = prefix.as_bytes().to_vec();
        reparsed.extend_from_slice(attachment.as_bytes());
        parser = incremental_parser(&reparsed);
    }
    let assistant_append = parser
        .append_bytes(assistant.as_bytes(), fixed_time())
        .unwrap();
    if assistant_append == IncrementalAppend::RestartRequired {
        parser = incremental_parser(&transcript);
    }

    assert_eq!(
        (
            attachment_append,
            assistant_append,
            parser.render_with(fixed_time(), empty_additions()).unwrap(),
        ),
        (
            IncrementalAppend::RestartRequired,
            IncrementalAppend::RestartRequired,
            parse_all(&transcript, empty_additions()),
        ),
    );
}

#[rstest]
#[case::hook_success(indoc! {r#"
    {"type":"attachment","uuid":"fixture-hook","parentUuid":"fixture-tool-result","attachment":{"type":"hook_success","hookName":"PostToolUse"}}
    {"type":"assistant","uuid":"fixture-followup","parentUuid":"fixture-hook","message":{"content":[{"type":"text","text":"The fixture tool completed."}]}}
"#})]
#[case::empty_queued_command(indoc! {r#"
    {"type":"attachment","uuid":"fixture-queued-command","parentUuid":"fixture-tool-result","attachment":{"type":"queued_command","prompt":""}}
    {"type":"assistant","uuid":"fixture-followup","parentUuid":"fixture-queued-command","message":{"content":[{"type":"text","text":"The fixture tool completed."}]}}
"#})]
fn transparent_tip_attachments_and_their_children_remain_incremental(
    #[case] appended_records: &str,
) {
    let transcript = format!(
        "{}{}",
        indoc! {r#"
            {"type":"user","uuid":"fixture-request","parentUuid":null,"message":{"content":"Run the fixture tool."}}
            {"type":"assistant","uuid":"fixture-tool-call","parentUuid":"fixture-request","message":{"content":[{"type":"tool_use","id":"fixture-tool-use","name":"Read","input":{"file_path":"/fixture/input.md"}}]}}
            {"type":"user","uuid":"fixture-tool-result","parentUuid":"fixture-tool-call","message":{"content":[{"type":"tool_result","tool_use_id":"fixture-tool-use","content":"fixture output"}]}}
        "#},
        appended_records,
    );
    let expected = parse_all(transcript.as_bytes(), empty_additions());

    assert_incremental_matches_full_parse_at_every_split_point(
        &transcript,
        &expected,
        Some(IncrementalAppend::Applied),
        "transparent tip attachment and its assistant child",
    );
}

#[test]
fn active_record_cache_tracks_a_tip_append_after_render() {
    let prefix = PARALLEL_TOOL_USE_RESULT_BRANCH_PREFIX;
    let suffix = indoc! {r#"
        {"type":"assistant","uuid":"fixture-active-cache-followup","parentUuid":"fixture-next-call","message":{"content":[{"type":"text","text":"The active branch continues."}]}}
    "#};
    let transcript = format!("{prefix}{suffix}");
    let mut parser = incremental_parser(prefix.as_bytes());
    let rendered_before = parser.render_with(fixed_time(), empty_additions()).unwrap();
    let append = parser
        .append_bytes(suffix.as_bytes(), fixed_time())
        .unwrap();
    let rendered_after = parser.render_with(fixed_time(), empty_additions()).unwrap();

    assert_eq!(
        (rendered_before, append, rendered_after,),
        (
            parse_all(prefix.as_bytes(), empty_additions()),
            IncrementalAppend::Applied,
            parse_all(transcript.as_bytes(), empty_additions()),
        ),
    );
}

#[test]
fn active_record_cache_tracks_a_parallel_result_branch_switch_after_render() {
    let prefix = PARALLEL_TOOL_RESULT_BRANCH_PREFIX;
    let suffix = PARALLEL_TOOL_RESULT_BRANCH_SUFFIX;
    let transcript = format!("{prefix}{suffix}");
    let mut parser = incremental_parser(prefix.as_bytes());
    let rendered_before = parser.render_with(fixed_time(), empty_additions()).unwrap();
    let append = parser
        .append_bytes(suffix.as_bytes(), fixed_time())
        .unwrap();
    let rendered_after = parser.render_with(fixed_time(), empty_additions()).unwrap();

    assert_eq!(
        (rendered_before, append, rendered_after,),
        (
            parse_all(prefix.as_bytes(), empty_additions()),
            IncrementalAppend::Applied,
            parse_all(transcript.as_bytes(), empty_additions()),
        ),
    );
}

#[test]
fn pending_sibling_renders_only_the_active_branch() {
    let transcript = INITIAL_PENDING_SIBLING_TRANSCRIPT.trim_end_matches('\n');
    let expected: super::ParsedTranscript = (
        vec![
            ChatItem::User {
                rewind_id: Some("root".to_owned()),
                text: "The original request.".to_owned(),
                timestamp: None,
                from: None,
                queued_prompt_id: None,
            },
            ChatItem::Assistant {
                text: "The replacement response.".to_owned(),
            },
        ],
        Vec::new(),
        Vec::new(),
        Vec::new(),
    );

    assert_eq!(
        incremental_parser(transcript.as_bytes())
            .render_with(fixed_time(), empty_additions())
            .unwrap(),
        expected,
    );
}

#[test]
fn tail_window_matches_the_full_parse_after_a_compact_boundary_with_a_missing_parent() {
    let mut records = vec![
        r#"{"type":"user","uuid":"fixture-old-root","parentUuid":null,"timestamp":"2026-01-01T00:00:01Z","message":{"content":"Earlier context."}}"#.to_owned(),
        r#"{"type":"system","uuid":"fixture-compact","parentUuid":null,"logicalParentUuid":"fixture-unrecorded-parent","timestamp":"2026-01-01T00:00:02Z","subtype":"compact_boundary"}"#.to_owned(),
    ];
    let mut parent_uuid = "fixture-compact".to_owned();
    for index in 0..101 {
        let uuid = format!("fixture-message-{index}");
        records.push(format!(
            r#"{{"type":"user","uuid":"{uuid}","parentUuid":"{parent_uuid}","timestamp":"2026-01-01T00:00:03Z","message":{{"content":"Fixture request {index}."}}}}"#
        ));
        parent_uuid = uuid;
    }
    let transcript = records.join("\n");
    let additions = empty_additions();
    let expected = parse_all(transcript.as_bytes(), additions);
    let actual = tail_window_parser(Cursor::new(transcript.as_bytes()), 100, &[])
        .unwrap()
        .unwrap()
        .render_with(fixed_time(), additions)
        .unwrap();
    assert_eq!(last_messages(actual, 100), last_messages(expected, 100));
}

#[test]
fn tail_window_preserves_an_initial_incomplete_jsonl_row_after_append() {
    let padding = serde_json::json!({
        "fixturePadding": "x".repeat(5 * 1024 * 1024),
    })
    .to_string();
    let mut records = vec![padding];
    let mut parent_uuid = None;
    for index in 0..120 {
        let uuid = format!("fixture-message-{index}");
        records.push(
            serde_json::json!({
                "type": "user",
                "uuid": uuid,
                "parentUuid": parent_uuid,
                "timestamp": "2026-01-01T00:00:01Z",
                "message": {"content": format!("Fixture request {index}.")},
            })
            .to_string(),
        );
        parent_uuid = Some(uuid);
    }
    let appended_line = serde_json::json!({
        "type": "user",
        "uuid": "fixture-appended-message",
        "parentUuid": parent_uuid,
        "timestamp": "2026-01-01T00:00:02Z",
        "message": {"content": "The fixture row completes after initialization."},
    })
    .to_string();
    let split_at = appended_line.len() / 2;
    let initial_transcript = format!("{}\n{}", records.join("\n"), &appended_line[..split_at]);
    let appended_suffix = format!("{}\n", &appended_line[split_at..]);
    let complete_transcript = format!("{initial_transcript}{appended_suffix}");
    let mut parser = tail_window_parser(Cursor::new(initial_transcript.as_bytes()), 100, &[])
        .unwrap()
        .unwrap();
    let append_result = parser
        .append_bytes(appended_suffix.as_bytes(), fixed_time())
        .unwrap();
    let actual = parser.render_with(fixed_time(), empty_additions()).unwrap();
    let mut expected = parse_all(complete_transcript.as_bytes(), empty_additions());
    expected.0 = expected.0.into_iter().skip(20).collect();

    assert_eq!(
        (append_result, actual),
        (IncrementalAppend::Applied, expected)
    );
}

#[test]
fn tail_window_uses_an_unterminated_record_to_select_the_active_branch() {
    let padding = serde_json::json!({
        "type": "system",
        "padding": "x".repeat(5 * 1024 * 1024),
    });
    let mut records = vec![
        serde_json::json!({
            "type": "user",
            "uuid": "fixture-root",
            "parentUuid": null,
            "timestamp": "2026-01-01T00:00:00Z",
            "message": {"content": "Fixture root."},
        }),
        padding,
    ];
    let mut parent_uuid = "fixture-root".to_owned();
    for index in 0..120 {
        let uuid = format!("fixture-old-branch-{index}");
        records.push(serde_json::json!({
            "type": "user",
            "uuid": uuid,
            "parentUuid": parent_uuid,
            "timestamp": format!("2026-01-01T00:00:{:02}Z", index % 60),
            "message": {"content": format!("Abandoned fixture message {index}.")},
        }));
        parent_uuid = uuid;
    }
    let transcript = records
        .iter()
        .map(serde_json::Value::to_string)
        .collect::<Vec<_>>()
        .join("\n");
    let pending_branch_switch = serde_json::json!({
        "type": "user",
        "uuid": "fixture-current-branch",
        "parentUuid": "fixture-root",
        "timestamp": "2026-01-01T00:01:00Z",
        "message": {"content": "Current fixture branch."},
    })
    .to_string();
    let initial_transcript = format!("{transcript}\n{pending_branch_switch}");
    let additions = empty_additions();
    let actual = tail_window_parser(Cursor::new(initial_transcript.as_bytes()), 100, &[])
        .unwrap()
        .unwrap()
        .render_with(fixed_time(), additions)
        .unwrap();

    assert_eq!(
        actual,
        (
            vec![ChatItem::User {
                text: "Current fixture branch.".to_owned(),
                rewind_id: Some("fixture-current-branch".to_owned()),
                queued_prompt_id: None,
                timestamp: Some("2026-01-01T00:01:00+00:00".to_owned()),
                from: None,
            }],
            Vec::new(),
            Vec::new(),
            Vec::new(),
        ),
    );
}

#[test]
fn tail_window_excludes_an_abandoned_branch_whose_parent_is_outside_the_window() {
    let padding = "x".repeat(5 * 1024 * 1024);
    let transcript = [
        serde_json::json!({
            "type": "user",
            "uuid": "fixture-root",
            "parentUuid": null,
            "timestamp": "2026-01-01T00:00:01Z",
            "message": {"content": "Fixture root."},
        }),
        serde_json::json!({
            "type": "assistant",
            "uuid": "fixture-abandoned-parent",
            "parentUuid": "fixture-root",
            "timestamp": "2026-01-01T00:00:02Z",
            "message": {"content": [{"type": "text", "text": "Abandoned response."}]},
        }),
        serde_json::json!({
            "type": "system",
            "uuid": "fixture-padding",
            "parentUuid": "fixture-root",
            "padding": padding,
        }),
        serde_json::json!({
            "type": "user",
            "uuid": "fixture-abandoned-request",
            "parentUuid": "fixture-abandoned-parent",
            "timestamp": "2026-01-01T00:00:03Z",
            "message": {"content": "Abandoned branch request."},
        }),
        serde_json::json!({
            "type": "user",
            "uuid": "fixture-replacement-request",
            "parentUuid": "fixture-root",
            "timestamp": "2026-01-01T00:00:04Z",
            "message": {"content": "Replacement request."},
        }),
        serde_json::json!({
            "type": "assistant",
            "uuid": "fixture-replacement-response",
            "parentUuid": "fixture-replacement-request",
            "timestamp": "2026-01-01T00:00:05Z",
            "message": {"content": [{"type": "text", "text": "Replacement response."}]},
        }),
        serde_json::json!({
            "type": "user",
            "uuid": "fixture-latest-request",
            "parentUuid": "fixture-replacement-response",
            "timestamp": "2026-01-01T00:00:06Z",
            "message": {"content": "Latest request."},
        }),
    ]
    .into_iter()
    .map(|record| record.to_string())
    .collect::<Vec<_>>()
    .join("\n");
    let additions = empty_additions();
    let mut expected = parse_all(transcript.as_bytes(), additions);
    expected.0 = expected.0.into_iter().skip(1).collect();
    let actual = tail_window_parser(Cursor::new(transcript.as_bytes()), 3, &[])
        .unwrap()
        .unwrap()
        .render_with(fixed_time(), additions)
        .unwrap();

    assert_eq!(actual, expected);
}

#[rstest]
fn tail_window_matches_the_full_parse_when_the_prefix_exceeds_the_search_limit(
    prefix_beyond_search_limit: PrefixBeyondSearchLimit,
) {
    let additions = empty_additions();
    let reference_transcript = prefix_beyond_search_limit.reference_transcript();
    let expected = parse_all(&reference_transcript, additions);
    let tail_parser = tail_window_parser(prefix_beyond_search_limit.reader(), 100, &[])
        .unwrap()
        .unwrap();
    let prefix_length = tail_parser.tail_window_start_offset().unwrap();
    let actual = tail_parser.render_with(fixed_time(), additions).unwrap();
    assert_eq!(
        (
            prefix_length > MAX_PREFIX_SEARCH_BYTES,
            last_messages(actual, 100)
        ),
        (true, last_messages(expected, 100)),
    );
}

#[rstest]
fn queued_prompt_resolution_scans_a_prefix_over_the_search_limit(
    prefix_beyond_search_limit: PrefixBeyondSearchLimit,
) {
    let user_marker = "⟦armyknife-user:fixture-marker-cyan⟧";
    let tail_parser = tail_window_parser(prefix_beyond_search_limit.reader(), 100, &[])
        .unwrap()
        .unwrap();
    let prefix_length = tail_parser.tail_window_start_offset().unwrap();
    let queued_prompt = saved_queued_prompt();
    let mut reader = prefix_beyond_search_limit.reader();
    let resolved = tail_parser
        .resolve_queued_prompt_ids_before_window(
            &mut reader,
            prefix_length,
            &[queued_prompt],
            Some(user_marker),
        )
        .unwrap();

    assert_eq!(
        (prefix_length > MAX_PREFIX_SEARCH_BYTES, resolved),
        (true, Some(vec![7])),
    );
}

#[test]
fn tail_window_matches_the_full_parse_when_an_attachment_follows_tool_results() {
    let transcript = indoc! {r#"
        {"type":"user","uuid":"fixture-earlier-request","parentUuid":null,"timestamp":"2026-01-01T00:00:00Z","message":{"content":"Earlier fixture request."}}
        {"type":"user","uuid":"fixture-root","parentUuid":"fixture-earlier-request","timestamp":"2026-01-01T00:00:01Z","message":{"content":"Run two fixture tools."}}
        {"type":"assistant","uuid":"fixture-tool-call","parentUuid":"fixture-root","timestamp":"2026-01-01T00:00:02Z","message":{"content":[{"type":"tool_use","id":"fixture-call-a","name":"Read","input":{"file_path":"/fixture/a"}},{"type":"tool_use","id":"fixture-call-b","name":"Read","input":{"file_path":"/fixture/b"}}]}}
        {"type":"user","uuid":"fixture-result-a","parentUuid":"fixture-tool-call","timestamp":"2026-01-01T00:00:03Z","message":{"content":[{"type":"tool_result","tool_use_id":"fixture-call-a","content":"First fixture result."}]}}
        {"type":"attachment","uuid":"fixture-hook","parentUuid":"fixture-result-a","timestamp":"2026-01-01T00:00:04Z","attachment":{"type":"hook_success"}}
        {"type":"user","uuid":"fixture-result-b","parentUuid":"fixture-tool-call","timestamp":"2026-01-01T00:00:05Z","message":{"content":[{"type":"tool_result","tool_use_id":"fixture-call-b","content":"Second fixture result."}]}}
        {"type":"assistant","uuid":"fixture-tool-answer","parentUuid":"fixture-result-b","timestamp":"2026-01-01T00:00:06Z","message":{"content":[{"type":"text","text":"The fixture tools are complete."}]}}
        {"type":"user","uuid":"fixture-latest-request","parentUuid":"fixture-tool-answer","timestamp":"2026-01-01T00:00:07Z","message":{"content":"Latest fixture request."}}
        {"type":"attachment","uuid":"fixture-final-hook","parentUuid":"fixture-latest-request","timestamp":"2026-01-01T00:00:08Z","attachment":{"type":"hook_success"}}
    "#};
    let additions = empty_additions();
    let expected = parse_all(transcript.as_bytes(), additions);
    let actual = tail_window_parser(Cursor::new(transcript.as_bytes()), 2, &[])
        .unwrap()
        .unwrap()
        .render_with(fixed_time(), additions)
        .unwrap();
    assert_eq!(last_messages(actual, 2), last_messages(expected, 2));
}

#[test]
fn tail_window_restores_an_older_pending_task_start() {
    let prefix = indoc! {r#"
        {"type":"assistant","uuid":"fixture-task-call","parentUuid":null,"timestamp":"2026-01-01T00:00:01Z","message":{"content":[{"type":"tool_use","id":"fixture-task-use","name":"Bash","input":{"command":"fixture background task","run_in_background":true}}]}}
        {"type":"user","uuid":"fixture-task-started","parentUuid":"fixture-task-call","timestamp":"2026-01-01T00:00:02Z","toolUseResult":{"task_id":"fixture-task-id"},"message":{"content":[{"type":"tool_result","tool_use_id":"fixture-task-use","content":"Background task started (id: fixture-task-id)"}]}}
    "#};
    let padded_record = format!(
        r#"{{"type":"system","uuid":"fixture-padding","parentUuid":null,"timestamp":"2026-01-01T00:00:02Z","padding":"{}"}}"#,
        "x".repeat(70 * 1024),
    );
    let suffix = indoc! {r#"
        {"type":"user","uuid":"fixture-first-request","parentUuid":"fixture-task-started","timestamp":"2026-01-01T00:00:03Z","message":{"content":"First request."}}
        {"type":"user","uuid":"fixture-latest-request","parentUuid":"fixture-first-request","timestamp":"2026-01-01T00:00:04Z","message":{"content":"Latest request."}}
    "#};
    let transcript = [prefix, &padded_record, suffix].join("\n");
    let additions = empty_additions();
    let expected = parse_all(transcript.as_bytes(), additions);
    let actual = tail_window_parser(
        Cursor::new(transcript.as_bytes()),
        1,
        &["fixture-task-id".to_owned()],
    )
    .unwrap()
    .unwrap()
    .render_with(fixed_time(), additions)
    .unwrap();
    assert_eq!(
        last_message_and_state(actual),
        last_message_and_state(expected)
    );
}

#[test]
fn tail_window_restores_a_running_task_behind_a_compact_boundary() {
    let padding = "x".repeat(5 * 1024 * 1024);
    let records = [
        serde_json::json!({
            "type": "user",
            "uuid": "fixture-root",
            "parentUuid": null,
            "timestamp": "2026-01-01T00:00:01Z",
            "message": {"content": "Fixture root."},
        }),
        serde_json::json!({
            "type": "assistant",
            "uuid": "fixture-task-call",
            "parentUuid": "fixture-root",
            "timestamp": "2026-01-01T00:00:02Z",
            "message": {"content": [{
                "type": "tool_use",
                "id": "fixture-task-use",
                "name": "Bash",
                "input": {"command": "fixture background task", "run_in_background": true},
            }]},
        }),
        serde_json::json!({
            "type": "user",
            "uuid": "fixture-task-started",
            "parentUuid": "fixture-task-call",
            "timestamp": "2026-01-01T00:00:03Z",
            "toolUseResult": {"task_id": "fixture-task-id"},
            "message": {"content": [{
                "type": "tool_result",
                "tool_use_id": "fixture-task-use",
                "content": "Background task started (id: fixture-task-id)",
            }]},
        }),
        serde_json::json!({
            "type": "system",
            "uuid": "fixture-compact-boundary",
            "parentUuid": null,
            "logicalParentUuid": "fixture-task-started",
            "timestamp": "2026-01-01T00:00:04Z",
            "subtype": "compact_boundary",
        }),
        serde_json::json!({
            "type": "system",
            "uuid": "fixture-padding",
            "parentUuid": "fixture-compact-boundary",
            "padding": padding,
        }),
        serde_json::json!({
            "type": "user",
            "uuid": "fixture-latest-request",
            "parentUuid": "fixture-compact-boundary",
            "timestamp": "2026-01-01T00:00:05Z",
            "message": {"content": "Latest fixture request."},
        }),
    ]
    .into_iter()
    .map(|record| record.to_string())
    .collect::<Vec<_>>()
    .join("\n");
    let additions = empty_additions();
    let expected = parse_all(records.as_bytes(), additions);
    let actual = tail_window_parser(
        Cursor::new(records.as_bytes()),
        1,
        &["fixture-task-id".to_owned()],
    )
    .unwrap()
    .unwrap()
    .render_with(fixed_time(), additions)
    .unwrap();

    assert_eq!(
        last_message_and_state(actual),
        last_message_and_state(expected),
    );
}

#[test]
fn tail_window_restores_the_initial_skills_before_an_in_window_addition() {
    let transcript = indoc! {r#"
        {"type":"attachment","attachment":{"type":"skill_listing","isInitial":true,"content":"- fixture-existing-skill: Existing skill description."}}
        {"type":"user","uuid":"fixture-first-request","parentUuid":null,"timestamp":"2026-01-01T00:00:01Z","message":{"content":"First request."}}
        {"type":"attachment","attachment":{"type":"skill_listing","isInitial":false,"content":"- fixture-added-skill: Added skill description."}}
        {"type":"user","uuid":"fixture-latest-request","parentUuid":"fixture-first-request","timestamp":"2026-01-01T00:00:02Z","message":{"content":"Latest request."}}
    "#};
    let additions = empty_additions();
    let expected = parse_all(transcript.as_bytes(), additions);
    let actual = tail_window_parser(Cursor::new(transcript.as_bytes()), 1, &[])
        .unwrap()
        .unwrap()
        .render_with(fixed_time(), additions)
        .unwrap();
    assert_eq!(
        last_message_and_state(actual),
        last_message_and_state(expected)
    );
}

#[test]
fn pending_json_completed_after_a_second_append_matches_a_full_parse() {
    let prefix = indoc! {r#"
        {"type":"user","uuid":"message-one","parentUuid":null,"timestamp":"2026-01-01T00:00:01Z","message":{"content":"A question."}}
    "#};
    let first_append = r#"{"type":"assistant","uuid":"message-two","parentUuid":"message-one","timestamp":"2026-01-01T00:00:02Z","message":{"content":[{"type":"text","text":"An answer"#;
    let second_append = r#"."}]}}"#;
    let mut transcript = prefix.as_bytes().to_vec();
    transcript.extend_from_slice(first_append.as_bytes());
    transcript.extend_from_slice(second_append.as_bytes());

    let mut parser = incremental_parser(prefix.as_bytes());
    let first_result = parser
        .append_bytes(first_append.as_bytes(), fixed_time())
        .unwrap();
    let second_result = parser
        .append_bytes(second_append.as_bytes(), fixed_time())
        .unwrap();
    let actual = parser.render_with(fixed_time(), empty_additions()).unwrap();

    assert_eq!(
        (first_result, second_result, actual),
        (
            IncrementalAppend::Applied,
            IncrementalAppend::Applied,
            parse_all(&transcript, empty_additions()),
        ),
    );
}

#[rstest]
fn tail_window_expansion_handles_files_smaller_than_the_next_window(oversized_transcript: String) {
    let actual = tail_window_parser(Cursor::new(oversized_transcript.as_bytes()), 2, &[]);

    assert_eq!(
        actual
            .map(|parser| parser.is_none())
            .map_err(|error| error.kind()),
        Ok(true),
    );
}

#[rstest]
fn initial_parse_falls_back_when_window_expansion_reaches_file_start(oversized_transcript: String) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("fixture.jsonl");
    std::fs::write(&path, oversized_transcript).unwrap();
    let input = || TranscriptInput {
        path: Some(&path),
        engine: Engine::Claude,
        additions: ParserAdditions {
            session_id: "fixture-session",
            parent: None,
            pending_tool: None,
            user_marker: None,
            queued_prompts: &[],
            peer_out_messages: &[],
            proposals: &[],
            todos: &[],
            additional_cards: &[],
        },
    };
    let expected = parse_session_transcript(input()).unwrap();
    let actual = parse_initial_or_full(
        "fixture-session",
        input(),
        Some(1),
        &[],
        None,
        &MessageCache::default(),
    )
    .unwrap();

    assert_eq!(actual, (expected.0, expected.1, false));
}

#[rstest]
fn appended_peer_delivery_resolves_queued_prompt_after_branch_reparse(
    saved_queued_prompt: WatchWebQueuedPrompt,
) {
    let suffix = indoc! {r#"
        {"type":"assistant","uuid":"fixture-delivery-child","parentUuid":"fixture-delivery","message":{"content":[{"type":"text","text":"A child record arrived first."}]}}
        {"type":"user","uuid":"fixture-delivery","parentUuid":"fixture-root","timestamp":"2026-01-01T00:00:03Z","isMeta":true,"origin":{"kind":"peer","from":"unknown"},"message":{"content":"⟦armyknife-user:fixture-marker-cobalt⟧\nThis message was typed by the human user in armyknife watch-web.\nEngine-formatted delivery.\n<!-- armyknife-queued-prompt-id:fixture-message-cyan -->"}}
        {"type":"assistant","uuid":"fixture-final","parentUuid":"fixture-current","message":{"content":[{"type":"text","text":"Final response."}]}}
    "#};
    let mut transcript = ROOT_AND_CURRENT.as_bytes().to_vec();
    transcript.extend_from_slice(suffix.as_bytes());
    let mut parser = incremental_parser(ROOT_AND_CURRENT.as_bytes());
    let append = parser
        .append_bytes(suffix.as_bytes(), fixed_time())
        .unwrap();
    let parser = match append {
        IncrementalAppend::Applied => parser,
        IncrementalAppend::RestartRequired => incremental_parser(&transcript),
    };
    let actual = (
        append,
        parser
            .render_with(fixed_time(), queued_prompt_additions(&saved_queued_prompt))
            .unwrap(),
    );

    assert_eq!(
        actual,
        (
            IncrementalAppend::RestartRequired,
            (
                vec![
                    ChatItem::User {
                        rewind_id: Some("fixture-root".to_owned()),
                        text: "Initial request.".to_owned(),
                        timestamp: None,
                        from: None,
                        queued_prompt_id: None,
                    },
                    ChatItem::Assistant {
                        text: "Current response.".to_owned(),
                    },
                    ChatItem::User {
                        rewind_id: Some("fixture-delivery".to_owned()),
                        text: "Engine-formatted delivery.\n<!-- armyknife-queued-prompt-id:fixture-message-cyan -->".to_owned(),
                        timestamp: Some("2026-01-01T00:00:03+00:00".to_owned()),
                        from: None,
                        queued_prompt_id: None,
                    },
                    ChatItem::Assistant {
                        text: "Final response.".to_owned(),
                    },
                ],
                Vec::new(),
                vec![7],
                Vec::new(),
            ),
        ),
    );
}

#[rstest]
fn inactive_peer_delivery_does_not_resolve_queued_prompt(
    saved_queued_prompt: WatchWebQueuedPrompt,
) {
    let suffix = indoc! {r#"
        {"type":"user","uuid":"fixture-delivery","parentUuid":"fixture-root","timestamp":"2026-01-01T00:00:03Z","isMeta":true,"origin":{"kind":"peer","from":"unknown"},"message":{"content":"⟦armyknife-user:fixture-marker-cobalt⟧\nThis message was typed by the human user in armyknife watch-web.\nEngine-formatted delivery.\n<!-- armyknife-queued-prompt-id:fixture-message-cyan -->"}}
        {"type":"assistant","uuid":"fixture-final","parentUuid":"fixture-current","message":{"content":[{"type":"text","text":"Final response."}]}}
    "#};
    let mut transcript = ROOT_AND_CURRENT.as_bytes().to_vec();
    transcript.extend_from_slice(suffix.as_bytes());
    let mut parser = incremental_parser(ROOT_AND_CURRENT.as_bytes());
    let append = parser
        .append_bytes(suffix.as_bytes(), fixed_time())
        .unwrap();
    let parser = match append {
        IncrementalAppend::Applied => parser,
        IncrementalAppend::RestartRequired => incremental_parser(&transcript),
    };
    let actual = (
        append,
        parser
            .render_with(fixed_time(), queued_prompt_additions(&saved_queued_prompt))
            .unwrap(),
    );

    assert_eq!(
        actual,
        (
            IncrementalAppend::RestartRequired,
            (
                vec![
                    ChatItem::User {
                        rewind_id: Some("fixture-root".to_owned()),
                        text: "Initial request.".to_owned(),
                        timestamp: None,
                        from: None,
                        queued_prompt_id: None,
                    },
                    ChatItem::Assistant {
                        text: "Current response.".to_owned(),
                    },
                    ChatItem::Assistant {
                        text: "Final response.".to_owned(),
                    },
                    ChatItem::QueuedUser {
                        content: vec![super::super::super::super::UserContentPart::Text {
                            text: "Delivered request.".to_owned(),
                        }],
                        queued_prompt_id: Some("fixture-message-cyan".to_owned()),
                        from: None,
                        timestamp: Some("2026-01-01T00:00:00+00:00".to_owned()),
                    },
                ],
                Vec::new(),
                Vec::new(),
                Vec::new(),
            ),
        ),
    );
}

#[rstest]
fn rewound_peer_delivery_with_an_in_order_child_does_not_resolve_queued_prompt(
    saved_queued_prompt: WatchWebQueuedPrompt,
) {
    let transcript = indoc! {r#"
        {"type":"user","uuid":"fixture-root","parentUuid":null,"message":{"content":"Initial request."}}
        {"type":"assistant","uuid":"fixture-shared","parentUuid":"fixture-root","message":{"content":[{"type":"text","text":"Shared response."}]}}
        {"type":"user","uuid":"fixture-old-delivery","parentUuid":"fixture-shared","isMeta":true,"origin":{"kind":"peer","from":"unknown"},"message":{"content":"⟦armyknife-user:fixture-marker-cobalt⟧\nThis message was typed by the human user in armyknife watch-web.\nEngine-formatted delivery.\n<!-- armyknife-queued-prompt-id:fixture-message-cyan -->"}}
        {"type":"assistant","uuid":"fixture-old-child","parentUuid":"fixture-old-delivery","message":{"content":[{"type":"text","text":"Discard this response."}]}}
        {"type":"user","uuid":"fixture-replacement","parentUuid":"fixture-shared","message":{"content":"Replacement request."}}
        {"type":"assistant","uuid":"fixture-current","parentUuid":"fixture-replacement","message":{"content":[{"type":"text","text":"Keep this response."}]}}
    "#};

    assert_eq!(
        parse_all(
            transcript.as_bytes(),
            queued_prompt_additions(&saved_queued_prompt)
        ),
        (
            vec![
                ChatItem::User {
                    rewind_id: Some("fixture-root".to_owned()),
                    text: "Initial request.".to_owned(),
                    timestamp: None,
                    from: None,
                    queued_prompt_id: None,
                },
                ChatItem::Assistant {
                    text: "Shared response.".to_owned(),
                },
                ChatItem::User {
                    rewind_id: Some("fixture-replacement".to_owned()),
                    text: "Replacement request.".to_owned(),
                    timestamp: None,
                    from: None,
                    queued_prompt_id: None,
                },
                ChatItem::Assistant {
                    text: "Keep this response.".to_owned(),
                },
                ChatItem::QueuedUser {
                    content: vec![super::super::super::super::UserContentPart::Text {
                        text: "Delivered request.".to_owned(),
                    }],
                    queued_prompt_id: Some("fixture-message-cyan".to_owned()),
                    from: None,
                    timestamp: Some("2026-01-01T00:00:00+00:00".to_owned()),
                },
            ],
            Vec::new(),
            Vec::new(),
            Vec::new(),
        ),
    );
}

#[test]
fn todo_overlay_changes_render_the_cached_transcript_without_reingestion() {
    let prefix = indoc! {r#"
        {"type":"user","uuid":"message-one","parentUuid":null,"timestamp":"2026-01-01T00:00:01Z","message":{"content":"A question."}}
    "#};
    let suffix = indoc! {r#"
        {"type":"assistant","uuid":"message-two","parentUuid":"message-one","timestamp":"2026-01-01T00:00:02Z","message":{"content":[{"type":"text","text":"An answer."}]}}
    "#};
    let todo = AgentTodo {
        session_id: "fixture-session".to_owned(),
        seq: 1,
        title: "Review the sample report".to_owned(),
        detail: None,
        folded: false,
        added_folded: false,
        created_at: fixed_time(),
        completed_at: None,
        resolution: None,
        comment: None,
    };
    let mut parser = incremental_parser(prefix.as_bytes());
    match parser
        .append_bytes(suffix.as_bytes(), fixed_time())
        .unwrap()
    {
        IncrementalAppend::Applied => {}
        IncrementalAppend::RestartRequired => {
            panic!("the fixture continues the active transcript branch")
        }
    }
    let additions = ParserAdditions {
        todos: std::slice::from_ref(&todo),
        ..empty_additions()
    };
    let actual = parser.render_with(fixed_time(), additions).unwrap();
    let mut transcript = prefix.as_bytes().to_vec();
    transcript.extend_from_slice(suffix.as_bytes());
    assert_eq!(actual, parse_all(&transcript, additions));
}
