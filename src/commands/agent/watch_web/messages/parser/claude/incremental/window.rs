use std::collections::{BTreeMap, HashSet};
use std::io::{self, BufRead, Read, Seek, SeekFrom};
use std::path::Path;
use std::time::Instant;

use serde_json::Value;

use super::super::super::record_timestamp;
use super::super::state::record_parent_uuid;
use super::tail_window;
use super::{IncrementalClaudeParser, parse_record};
use crate::commands::agent::watch_web::messages::log_tail_window_fallback;

const INITIAL_WINDOW_BYTES: u64 = 4 * 1024 * 1024;
const MAX_WINDOW_BYTES: u64 = 64 * 1024 * 1024;
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
    let selected_uuids = selected_records
        .iter()
        .filter_map(|record| {
            record
                .get("uuid")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .collect::<HashSet<_>>();
    let active_ancestor_uuid = active_ancestor_uuid(&selected_records);
    if missing_task_ids.len() > 4096 {
        log_tail_window_fallback(started, "pending_task_search_limit_exceeded");
        return Ok(None);
    }

    let (mut helper_records, active_ancestor_uuids, complete) = tail_window::find_prefix_helpers(
        &mut reader,
        window_start,
        &missing_task_ids,
        needs_skill_baseline,
        &selected_uuids,
        active_ancestor_uuid,
    )?;
    if !complete {
        log_tail_window_fallback(started, "prefix_parser_state_unavailable");
        return Ok(None);
    }
    helper_records.extend(selected_records.iter().cloned());
    let tail_window_start = selected_records
        .iter()
        .find(|record| tail_window::is_human_input_record(record))
        .and_then(record_timestamp);
    let mut parser = IncrementalClaudeParser::from_tail_window_records(
        helper_records,
        active_ancestor_uuids,
        image_directory,
    );
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

fn active_ancestor_uuid(records: &[Value]) -> Option<String> {
    let parents = records
        .iter()
        .filter_map(|record| {
            let uuid = record.get("uuid").and_then(Value::as_str)?;
            Some((uuid.to_owned(), record_parent_uuid(record)))
        })
        .collect::<BTreeMap<_, _>>();
    let mut current_uuid = records
        .iter()
        .rev()
        .find_map(|record| record.get("uuid").and_then(Value::as_str))?
        .to_owned();
    let mut visited = HashSet::new();
    while visited.insert(current_uuid.clone()) {
        let parent_uuid = parents.get(&current_uuid).and_then(Clone::clone)?;
        if !parents.contains_key(&parent_uuid) {
            return Some(parent_uuid);
        }
        current_uuid = parent_uuid;
    }
    None
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
            if !has_user_record_type(line) {
                continue;
            }
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

fn has_user_record_type(line: &[u8]) -> bool {
    let mut search_start = 0;
    while let Some(relative_type_key) = memchr::memmem::find(&line[search_start..], b"\"type\"") {
        let type_key = search_start + relative_type_key;
        let suffix = line[type_key + b"\"type\"".len()..].trim_ascii_start();
        if suffix
            .strip_prefix(b":")
            .is_some_and(|value| value.trim_ascii_start().starts_with(b"\"user\""))
        {
            return true;
        }
        search_start = type_key + 1;
    }
    false
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
