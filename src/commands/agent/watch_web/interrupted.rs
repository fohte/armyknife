use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::Path;

use serde_json::Value;

use crate::commands::agent::types::{Engine, Session, SessionStatus};

const MAX_TAIL_SIZE: usize = 64 * 1024;
const INTERRUPT_PREFIX: &str = "[Request interrupted by user";

pub(super) fn is_waiting_after_user_interrupt(session: &Session) -> bool {
    if session.engine != Engine::Claude || session.status != SessionStatus::Running {
        return false;
    }

    session
        .transcript_path
        .as_deref()
        .is_some_and(latest_relevant_record_is_interrupt)
}

fn latest_relevant_record_is_interrupt(path: &Path) -> bool {
    let Some(file) = File::open(path).ok() else {
        return false;
    };
    let Some(file_size) = file.metadata().ok().map(|metadata| metadata.len()) else {
        return false;
    };
    if file_size == 0 {
        return false;
    }

    let read_size = file_size.min(MAX_TAIL_SIZE as u64) as usize;
    let mut reader = BufReader::new(file);
    if reader.seek(SeekFrom::End(-(read_size as i64))).is_err() {
        return false;
    }
    let mut buffer = vec![0; read_size];
    if reader.read_exact(&mut buffer).is_err() {
        return false;
    }

    let content = String::from_utf8_lossy(&buffer);
    let complete_content = if (read_size as u64) < file_size {
        let Some(first_newline) = content.find('\n') else {
            return false;
        };
        &content[first_newline + 1..]
    } else {
        &content
    };

    for line in complete_content.lines().rev() {
        if line.trim().is_empty() {
            continue;
        }
        let Ok(record) = serde_json::from_str::<Value>(line) else {
            return false;
        };

        if record
            .get("attachment")
            .and_then(|attachment| attachment.get("type"))
            .and_then(Value::as_str)
            == Some("queued_command")
        {
            return false;
        }

        match record.get("type").and_then(Value::as_str) {
            Some("assistant") => return false,
            Some("user") => return user_text(&record).starts_with(INTERRUPT_PREFIX),
            _ => {}
        }
    }

    false
}

fn user_text(record: &Value) -> String {
    match record
        .get("message")
        .and_then(|message| message.get("content"))
    {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(blocks)) => blocks
            .iter()
            .filter_map(|block| block.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}
