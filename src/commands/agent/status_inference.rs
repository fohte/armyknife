mod command;
mod fork;
mod prompt;
mod response;

// The global ask rule outranks user allow rules; dontAsk denies each prompt.
pub(in crate::commands::agent) const FORK_SETTINGS: &str =
    r#"{"disableAllHooks":true,"permissions":{"ask":["*"]}}"#;
pub(in crate::commands::agent) use response::parse_configured_status_response;

#[cfg(not(test))]
pub use command::spawn_in_background;
pub(crate) use command::{
    CodexStopSnapshot, StatusInferenceSpawnRequest, record_codex_failure_status_if_unchanged,
};
pub use command::{StatusInferenceArgs, run};
