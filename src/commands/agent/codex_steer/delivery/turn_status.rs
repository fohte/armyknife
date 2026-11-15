use std::os::unix::net::UnixStream;

use tungstenite::WebSocket;

use super::history::{
    latest_turn as latest_turn_record, latest_turn_error_message,
    latest_turn_status as latest_status,
};
use super::{ItemsView, read_turns};

pub(in crate::commands::agent::codex_steer) fn latest_turn_status(
    socket: &mut WebSocket<UnixStream>,
    thread_id: &str,
    request_id: u64,
) -> anyhow::Result<Option<String>> {
    let (thread, _) = read_turns(socket, thread_id, request_id, 1, ItemsView::NotLoaded)
        .map_err(super::super::request_error_to_anyhow)?;
    Ok(latest_status(&thread).map(str::to_string))
}

pub(in crate::commands::agent::codex_steer) fn latest_turn_status_and_error_message(
    socket: &mut WebSocket<UnixStream>,
    thread_id: &str,
    request_id: u64,
) -> anyhow::Result<Option<(String, Option<String>)>> {
    let (thread, _) = read_turns(socket, thread_id, request_id, 1, ItemsView::NotLoaded)
        .map_err(super::super::request_error_to_anyhow)?;
    Ok(latest_turn_record(&thread).map(|(_, status)| {
        (
            status.to_string(),
            latest_turn_error_message(&thread).map(str::to_string),
        )
    }))
}

pub(in crate::commands::agent::codex_steer) fn latest_turn_details(
    socket: &mut WebSocket<UnixStream>,
    thread_id: &str,
    request_id: u64,
) -> anyhow::Result<Option<(String, String)>> {
    let (thread, _) = read_turns(socket, thread_id, request_id, 1, ItemsView::NotLoaded)
        .map_err(super::super::request_error_to_anyhow)?;
    Ok(latest_turn_record(&thread)
        .map(|(turn_id, status)| (turn_id.to_string(), status.to_string())))
}
