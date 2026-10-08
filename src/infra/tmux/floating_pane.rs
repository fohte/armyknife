use std::ffi::{OsStr, OsString};

pub(crate) struct FloatingPaneSpec<'a> {
    pub(crate) parent_pane_id: &'a str,
    pub(crate) title: &'a str,
    pub(crate) command: &'a OsStr,
}

pub(crate) fn open_floating_pane(spec: FloatingPaneSpec<'_>) -> super::Result<String> {
    let args = floating_pane_args(spec.parent_pane_id, spec.command);
    let pane_id = super::run_tmux_output(&args)?;
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
        super::escape_format_value(title),
        "-t".to_string(),
        pane_id.to_string(),
    ]
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
