//! Hook integration for PR draft commands.
//!
//! Both `review` and `submit` expose the same PR-shaped contract to user
//! hooks (title, body, owner/repo/branch). Centralizing the env-var wiring
//! here keeps the two call sites in sync and keeps a single source of truth
//! for the env-var names listed in README.

use std::io::Write;

use super::common::DraftFile;
use crate::shared::env_var::EnvVars;
use crate::shared::hooks;

/// Hook fired before `a ai pr-draft review` opens the editor. Lets users lint
/// the draft body so violations surface before the human review step instead
/// of at submit time.
pub const PRE_PR_REVIEW_HOOK: &str = "pre-pr-review";

/// Hook fired right before `a ai pr-draft submit` creates or updates a PR on
/// GitHub. A non-zero exit aborts submission and acts as the final gate.
pub const PRE_PR_SUBMIT_HOOK: &str = "pre-pr-submit";

/// Function signature for executing a hook script. Production code uses
/// [`hooks::run_hook`]; tests inject a closure to verify invocation without
/// mutating the real `XDG_CONFIG_HOME`.
pub type HookRunner<'a> = &'a (dyn Fn(&str, &[(&str, &str)]) -> anyhow::Result<()> + Send + Sync);

/// Repository / branch context passed to PR-shaped hooks.
pub struct HookContext<'a> {
    pub owner: &'a str,
    pub repo: &'a str,
    pub head_branch: &'a str,
    /// Base branch as supplied by the caller. Empty string when defaulted by
    /// GitHub. Only meaningful at submit time.
    pub base_branch: &'a str,
    /// `Some(n)` when an existing open PR will be updated, `None` when a new
    /// PR will be created. Only meaningful at submit time; review-time hooks
    /// receive `None`.
    pub update_pr_number: Option<u64>,
}

/// Materialize the PR body as a temp file and invoke the named hook.
///
/// The body is written to disk rather than passed via env so that hook
/// scripts can grep/match it without worrying about argv/env size limits and
/// so that embedded newlines round-trip exactly. The temp file is deleted as
/// soon as this function returns.
pub fn run_pr_hook(
    hook_name: &str,
    draft: &DraftFile,
    context: &HookContext<'_>,
    run_hook: HookRunner<'_>,
) -> anyhow::Result<()> {
    let mut body_file = tempfile::Builder::new()
        .prefix("armyknife-pr-body-")
        .suffix(".md")
        .tempfile()?;
    body_file.write_all(draft.body.as_bytes())?;
    body_file.flush()?;

    let body_path = body_file
        .path()
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("PR body temp file path is not valid UTF-8"))?
        .to_owned();
    let pr_number = context
        .update_pr_number
        .map(|n| n.to_string())
        .unwrap_or_default();
    let is_update = if context.update_pr_number.is_some() {
        "1"
    } else {
        "0"
    };

    run_hook(
        hook_name,
        &[
            (EnvVars::pr_title_name(), draft.frontmatter.title.as_str()),
            (EnvVars::pr_body_file_name(), body_path.as_str()),
            (EnvVars::pr_owner_name(), context.owner),
            (EnvVars::pr_repo_name(), context.repo),
            (EnvVars::pr_head_name(), context.head_branch),
            (EnvVars::pr_base_name(), context.base_branch),
            (EnvVars::pr_number_name(), pr_number.as_str()),
            (EnvVars::pr_is_update_name(), is_update),
        ],
    )?;

    Ok(())
}

/// Production hook runner. Tests inject a closure instead.
pub fn default_runner() -> HookRunner<'static> {
    &hooks::run_hook
}
