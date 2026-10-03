use std::ffi::{OsStr, OsString};

use crate::infra::external_tool::ExternalTool;
use crate::infra::process;

pub(crate) struct FloatingPaneSpec<'a> {
    pub(crate) parent_pane_id: &'a str,
    pub(crate) title: &'a str,
    pub(crate) command: &'a OsStr,
}

pub(crate) fn open_floating_pane(spec: FloatingPaneSpec<'_>) -> super::Result<String> {
    let args = floating_pane_args(spec.parent_pane_id, spec.command);
    let pane_id = run_tmux_output_with_os_args(&args)?;
    let title_args = pane_title_args(&pane_id, spec.title);
    if let Err(error) = super::run_tmux(&title_args.iter().map(String::as_str).collect::<Vec<_>>())
    {
        let _ = super::kill_pane(&pane_id);
        return Err(error);
    }
    Ok(pane_id)
}

fn floating_pane_args(parent_pane_id: &str, command: &OsStr) -> Vec<OsString> {
    let mut args = [
        "new-pane",
        "-P",
        "-F",
        "#{pane_id}",
        "-x",
        "90%",
        "-y",
        "90%",
        "-X",
        "5%",
        "-Y",
        "5%",
        "-S",
        "fg=colour98",
        "-t",
        parent_pane_id,
    ]
    .into_iter()
    .map(OsString::from)
    .collect::<Vec<_>>();
    args.push(command.to_os_string());
    args
}

fn pane_title_args(pane_id: &str, title: &str) -> Vec<String> {
    vec![
        "select-pane".to_string(),
        "-T".to_string(),
        tmux_title(title),
        "-t".to_string(),
        pane_id.to_string(),
    ]
}

fn tmux_title(value: &str) -> String {
    value
        .chars()
        .filter(|c| !c.is_control())
        .collect::<String>()
        .replace('#', "##")
}

fn run_tmux_output_with_os_args(args: &[OsString]) -> super::Result<String> {
    let mut command = ExternalTool::Tmux.command();
    command.args(args);

    let output = process::run_with_timeout(command, super::TMUX_COMMAND_TIMEOUT)
        .map_err(|error| tmux_command_error(args, error.to_string(), None))?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(tmux_command_error(
            args,
            "command exited with non-zero status".to_string(),
            Some(stderr),
        ))
    }
}

fn tmux_command_error(
    args: &[OsString],
    message: String,
    stderr: Option<String>,
) -> super::TmuxError {
    let display_args = args
        .iter()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    super::TmuxError::command_failed(
        &display_args.iter().map(String::as_str).collect::<Vec<_>>(),
        message,
        stderr,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn floating_pane_args_preserve_the_command_as_one_argument() {
        let command = OsString::from("'/tmp/example app' --review");

        assert_eq!(
            floating_pane_args("%12", &command),
            vec![
                "new-pane",
                "-P",
                "-F",
                "#{pane_id}",
                "-x",
                "90%",
                "-y",
                "90%",
                "-X",
                "5%",
                "-Y",
                "5%",
                "-S",
                "fg=colour98",
                "-t",
                "%12",
                "'/tmp/example app' --review",
            ],
        );
    }

    #[test]
    fn pane_title_args_escape_tmux_formats_and_remove_controls() {
        assert_eq!(
            pane_title_args("%18", "review #1\nready"),
            vec!["select-pane", "-T", "review ##1ready", "-t", "%18"],
        );
    }
}
