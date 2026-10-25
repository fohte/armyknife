mod fork;
mod relay;

use anyhow::{Result, ensure};

use super::Client;
use crate::commands::agent::types::{AgentStatus, Session};

pub(crate) struct Request<'a> {
    pub session: &'a Session,
    pub thread_id: &'a str,
    pub turn_id: &'a str,
    pub last_assistant_message: &'a str,
    pub prompt: &'a str,
    pub configured_kinds: &'a [String],
}

pub(crate) fn infer_status(request: &Request<'_>) -> Result<AgentStatus> {
    ensure!(
        !request.turn_id.is_empty(),
        "Codex Stop snapshot has no turn ID"
    );
    ensure!(
        !request.last_assistant_message.trim().is_empty(),
        "Codex Stop snapshot has no final assistant message"
    );
    let mut client = Client::connect()?;
    let thread = client.read_complete_thread(
        request.thread_id,
        request.turn_id,
        request.last_assistant_message,
    )?;

    ensure!(
        thread.model_provider == "openai",
        "Codex status fork requires the built-in OpenAI provider"
    );

    fork::infer_status(
        request.session,
        &thread,
        request.prompt,
        request.configured_kinds,
    )
}
