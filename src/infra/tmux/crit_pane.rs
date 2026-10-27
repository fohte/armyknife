use anyhow::{Context, Result};

use crate::infra::external_tool::ExternalTool;

const CRIT_PANE_OPTION: &str = "@armyknife-crit-pane";
const CRIT_PANE_PORT_OPTION: &str = "@armyknife-crit-pane-port";

pub(crate) struct CritPaneSpec<'a> {
    pub(crate) parent_pane_id: &'a str,
    pub(crate) url: &'a str,
    pub(crate) port: u16,
    pub(crate) title: &'a str,
    pub(crate) force_shpool_attach: bool,
}

pub(crate) fn get_crit_pane_parent_id(pane_id: &str) -> Option<String> {
    super::get_pane_option(pane_id, CRIT_PANE_OPTION)
}

pub(crate) fn get_crit_pane_port(pane_id: &str) -> Option<u16> {
    super::get_pane_option(pane_id, CRIT_PANE_PORT_OPTION)?
        .parse()
        .ok()
}

pub(crate) fn find_crit_panes_for_parent(parent_pane_id: &str) -> super::Result<Vec<String>> {
    find_crit_panes_for_parent_with(parent_pane_id, |args| super::run_tmux_output(args))
}

fn find_crit_panes_for_parent_with(
    parent_pane_id: &str,
    run_tmux_output: impl FnOnce(&[&str]) -> super::Result<String>,
) -> super::Result<Vec<String>> {
    let filter = format!("#{{==:#{{{CRIT_PANE_OPTION}}},{parent_pane_id}}}");
    let output = run_tmux_output(&["list-panes", "-a", "-f", &filter, "-F", "#{pane_id}"])?;
    Ok(output
        .lines()
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect())
}

pub(crate) fn find_crit_panes_for_port(port: u16) -> super::Result<Vec<String>> {
    let filter = format!("#{{==:#{{{CRIT_PANE_PORT_OPTION}}},{port}}}");
    let output = super::run_tmux_output(&["list-panes", "-a", "-f", &filter, "-F", "#{pane_id}"])?;
    Ok(output
        .lines()
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect())
}

pub(crate) fn close_crit_pane(pane_id: &str) -> super::Result<()> {
    super::kill_pane(pane_id)
}

pub(crate) fn open_crit_pane(spec: CritPaneSpec<'_>) -> Result<()> {
    let shpool = tool_path(ExternalTool::Shpool)?;
    let terminal_browser = tool_path(ExternalTool::TerminalBrowser)?;
    let shpool_session = format!("crit-{}", spec.port);
    let inner_command = shlex::try_join([terminal_browser.as_str(), "open", spec.url])
        .context("failed to quote terminal-browser command")?;
    let shpool_command = shpool_attach_command(
        shpool.as_str(),
        inner_command.as_ref(),
        shpool_session.as_str(),
        spec.force_shpool_attach,
    )
    .context("failed to quote shpool command")?;
    // shpool does not restore modifyOtherKeys across session attaches.
    let command = format!("printf '\\033[>4;2m' && exec {shpool_command}");

    let crit_pane_id = super::open_floating_pane(super::FloatingPaneSpec {
        parent_pane_id: spec.parent_pane_id,
        title: spec.title,
        command: command.as_ref(),
    })?;
    let pane_setup: super::Result<()> = (|| {
        super::set_pane_option(&crit_pane_id, CRIT_PANE_PORT_OPTION, &spec.port.to_string())?;
        super::set_pane_option(&crit_pane_id, CRIT_PANE_OPTION, spec.parent_pane_id)?;
        Ok(())
    })();
    if let Err(error) = pane_setup {
        let _ = close_crit_pane(&crit_pane_id);
        return Err(error.into());
    }
    Ok(())
}

fn shpool_attach_command(
    shpool: &str,
    inner_command: &str,
    session: &str,
    force: bool,
) -> Result<String> {
    let mut args = vec![shpool, "attach"];
    if force {
        args.push("--force");
    }
    args.extend(["-c", inner_command, session]);
    shlex::try_join(args).context("failed to quote shpool command")
}

pub(super) fn tool_path(tool: ExternalTool) -> Result<String> {
    tool.resolve_path()
        .with_context(|| format!("{} executable not found on PATH", tool.name()))?
        .to_str()
        .map(str::to_string)
        .with_context(|| format!("{} executable path is not valid UTF-8", tool.name()))
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    #[rstest]
    #[case::ordinary_attach(
        false,
        "/mock/bin/shpool attach -c '/mock/bin/terminal-browser open http://127.0.0.1:58321/review' crit-58321"
    )]
    #[case::forced_attach(
        true,
        "/mock/bin/shpool attach --force -c '/mock/bin/terminal-browser open http://127.0.0.1:58321/review' crit-58321"
    )]
    fn shpool_attach_command_selects_force_flag(#[case] force: bool, #[case] expected: &str) {
        assert_eq!(
            shpool_attach_command(
                "/mock/bin/shpool",
                "/mock/bin/terminal-browser open http://127.0.0.1:58321/review",
                "crit-58321",
                force,
            )
            .map_err(|error| error.to_string()),
            Ok(expected.to_string()),
        );
    }

    #[rstest]
    #[case::searches_all_windows("%test-parent", "#{==:#{@armyknife-crit-pane},%test-parent}")]
    fn find_crit_panes_for_parent_searches_all_windows(
        #[case] parent_pane_id: &str,
        #[case] filter: &str,
    ) {
        let mut operation = None;
        let result = find_crit_panes_for_parent_with(parent_pane_id, |args| {
            operation = Some(args.iter().map(|arg| arg.to_string()).collect::<Vec<_>>());
            Ok(indoc::indoc! {
                "
                %test-crit
                %other-test-crit
                "
            }
            .to_string())
        })
        .map_err(|error| error.to_string());

        assert_eq!(
            (result, operation),
            (
                Ok(vec![
                    "%test-crit".to_string(),
                    "%other-test-crit".to_string()
                ]),
                Some(vec![
                    "list-panes".to_string(),
                    "-a".to_string(),
                    "-f".to_string(),
                    filter.to_string(),
                    "-F".to_string(),
                    "#{pane_id}".to_string(),
                ]),
            ),
        );
    }
}
