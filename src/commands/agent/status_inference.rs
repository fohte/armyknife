mod command;
mod fork;
mod prompt;
mod response;

// The global ask rule outranks user allow rules; dontAsk denies each prompt.
pub(in crate::commands::agent) const FORK_SETTINGS: &str =
    r#"{"disableAllHooks":true,"permissions":{"ask":["*"]}}"#;
pub(in crate::commands::agent) use response::parse_claude_status;

#[cfg(not(test))]
pub use command::spawn_in_background;
pub use command::{StatusInferenceArgs, run};
