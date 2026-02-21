//! Centralized reader for ARMYKNIFE_* environment variables.
//!
//! Environment variable names are defined as private constants here;
//! external code accesses values through the `EnvVars` struct.

const SKIP_HOOKS: &str = "ARMYKNIFE_SKIP_HOOKS";
const SESSION_ID: &str = "ARMYKNIFE_SESSION_ID";
const SESSION_LABEL: &str = "ARMYKNIFE_SESSION_LABEL";
const ANCESTOR_SESSION_IDS: &str = "ARMYKNIFE_ANCESTOR_SESSION_IDS";
const CC_HOOK_LOG: &str = "ARMYKNIFE_CC_HOOK_LOG";
const CC_NOTIFY: &str = "ARMYKNIFE_CC_NOTIFY";
const WORKTREE_PATH: &str = "ARMYKNIFE_WORKTREE_PATH";
const BRANCH_NAME: &str = "ARMYKNIFE_BRANCH_NAME";
const REPO_ROOT: &str = "ARMYKNIFE_REPO_ROOT";

/// Snapshot of all ARMYKNIFE_* environment variables at load time.
pub struct EnvVars {
    /// When set, hooks are skipped entirely.
    /// Used by `claude -p` invocations to prevent infinite recursion.
    pub skip_hooks: bool,

    /// Session ID of the current Claude Code session.
    pub session_id: Option<String>,

    /// Session label set by `wm new --label` or auto-generated.
    pub session_label: Option<String>,

    /// Comma-separated ancestor session IDs from root to immediate parent.
    pub ancestor_session_ids: Option<String>,

    /// Hook log level: "debug", "error", or unset (default: error).
    pub cc_hook_log: Option<String>,

    /// Override notification behavior: "on" or "off".
    pub cc_notify: Option<String>,
}

fn non_empty_var(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|s| !s.is_empty())
}

impl EnvVars {
    /// Read all ARMYKNIFE_* environment variables from the current process.
    pub fn load() -> Self {
        Self {
            skip_hooks: std::env::var(SKIP_HOOKS).is_ok(),
            session_id: non_empty_var(SESSION_ID),
            session_label: non_empty_var(SESSION_LABEL),
            ancestor_session_ids: non_empty_var(ANCESTOR_SESSION_IDS),
            cc_hook_log: non_empty_var(CC_HOOK_LOG),
            cc_notify: non_empty_var(CC_NOTIFY),
        }
    }

    /// Returns env var pairs for setting SKIP_HOOKS in a child process.
    pub fn skip_hooks_pair() -> (&'static str, &'static str) {
        (SKIP_HOOKS, "1")
    }

    /// Returns the env var name for SESSION_ID (used in CLAUDE_ENV_FILE export).
    pub fn session_id_name() -> &'static str {
        SESSION_ID
    }

    /// Returns the env var name for SESSION_LABEL (used as key in env var pairs).
    pub fn session_label_name() -> &'static str {
        SESSION_LABEL
    }

    /// Returns the env var name for ANCESTOR_SESSION_IDS (used as key in env var pairs).
    pub fn ancestor_session_ids_name() -> &'static str {
        ANCESTOR_SESSION_IDS
    }

    /// Returns the env var name for WORKTREE_PATH (used as key in env var pairs).
    pub fn worktree_path_name() -> &'static str {
        WORKTREE_PATH
    }

    /// Returns the env var name for BRANCH_NAME (used as key in env var pairs).
    pub fn branch_name_name() -> &'static str {
        BRANCH_NAME
    }

    /// Returns the env var name for REPO_ROOT (used as key in env var pairs).
    pub fn repo_root_name() -> &'static str {
        REPO_ROOT
    }
}
