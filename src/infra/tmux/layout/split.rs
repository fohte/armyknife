use std::path::Path;

use super::codex::{Launch as CodexLaunch, RecoverySpec as CodexRecoverySpec};
use super::execution::{execute_commands, flatten_commands};
use super::prompt::apply_prompt_if_agent;
use super::{
    AgentLaunchRoute, TmuxCommand, TmuxSessionSpec, set_environment_commands,
    unset_environment_commands, write_prompt_file,
};

/// Inputs for `build_split_pane_setup_commands`: the `set-environment` +
/// `split-window` commands that must run before the new pane's id is known
/// (mirrors `execute_layout`'s new-window capture).
pub(super) struct SplitPaneSetupSpec<'a> {
    pub(super) session: &'a str,
    pub(super) target_pane: &'a str,
    pub(super) cwd: &'a str,
    pub(super) env_vars: &'a [(&'a str, &'a str)],
    pub(super) background: bool,
}

/// Builds the `set-environment` (if any) + `split-window` command sequence.
/// Always splits horizontally (side-by-side): this path has no layout
/// config, unlike `--worktree`'s `config.wm.layout`.
pub(super) fn build_split_pane_setup_commands(spec: SplitPaneSetupSpec) -> Vec<TmuxCommand> {
    let mut commands = set_environment_commands(spec.session, spec.env_vars);
    let mut split_args = vec!["split-window"];
    if spec.background {
        split_args.push("-d");
    }
    split_args.extend(["-h", "-t", spec.target_pane, "-c", spec.cwd]);
    commands.push(TmuxCommand::new(&split_args));
    commands
}

/// Inputs for `split_pane`.
pub struct SplitSpec<'a> {
    pub common: TmuxSessionSpec<'a>,
    pub target_pane: &'a str,
    pub command: &'a str,
}

/// The created pane and the route used for its initial prompt.
pub struct SplitResult {
    pub pane_id: String,
    pub route: AgentLaunchRoute,
}

/// Splits `target_pane` into a new pane within the same window and starts
/// `command` there (typically `claude` or `codex`). Returns the new pane's id
/// and the route used for the initial prompt.
///
/// Unlike `build_layout`, this never creates a window: `a agent new` without
/// `--worktree` uses it to keep a handoff session visually attached to the
/// pane it continues, instead of opening in a separate window.
pub fn split_pane(spec: SplitSpec) -> anyhow::Result<SplitResult> {
    let SplitSpec {
        common:
            TmuxSessionSpec {
                session,
                cwd,
                model,
                reasoning_effort,
                prompt,
                engine,
                env_vars,
                background,
            },
        target_pane,
        command,
    } = spec;

    let codex_launch = CodexLaunch::prepare(engine, prompt, 1, Path::new(cwd));
    let prompt_file = prompt.map(write_prompt_file).transpose()?;
    let (launch_effort, launch_prompt_file) =
        codex_launch.command_options(reasoning_effort, prompt_file.as_deref());
    let cmd = apply_prompt_if_agent(
        command,
        engine,
        model,
        launch_effort,
        launch_prompt_file,
        true,
    );

    let setup = build_split_pane_setup_commands(SplitPaneSetupSpec {
        session,
        target_pane,
        cwd,
        env_vars,
        background,
    });

    let mut capture_args = flatten_commands(&setup);
    capture_args.extend(["-P", "-F", "#{pane_id}"]);
    let new_pane_id = crate::infra::tmux::run_tmux_output(&capture_args)?;

    let mut remaining = vec![
        TmuxCommand::new(&[
            "send-keys",
            "-t",
            new_pane_id.as_str(),
            "-l",
            "--",
            cmd.as_str(),
        ]),
        TmuxCommand::new(&["send-keys", "-t", new_pane_id.as_str(), "C-m"]),
    ];
    remaining.extend(unset_environment_commands(session, env_vars));
    execute_commands(&remaining).inspect_err(|_| {
        // Best-effort: if send-keys/unset itself failed partway, don't
        // leave session-level env vars leaked past this call's lifetime.
        for (key, _) in env_vars {
            let _ = crate::infra::tmux::run_tmux(&["set-environment", "-u", "-t", session, key]);
        }
    })?;

    let route = codex_launch.finish_and_recover(CodexRecoverySpec {
        cwd: Path::new(cwd),
        effort: reasoning_effort,
        prompt_file: prompt_file.as_deref(),
        command,
        model,
        pane_id: Some(&new_pane_id),
        env_vars,
    })?;

    Ok(SplitResult {
        pane_id: new_pane_id,
        route,
    })
}
