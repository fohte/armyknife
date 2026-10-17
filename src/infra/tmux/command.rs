use std::ffi::OsStr;
use std::time::Duration;

use crate::infra::external_tool::ExternalTool;
use crate::infra::process;

use super::{Result, TmuxError};

/// Upper bound on how long a single `tmux` invocation may run.
///
/// `armyknife` itself can be invoked as a tmux `run-shell` job (e.g. from a
/// tmux-resurrect hook). If such a job then shells out back into `tmux`, the
/// new client can deadlock waiting for a server that is itself blocked
/// processing that same job queue. Bounding the wait turns that deadlock into
/// a fast, reported error instead of a hang with no upper bound.
const TMUX_COMMAND_TIMEOUT: Duration = Duration::from_secs(5);

/// Run a tmux command and return stdout on success.
pub(crate) fn run_tmux_output<S: AsRef<OsStr>>(args: &[S]) -> Result<String> {
    run_tmux_output_untrimmed(args).map(|output| output.trim().to_string())
}

/// Run a tmux command and return stdout on success without trimming it.
pub(crate) fn run_tmux_output_untrimmed<S: AsRef<OsStr>>(args: &[S]) -> Result<String> {
    let mut command = ExternalTool::Tmux.command();
    command.args(args);
    let display_args = args
        .iter()
        .map(|arg| arg.as_ref().to_string_lossy().into_owned())
        .collect::<Vec<_>>();

    let output = process::run_with_timeout(command, TMUX_COMMAND_TIMEOUT)
        .map_err(|e| TmuxError::command_failed(&display_args, e.to_string(), None))?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(TmuxError::command_failed(
            &display_args,
            "command exited with non-zero status",
            Some(stderr),
        ))
    }
}
