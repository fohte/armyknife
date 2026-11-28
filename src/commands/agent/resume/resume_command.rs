use crate::commands::agent::types::Session;

/// Wraps `a agent resume` in the user's login shell so that control returns
/// to a shell prompt instead of tmux closing the pane after the agent exits.
///
/// `-i` is required on the outer shell because the selected agent CLI may
/// only be available in the user's interactive PATH.
pub(super) fn build_resume_command() -> Option<String> {
    let exe = std::env::current_exe()
        .ok()
        .and_then(|path| path.to_str().map(String::from))
        .unwrap_or_else(|| "a".to_string());
    let inner = shlex::try_join([exe.as_str(), "agent", "resume"]).ok()?;
    build_login_shell_command(&inner)
}

pub(super) fn build_resume_command_for_session(session: &Session) -> Option<String> {
    let inner = crate::commands::agent::resurrect::resume_command_for_session(session);
    build_login_shell_command(&inner)
}

fn build_login_shell_command(inner: &str) -> Option<String> {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
    let exec_shell = shlex::try_join([shell.as_str(), "-i"]).ok()?;
    let script = format!("{inner}; exec {exec_shell}");
    shlex::try_join([shell.as_str(), "-i", "-c", &script]).ok()
}
