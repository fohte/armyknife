use std::collections::HashSet;
use std::fs::File;
use std::io::{self, BufReader, Read, Seek, SeekFrom};
use std::path::Path;

use serde_json::Value;

use super::messages::claude_content_text;
use crate::commands::agent::types::{Engine, Session, SessionStatus};

const MAX_TAIL_SIZE: usize = 64 * 1024;
const INTERRUPT_PREFIX: &str = "[Request interrupted by user";

pub(super) fn interrupted_session_ids(sessions: Vec<Session>) -> HashSet<String> {
    sessions
        .into_iter()
        .filter_map(|session| {
            is_waiting_after_user_interrupt(&session).then_some(session.session_id)
        })
        .collect()
}

fn is_waiting_after_user_interrupt(session: &Session) -> bool {
    if session.engine != Engine::Claude || session.status != SessionStatus::Running {
        return false;
    }

    let Some(path) = session.transcript_path.as_deref() else {
        return false;
    };
    match latest_relevant_record_is_interrupt(path) {
        Ok(is_interrupted) => is_interrupted,
        Err(error) => {
            tracing::warn!(
                session_id = %session.session_id,
                transcript_path = %path.display(),
                %error,
                "failed to inspect Claude transcript for a user interrupt"
            );
            false
        }
    }
}

fn latest_relevant_record_is_interrupt(path: &Path) -> io::Result<bool> {
    let file = File::open(path)?;
    let file_size = file.metadata()?.len();
    if file_size == 0 {
        return Ok(false);
    }

    let read_size = file_size.min(MAX_TAIL_SIZE as u64) as usize;
    let mut reader = BufReader::new(file);
    reader.seek(SeekFrom::End(-(read_size as i64)))?;
    let mut buffer = vec![0; read_size];
    reader.read_exact(&mut buffer)?;

    let content = String::from_utf8_lossy(&buffer);
    let complete_content = if (read_size as u64) < file_size {
        let Some(first_newline) = content.find('\n') else {
            return Ok(false);
        };
        &content[first_newline + 1..]
    } else {
        &content
    };

    for line in complete_content.lines().rev() {
        if line.trim().is_empty() {
            continue;
        }
        let record = serde_json::from_str::<Value>(line)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;

        if record
            .get("attachment")
            .and_then(|attachment| attachment.get("type"))
            .and_then(Value::as_str)
            == Some("queued_command")
        {
            return Ok(false);
        }

        match record.get("type").and_then(Value::as_str) {
            Some("assistant") => return Ok(false),
            Some("user") => {
                return Ok(claude_content_text(
                    record
                        .get("message")
                        .and_then(|message| message.get("content")),
                )
                .starts_with(INTERRUPT_PREFIX));
            }
            _ => {}
        }
    }

    Ok(false)
}
