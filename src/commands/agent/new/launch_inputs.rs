use anyhow::Result;

use super::CommonNewArgs;
use super::session_context::build_ancestor_chain;
use crate::shared::env_var::EnvVars;

/// Reads the parent process's `TQ_SESSION_ID` (exported by tq's own
/// SessionStart hook into `CLAUDE_ENV_FILE`, mirroring how armyknife exports
/// `ARMYKNIFE_SESSION_ID`) and returns it as the `TQ_PARENT_SESSION_ID` pair
/// for the child session. `None` when the parent has no tq hook installed.
pub(super) fn tq_parent_session_env_var() -> Option<(String, String)> {
    std::env::var("TQ_SESSION_ID")
        .ok()
        .filter(|s| !s.is_empty())
        .map(|id| ("TQ_PARENT_SESSION_ID".to_string(), id))
}

/// Build tmux session-level env vars for the child session's launch metadata,
/// ancestor-session-id chain, and tq parent session ID. Shared between the
/// worktree and no-worktree flows.
pub(super) fn build_env_vars(
    common: &CommonNewArgs,
    vibe: bool,
    own_session_id: Option<String>,
) -> Result<Vec<(String, String)>> {
    let mut env_vars: Vec<(String, String)> = Vec::new();
    if let Some(ref label) = common.label {
        env_vars.push((EnvVars::session_label_name().to_string(), label.clone()));
    }
    if let Some(ref kind) = common.kind {
        env_vars.push((EnvVars::session_work_type_name().to_string(), kind.clone()));
    }
    let child_launch = common.child_launch.unwrap_or_default();
    env_vars.push((
        EnvVars::session_child_launch_name().to_string(),
        child_launch.as_env_str().to_string(),
    ));
    if vibe {
        env_vars.push((
            EnvVars::session_vibe_launch_name().to_string(),
            "1".to_string(),
        ));
    }
    // Resolve parent session ID: explicit flag > ARMYKNIFE_SESSION_ID env var.
    // ARMYKNIFE_SESSION_ID is set by the SessionStart hook via CLAUDE_ENV_FILE,
    // so `a agent new` called from a Claude Code Bash tool automatically inherits
    // the parent session ID without requiring --parent-session-id. Codex has no
    // env file, so it falls back to CODEX_SESSION_ID (see `own_session_id`).
    let parent_id = common.parent_session_id.clone().or(own_session_id);
    if let Some(ref parent_id) = parent_id {
        let ancestor_chain = build_ancestor_chain(parent_id)?;
        env_vars.push((
            EnvVars::ancestor_session_ids_name().to_string(),
            ancestor_chain,
        ));
    }
    if let Some(pair) = tq_parent_session_env_var() {
        env_vars.push(pair);
    }
    Ok(env_vars)
}
