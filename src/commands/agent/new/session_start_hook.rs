use anyhow::{Context, Result};
use std::path::Path;

use crate::shared::env_var::EnvVars;

pub(super) fn run_pre_session_start_hook(
    skip_hooks: bool,
    session_cwd: &Path,
    run_hook: impl FnOnce(&str, &[(&str, &str)]) -> Result<()>,
) -> Result<()> {
    if skip_hooks {
        eprintln!("Skipping pre-session-start hook (--skip-hooks)");
        return Ok(());
    }

    let session_cwd = session_cwd
        .to_str()
        .context("Session CWD is not valid UTF-8")?;
    run_hook(
        "pre-session-start",
        &[(EnvVars::session_cwd_name(), session_cwd)],
    )
}
