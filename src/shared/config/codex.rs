use serde::{Deserialize, Serialize};

use crate::commands::agent::types::ReasoningEffort;

/// Per-invocation `codex` defaults. Passed on the command line rather than
/// written to `~/.codex/config.toml`, so a hand-run `codex` is unaffected.
#[cfg_attr(feature = "schema-gen", derive(schemars::JsonSchema))]
#[derive(Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CodexConfig {
    /// Model passed to `codex --model` when `--model` is omitted.
    #[serde(default)]
    pub model: Option<String>,

    /// Value passed in the first Codex `turn/start` request when
    /// `--reasoning-effort` is omitted.
    #[serde(default)]
    pub reasoning_effort: Option<ReasoningEffort>,
}
