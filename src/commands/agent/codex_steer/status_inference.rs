mod classifier;

use anyhow::Result;

use super::Client;
use crate::commands::agent::types::AgentStatus;

pub(crate) struct Request<'a> {
    pub thread_id: &'a str,
    pub turn_id: &'a str,
    pub last_assistant_message: &'a str,
    pub model: &'a str,
    pub prompt: &'a str,
    pub configured_kinds: &'a [String],
}

pub(crate) fn infer_status(request: &Request<'_>) -> Result<AgentStatus> {
    let mut client = Client::connect()?;
    let conversation = client.read_complete_conversation(
        request.thread_id,
        request.turn_id,
        request.last_assistant_message,
    )?;
    classifier::infer_status(
        request.model,
        request.prompt,
        request.configured_kinds,
        &conversation,
    )
}
