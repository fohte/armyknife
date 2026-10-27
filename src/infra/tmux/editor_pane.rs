use anyhow::{Context, Result};

use crate::infra::external_tool::ExternalTool;

/// Marks a floating pane as a review editor and holds its agent pane ID.
const EDITOR_PANE_OPTION: &str = "@armyknife-editor-pane";

pub(crate) struct EditorPaneSpec<'a> {
    pub(crate) parent_pane_id: &'a str,
    pub(crate) title: &'a str,
    pub(crate) shpool_session: &'a str,
    /// Command that creates the session. `None` attaches to a running session.
    pub(crate) command: Option<&'a str>,
    pub(crate) force_shpool_attach: bool,
}

/// Names a shpool session after the pane that started the review so that the
/// session can be traced back to its agent pane.
pub(crate) fn new_editor_session_name(parent_pane_id: &str) -> String {
    format!(
        "armyknife-editor-{}-{}",
        parent_pane_id.trim_start_matches('%'),
        uuid::Uuid::new_v4().simple()
    )
}

pub(crate) fn open_editor_pane(spec: EditorPaneSpec<'_>) -> Result<String> {
    let shpool = super::crit_pane::tool_path(ExternalTool::Shpool)?;
    let shpool_command = shpool_attach_command(shpool.as_str(), &spec)?;
    // shpool does not restore modifyOtherKeys across session attaches.
    let command = format!("printf '\\033[>4;2m' && exec {shpool_command}");

    let pane_id = super::open_floating_pane(super::FloatingPaneSpec {
        parent_pane_id: spec.parent_pane_id,
        title: spec.title,
        command: command.as_ref(),
    })?;
    if let Err(error) = super::set_pane_option(&pane_id, EDITOR_PANE_OPTION, spec.parent_pane_id) {
        let _ = super::kill_pane(&pane_id);
        return Err(error.into());
    }
    Ok(pane_id)
}

pub(crate) fn find_editor_panes_for_parent(parent_pane_id: &str) -> super::Result<Vec<String>> {
    let filter = format!("#{{==:#{{{EDITOR_PANE_OPTION}}},{parent_pane_id}}}");
    let output = super::run_tmux_output(&["list-panes", "-a", "-f", &filter, "-F", "#{pane_id}"])?;
    Ok(output
        .lines()
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect())
}

pub(crate) fn get_editor_pane_parent_id(pane_id: &str) -> Option<String> {
    super::get_pane_option(pane_id, EDITOR_PANE_OPTION)
}

/// Creates the session without showing it, for a review that has to wait
/// behind another editor.
pub(crate) fn start_hidden_editor_session(session: &str, command: &str) -> Result<()> {
    let output = ExternalTool::Shpool
        .command()
        .args(["attach", "--background", "-c", command, session])
        .output()
        .context("failed to run shpool attach")?;
    anyhow::ensure!(
        output.status.success(),
        "shpool attach --background exited with {}: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr).trim()
    );
    Ok(())
}

fn shpool_attach_command(shpool: &str, spec: &EditorPaneSpec<'_>) -> Result<String> {
    let mut args = vec![shpool, "attach"];
    if spec.force_shpool_attach {
        args.push("--force");
    }
    if let Some(command) = spec.command {
        args.extend(["-c", command]);
    }
    args.push(spec.shpool_session);
    shlex::try_join(args).context("failed to quote shpool command")
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    #[rstest]
    #[case::creates_session(
        false,
        Some("/bin/sh -c 'run review'"),
        "/mock/bin/shpool attach -c \"/bin/sh -c 'run review'\" editor-session"
    )]
    #[case::reattaches_with_force(true, None, "/mock/bin/shpool attach --force editor-session")]
    fn shpool_attach_command_selects_flags(
        #[case] force: bool,
        #[case] command: Option<&str>,
        #[case] expected: &str,
    ) {
        let spec = EditorPaneSpec {
            parent_pane_id: "%1",
            title: "title",
            shpool_session: "editor-session",
            command,
            force_shpool_attach: force,
        };

        assert_eq!(
            shpool_attach_command("/mock/bin/shpool", &spec).map_err(|error| error.to_string()),
            Ok(expected.to_string()),
        );
    }
}
