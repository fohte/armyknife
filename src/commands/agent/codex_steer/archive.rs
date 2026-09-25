use anyhow::{Context, anyhow};
use serde_json::json;

use super::{Client, RpcRequest};

const THREAD_ARCHIVE_REQUEST_ID: u64 = 2;

pub(crate) fn archive_thread(thread_id: &str) -> anyhow::Result<()> {
    let socket_path = super::control_socket_path()?;
    if !socket_path.exists() {
        return Err(anyhow!("Codex app-server control socket is unavailable"));
    }

    let mut client = Client::connect_to(&socket_path)?;
    super::send_request(&mut client.socket, &thread_archive_request(thread_id))
        .map_err(super::request_error_to_anyhow)
        .context("failed to archive Codex thread")
}

fn thread_archive_request(thread_id: &str) -> RpcRequest {
    RpcRequest {
        id: THREAD_ARCHIVE_REQUEST_ID,
        method: "thread/archive",
        payload: json!({
            "id": THREAD_ARCHIVE_REQUEST_ID,
            "method": "thread/archive",
            "params": {
                "threadId": thread_id,
            },
        }),
    }
}
