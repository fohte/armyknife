use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::io::{self, BufRead, Read, Seek, SeekFrom};
use std::path::Path;
use std::time::Instant;

use chrono::{DateTime, Utc};
use serde_json::Value;

use super::super::super::record_timestamp;
use super::super::state::record_parent_uuid;
use super::tail_window;
use super::{IncrementalClaudeParser, parse_record};
use crate::commands::agent::watch_web::messages::log_tail_window_fallback;

const INITIAL_WINDOW_BYTES: u64 = 4 * 1024 * 1024;
const MAX_WINDOW_BYTES: u64 = 64 * 1024 * 1024;
const MAX_PREFIX_SEARCH_BYTES: u64 = 256 * 1024 * 1024;
const MAX_ANCESTRY_SEARCHES: usize = 4096;
type TimedRecord = (Option<DateTime<Utc>>, Value);

pub(super) fn new_tail_window(
    reader: impl BufRead + Seek,
    image_directory: Option<&Path>,
    message_limit: usize,
    pending_task_ids: &[String],
) -> io::Result<Option<IncrementalClaudeParser>> {
    new_tail_window_bounded(
        reader,
        image_directory,
        message_limit,
        pending_task_ids,
        MAX_WINDOW_BYTES,
    )
}

pub(super) fn new_tail_window_bounded(
    mut reader: impl BufRead + Seek,
    image_directory: Option<&Path>,
    message_limit: usize,
    pending_task_ids: &[String],
    max_window_bytes: u64,
) -> io::Result<Option<IncrementalClaudeParser>> {
    let started = Instant::now();
    let file_length = reader.seek(SeekFrom::End(0))?;
    let Some(window_start) =
        select_window_start(&mut reader, file_length, message_limit, max_window_bytes)?
    else {
        log_tail_window_fallback(started, "message_window_not_found_within_limit");
        return Ok(None);
    };
    if window_start == 0 {
        log_tail_window_fallback(started, "window_includes_transcript_start");
        return Ok(None);
    }

    let window_bytes = read_selected_window(&mut reader, file_length, window_start)?;
    let selected_records = window_bytes
        .split(|byte| *byte == b'\n')
        .filter_map(parse_record)
        .collect::<Vec<_>>();
    let mut graph_records = selected_records
        .iter()
        .filter_map(|record| {
            let uuid = record.get("uuid").and_then(Value::as_str)?;
            Some((uuid.to_owned(), record_parent_uuid(record)))
        })
        .collect::<BTreeMap<_, _>>();
    let mut unresolved = graph_records
        .values()
        .filter_map(Clone::clone)
        .filter(|uuid| !graph_records.contains_key(uuid))
        .collect::<BTreeSet<_>>();
    let missing_task_ids = pending_task_ids
        .iter()
        .filter(|task_id| {
            !selected_records
                .iter()
                .any(|record| tail_window::record_has_task_id(record, task_id))
        })
        .cloned()
        .collect::<Vec<_>>();
    let needs_skill_baseline = !selected_records
        .iter()
        .any(tail_window::resets_skill_listing_state);
    if window_start > MAX_PREFIX_SEARCH_BYTES
        && (!unresolved.is_empty()
            || !missing_task_ids.is_empty()
            || (needs_skill_baseline && window_start > 0))
    {
        log_tail_window_fallback(started, "prefix_search_limit_exceeded");
        return Ok(None);
    }
    if missing_task_ids.len() > MAX_ANCESTRY_SEARCHES {
        log_tail_window_fallback(started, "pending_task_search_limit_exceeded");
        return Ok(None);
    }

    let mut searches = 0;
    if !unresolved.is_empty()
        && !tail_window::resolve_ancestry_from_prefix(
            &mut reader,
            window_start,
            &mut unresolved,
            &mut graph_records,
            MAX_ANCESTRY_SEARCHES,
            &mut searches,
        )?
    {
        log_tail_window_fallback(started, "transcript_ancestry_unavailable");
        return Ok(None);
    }

    let Some(helper_records) = restore_prefix_state(
        &mut reader,
        window_start,
        needs_skill_baseline,
        &missing_task_ids,
        &mut graph_records,
    )?
    else {
        log_tail_window_fallback(started, "prefix_parser_state_unavailable");
        return Ok(None);
    };
    let selected_bytes = assemble_parser_input(&selected_records, &helper_records, &graph_records)?;
    let tail_window_start = selected_records
        .iter()
        .find(|record| tail_window::is_human_input_record(record))
        .and_then(record_timestamp);
    let mut parser =
        IncrementalClaudeParser::new(std::io::Cursor::new(selected_bytes), image_directory)?;
    parser.tail_window_start = tail_window_start;
    parser.tail_window_start_offset = Some(window_start);
    if let Some(cutoff) = tail_window_start {
        parser
            .state
            .events
            .retain(|event| event.timestamp.is_none_or(|timestamp| timestamp >= cutoff));
    }
    Ok(Some(parser))
}

fn select_window_start(
    reader: &mut (impl Read + Seek),
    file_length: u64,
    message_limit: usize,
    max_window_bytes: u64,
) -> io::Result<Option<u64>> {
    let mut window_length = INITIAL_WINDOW_BYTES.min(file_length).min(max_window_bytes);
    loop {
        let start = file_length.saturating_sub(window_length);
        reader.seek(SeekFrom::Start(start))?;
        let window_size = usize::try_from(window_length)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))?;
        let mut bytes = vec![0; window_size];
        reader.read_exact(&mut bytes)?;
        let aligned_start = if start == 0 {
            0
        } else {
            bytes
                .iter()
                .position(|byte| *byte == b'\n')
                .map_or(bytes.len(), |index| index + 1)
        };
        let mut row_offset = 0;
        let mut human_input_offsets = Vec::new();
        for line in bytes[aligned_start..].split(|byte| *byte == b'\n') {
            let offset = row_offset;
            row_offset += line.len() + 1;
            if let Ok(record) = serde_json::from_slice::<tail_window::GraphRecord>(line)
                && tail_window::is_human_input_line(&record, line)
            {
                human_input_offsets.push(offset);
            }
        }
        if message_limit > 0 && human_input_offsets.len() >= message_limit {
            let selected_offset = human_input_offsets[human_input_offsets.len() - message_limit];
            return Ok(Some(start + aligned_start as u64 + selected_offset as u64));
        }
        if start == 0 {
            return Ok(Some(0));
        }
        if window_length == max_window_bytes {
            return Ok(None);
        }
        window_length = window_length
            .saturating_mul(2)
            .min(max_window_bytes)
            .min(file_length);
    }
}

fn read_selected_window(
    reader: &mut (impl Read + Seek),
    file_length: u64,
    window_start: u64,
) -> io::Result<Vec<u8>> {
    let window_size = usize::try_from(file_length - window_start)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))?;
    let mut bytes = vec![0; window_size];
    reader.seek(SeekFrom::Start(window_start))?;
    reader.read_exact(&mut bytes)?;
    Ok(bytes)
}

fn restore_prefix_state(
    reader: &mut (impl Read + Seek),
    window_start: u64,
    needs_skill_baseline: bool,
    missing_task_ids: &[String],
    graph_records: &mut BTreeMap<String, Option<String>>,
) -> io::Result<Option<Vec<TimedRecord>>> {
    let mut helper_records = Vec::new();
    if needs_skill_baseline {
        let (skill_listings, complete) =
            tail_window::find_skill_listing_prefix_state(reader, window_start)?;
        if !complete {
            return Ok(None);
        }
        helper_records.extend(
            skill_listings
                .into_iter()
                .map(|record| (record_timestamp(&record), record)),
        );
    }
    if !missing_task_ids.is_empty() {
        let (task_results, complete) = tail_window::find_pending_background_task_starts(
            reader,
            window_start,
            missing_task_ids,
        )?;
        if !complete {
            return Ok(None);
        }
        let parent_uuids = task_results
            .values()
            .filter_map(record_parent_uuid)
            .collect::<HashSet<_>>();
        let (tool_uses, complete) =
            tail_window::find_records_by_uuid(reader, window_start, &parent_uuids)?;
        if !complete {
            return Ok(None);
        }
        for tool_use in tool_uses.into_values() {
            if let Some(uuid) = tool_use.get("uuid").and_then(Value::as_str) {
                graph_records.insert(uuid.to_owned(), record_parent_uuid(&tool_use));
            }
            helper_records.push((record_timestamp(&tool_use), tool_use));
        }
        for result in task_results.into_values() {
            if let Some(uuid) = result.get("uuid").and_then(Value::as_str) {
                graph_records.insert(uuid.to_owned(), record_parent_uuid(&result));
            }
            helper_records.push((record_timestamp(&result), result));
        }
    }
    helper_records.sort_by_key(|(timestamp, _)| *timestamp);
    Ok(Some(helper_records))
}

fn assemble_parser_input(
    selected_records: &[Value],
    helper_records: &[TimedRecord],
    graph_records: &BTreeMap<String, Option<String>>,
) -> io::Result<Vec<u8>> {
    let mut selected_bytes = Vec::new();
    for (uuid, parent_uuid) in graph_records.iter().filter(|(uuid, _)| {
        !selected_records
            .iter()
            .any(|record| record.get("uuid").and_then(Value::as_str) == Some(uuid.as_str()))
    }) {
        let mut record = serde_json::Map::new();
        record.insert("uuid".to_owned(), Value::String(uuid.clone()));
        if let Some(parent_uuid) = parent_uuid {
            record.insert("parentUuid".to_owned(), Value::String(parent_uuid.clone()));
        } else {
            record.insert("parentUuid".to_owned(), Value::Null);
        }
        append_json_line(&mut selected_bytes, &record)?;
    }
    for (_, record) in helper_records {
        append_json_line(&mut selected_bytes, record)?;
    }
    for record in selected_records {
        append_json_line(&mut selected_bytes, record)?;
    }
    Ok(selected_bytes)
}

fn append_json_line(bytes: &mut Vec<u8>, record: &impl serde::Serialize) -> io::Result<()> {
    let encoded = serde_json::to_vec(record)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))?;
    bytes.extend_from_slice(&encoded);
    bytes.push(b'\n');
    Ok(())
}
