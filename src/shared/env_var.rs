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
const PR_TITLE: &str = "ARMYKNIFE_PR_TITLE";
const PR_BODY_FILE: &str = "ARMYKNIFE_PR_BODY_FILE";
const PR_OWNER: &str = "ARMYKNIFE_PR_OWNER";
const PR_REPO: &str = "ARMYKNIFE_PR_REPO";
const PR_HEAD: &str = "ARMYKNIFE_PR_HEAD";
const PR_BASE: &str = "ARMYKNIFE_PR_BASE";
const PR_NUMBER: &str = "ARMYKNIFE_PR_NUMBER";
const PR_IS_UPDATE: &str = "ARMYKNIFE_PR_IS_UPDATE";

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

    /// Returns the env var name for PR_TITLE (passed to `pre-pr-submit` hook).
    pub fn pr_title_name() -> &'static str {
        PR_TITLE
    }

    /// Returns the env var name for PR_BODY_FILE.
    /// Path to a temporary file containing the PR body so the hook can grep it
    /// without worrying about environment variable size limits.
    pub fn pr_body_file_name() -> &'static str {
        PR_BODY_FILE
    }

    /// Returns the env var name for PR_OWNER (target repository owner).
    pub fn pr_owner_name() -> &'static str {
        PR_OWNER
    }

    /// Returns the env var name for PR_REPO (target repository name).
    pub fn pr_repo_name() -> &'static str {
        PR_REPO
    }

    /// Returns the env var name for PR_HEAD (head branch the PR is created from).
    pub fn pr_head_name() -> &'static str {
        PR_HEAD
    }

    /// Returns the env var name for PR_BASE (base branch; empty when defaulted).
    pub fn pr_base_name() -> &'static str {
        PR_BASE
    }

    /// Returns the env var name for PR_NUMBER (set only when updating an existing PR).
    pub fn pr_number_name() -> &'static str {
        PR_NUMBER
    }

    /// Returns the env var name for PR_IS_UPDATE ("1" when updating, "0" when creating).
    pub fn pr_is_update_name() -> &'static str {
        PR_IS_UPDATE
    }
}
