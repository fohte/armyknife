//! Layout builder: converts a LayoutNode tree into tmux command sequences.

use std::path::Path;

use anyhow::Context;

use crate::commands::agent::types::{Engine, ReasoningEffort};
use crate::shared::config::{LayoutNode, SplitDirection};

mod codex;
mod prompt;
pub use codex::AgentLaunchRoute;
use codex::Launch as CodexLaunch;
use prompt::{apply_prompt_if_agent, is_engine_command, retarget_agent_command};

/// A single tmux command represented as a list of arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TmuxCommand {
    pub args: Vec<String>,
}

impl TmuxCommand {
    fn new(args: &[&str]) -> Self {
        Self {
            args: args.iter().map(|s| s.to_string()).collect(),
        }
    }
}

/// Information about a pane collected during tree traversal.
struct PaneEntry {
    command: String,
    focus: bool,
}

/// The pane-targeting prefix used to address panes in a background-created
/// window before its real window ID is known (see `execute_background_layout`).
fn background_pane_prefix(session: &str, window_name: &str) -> String {
    format!("{session}:={window_name}.")
}

/// Inputs for `build_layout_commands`, grouped to keep its argument count in
/// check.
pub struct LayoutCommandsSpec<'a> {
    pub session: &'a str,
    pub cwd: &'a str,
    pub window_name: &'a str,
    pub layout: &'a LayoutNode,
    /// Inserted right after the program name in `engine` pane commands.
    pub model: Option<&'a str>,
    /// Passed to `engine` panes as `--effort` (claude) or
    /// `-c model_reasoning_effort=...` (codex).
    pub reasoning_effort: Option<ReasoningEffort>,
    /// When set, `engine` pane commands read the prompt from this file at
    /// shell execution time and delete it afterward.
    pub prompt_file: Option<&'a Path>,
    /// The agent CLI this session is for. `model` and `prompt_file` apply
    /// only to panes running it. A plain `claude` pane is retargeted to it
    /// (dropping its arguments) unless the layout already has a pane for it;
    /// all other panes, including another agent CLI in a user-configured
    /// layout, are left as written.
    pub engine: Engine,
    /// Set as tmux session-level environment variables so all panes in the
    /// window inherit them.
    pub env_vars: &'a [(&'a str, &'a str)],
    pub background: bool,
    /// Turns `automatic-rename` back on right after window creation, undoing
    /// tmux's default of disabling it whenever a window is created with an
    /// explicit name (`-n`).
    pub restore_automatic_rename: bool,
}

/// Builds `set-environment -t <session> <key> <value>` for each env var.
fn set_environment_commands(session: &str, env_vars: &[(&str, &str)]) -> Vec<TmuxCommand> {
    env_vars
        .iter()
        .map(|(key, value)| TmuxCommand::new(&["set-environment", "-t", session, key, value]))
        .collect()
}

/// Builds `set-environment -u -t <session> <key>` for each env var.
fn unset_environment_commands(session: &str, env_vars: &[(&str, &str)]) -> Vec<TmuxCommand> {
    env_vars
        .iter()
        .map(|(key, _)| TmuxCommand::new(&["set-environment", "-u", "-t", session, key]))
        .collect()
}

/// Build tmux command sequence from a LayoutNode tree.
///
/// Returns a list of tmux commands to create the window and configure panes.
/// The first command creates a new window, subsequent commands split panes.
pub fn build_layout_commands(spec: LayoutCommandsSpec) -> Vec<TmuxCommand> {
    let LayoutCommandsSpec {
        session,
        cwd,
        window_name,
        layout,
        model,
        reasoning_effort,
        prompt_file,
        engine,
        env_vars,
        background,
        restore_automatic_rename,
    } = spec;

    let mut commands = Vec::new();
    let mut pane_entries: Vec<PaneEntry> = Vec::new();

    // Set session-level environment variables before creating the window so
    // all spawned panes inherit them via tmux's update-environment mechanism.
    commands.extend(set_environment_commands(session, env_vars));

    // In background mode, create the window detached (`-d`) and address every
    // subsequent pane operation by the fully-qualified `{session}:={name}.N`
    // target so the attached client's active window never flips, not even for
    // a single frame.
    let new_window_args = if background {
        vec![
            "new-window",
            "-d",
            "-t",
            session,
            "-c",
            cwd,
            "-n",
            window_name,
        ]
    } else {
        vec!["new-window", "-t", session, "-c", cwd, "-n", window_name]
    };
    commands.push(TmuxCommand::new(&new_window_args));

    // Placed right after `new-window`, before any pane targets get rewritten
    // to a captured window ID (see `execute_background_layout`), so this can
    // always address the window by its just-created, still-unrenamed name.
    if restore_automatic_rename {
        let window_target = format!("{session}:={window_name}");
        commands.push(TmuxCommand::new(&[
            "set-option",
            "-w",
            "-t",
            &window_target,
            "automatic-rename",
            "on",
        ]));
    }

    let pane_prefix = if background {
        background_pane_prefix(session, window_name)
    } else {
        String::new()
    };

    // Recursively process layout tree to collect split commands and pane info.
    // Pane indices are 1-based in tmux (new-window creates pane 1).
    collect_layout(
        layout,
        &mut commands,
        &mut pane_entries,
        cwd,
        1,
        &pane_prefix,
    );

    // A layout that already has a pane for `engine` was written with that
    // engine in mind, so leave its `claude` pane alone.
    if !pane_entries
        .iter()
        .any(|e| is_engine_command(&e.command, engine))
    {
        for entry in &mut pane_entries {
            entry.command = retarget_agent_command(&entry.command, engine);
        }
    }

    // Find the last agent pane index so only it performs temp file cleanup
    let last_agent_index = prompt_file.and_then(|_| {
        pane_entries
            .iter()
            .rposition(|e| is_engine_command(&e.command, engine))
    });

    // Send commands to each pane
    for (i, entry) in pane_entries.iter().enumerate() {
        let pane_target = format!("{pane_prefix}{}", i + 1);
        let cleanup = last_agent_index == Some(i);
        let cmd = apply_prompt_if_agent(
            &entry.command,
            engine,
            model,
            reasoning_effort,
            prompt_file,
            cleanup,
        );
        commands.push(TmuxCommand::new(&["select-pane", "-t", &pane_target]));
        // Use -l to send the command literally (prevents interpreting special key sequences),
        // then send Enter separately. In background mode the active pane stays
        // on the user's current window, so send-keys must target the new pane
        // explicitly; in foreground the prior select-pane already makes the
        // target active so the -t argument is omitted to keep behavior intact.
        if background {
            commands.push(TmuxCommand::new(&[
                "send-keys",
                "-t",
                &pane_target,
                "-l",
                "--",
                &cmd,
            ]));
            commands.push(TmuxCommand::new(&["send-keys", "-t", &pane_target, "C-m"]));
        } else {
            commands.push(TmuxCommand::new(&["send-keys", "-l", "--", &cmd]));
            commands.push(TmuxCommand::new(&["send-keys", "C-m"]));
        }
    }

    // Focus the last pane with focus: true
    let focus_pane_index = pane_entries
        .iter()
        .enumerate()
        .rfind(|(_, e)| e.focus)
        .map(|(i, _)| i);

    if let Some(idx) = focus_pane_index {
        let pane_target = format!("{pane_prefix}{}", idx + 1);
        commands.push(TmuxCommand::new(&["select-pane", "-t", &pane_target]));
    }

    // Unset session-level env vars after all panes have been created to
    // prevent leaking into subsequent windows in the same tmux session.
    commands.extend(unset_environment_commands(session, env_vars));

    commands
}

/// Recursively collect split commands and pane entries from the layout tree.
///
/// `pane_offset` is the 1-based index of the first pane that will be created
/// for this subtree. It is used to emit `-t` targeting on `split-window`
/// commands so that nested splits target the correct pane regardless of which
/// pane is currently focused.
fn collect_layout(
    node: &LayoutNode,
    commands: &mut Vec<TmuxCommand>,
    panes: &mut Vec<PaneEntry>,
    cwd: &str,
    pane_offset: usize,
    pane_prefix: &str,
) {
    match node {
        LayoutNode::Pane(pane) => {
            panes.push(PaneEntry {
                command: pane.command.clone(),
                focus: pane.focus,
            });
        }
        LayoutNode::Split(split) => {
            // Process first child (uses current pane at pane_offset)
            collect_layout(&split.first, commands, panes, cwd, pane_offset, pane_prefix);

            let first_pane_count = count_panes(&split.first);

            // Split the first child's root pane to create second child's pane.
            // Use -t to explicitly target the pane, because recursing into
            // first may have created additional panes that changed the active pane.
            let direction_flag = match split.direction {
                SplitDirection::Horizontal => "-h",
                SplitDirection::Vertical => "-v",
            };
            let target = format!("{pane_prefix}{pane_offset}");
            commands.push(TmuxCommand::new(&[
                "split-window",
                direction_flag,
                "-t",
                &target,
                "-c",
                cwd,
            ]));

            // Process second child
            let second_offset = pane_offset + first_pane_count;
            collect_layout(
                &split.second,
                commands,
                panes,
                cwd,
                second_offset,
                pane_prefix,
            );
        }
    }
}

/// Count the total number of leaf panes in a layout subtree.
fn count_panes(node: &LayoutNode) -> usize {
    match node {
        LayoutNode::Pane(_) => 1,
        LayoutNode::Split(split) => count_panes(&split.first) + count_panes(&split.second),
    }
}

/// Inputs for `build_split_pane_setup_commands`: the `set-environment` +
/// `split-window` commands that must run before the new pane's id is known
/// (mirrors `execute_background_layout`'s new-window capture).
struct SplitPaneSetupSpec<'a> {
    session: &'a str,
    target_pane: &'a str,
    cwd: &'a str,
    env_vars: &'a [(&'a str, &'a str)],
    background: bool,
}

/// Builds the `set-environment` (if any) + `split-window` command sequence.
/// Always splits horizontally (side-by-side): this path has no layout
/// config, unlike `--worktree`'s `config.wm.layout`.
fn build_split_pane_setup_commands(spec: SplitPaneSetupSpec) -> Vec<TmuxCommand> {
    let mut commands = set_environment_commands(spec.session, spec.env_vars);
    let mut split_args = vec!["split-window"];
    if spec.background {
        split_args.push("-d");
    }
    split_args.extend(["-h", "-t", spec.target_pane, "-c", spec.cwd]);
    commands.push(TmuxCommand::new(&split_args));
    commands
}

/// Tmux session config shared by both `build_layout` and `split_pane`.
pub struct TmuxSessionSpec<'a> {
    /// Target tmux session name.
    pub session: &'a str,
    /// Working directory for the new pane(s).
    pub cwd: &'a str,
    /// Inserted right after the program name in `engine` pane commands.
    pub model: Option<&'a str>,
    /// Passed to `engine` panes as `--effort` (claude) or
    /// `-c model_reasoning_effort=...` (codex).
    pub reasoning_effort: Option<ReasoningEffort>,
    /// Written to a temp file and passed to `engine` pane commands; the temp
    /// file is read and deleted by the shell command at execution time.
    pub prompt: Option<&'a str>,
    /// The agent CLI this session is for. `model` and `prompt` apply only to
    /// panes running it.
    pub engine: Engine,
    /// Set as tmux session-level environment variables so all panes in the
    /// window inherit them.
    pub env_vars: &'a [(&'a str, &'a str)],
    /// When true, avoids stealing focus from the currently attached client.
    pub background: bool,
}

/// Inputs for `build_layout`, grouped to keep its argument count in check.
pub struct LayoutSpec<'a> {
    pub common: TmuxSessionSpec<'a>,
    pub window_name: &'a str,
    pub layout: &'a LayoutNode,
    /// Forwarded to `build_layout_commands` (see `LayoutCommandsSpec`).
    pub restore_automatic_rename: bool,
}

fn layout_agent_commands(layout: &LayoutNode, cwd: &str, engine: Engine) -> Vec<(usize, String)> {
    let mut commands = Vec::new();
    let mut pane_entries = Vec::new();
    collect_layout(layout, &mut commands, &mut pane_entries, cwd, 1, "");

    if !pane_entries
        .iter()
        .any(|entry| is_engine_command(&entry.command, engine))
    {
        for entry in &mut pane_entries {
            entry.command = retarget_agent_command(&entry.command, engine);
        }
    }

    pane_entries
        .into_iter()
        .enumerate()
        .filter(|(_, entry)| is_engine_command(&entry.command, engine))
        .map(|(index, entry)| (index + 1, entry.command))
        .collect()
}

fn remove_prompt_file(path: &Path) -> anyhow::Result<()> {
    std::fs::remove_file(path)
        .with_context(|| format!("Failed to remove prompt file {}", path.display()))
}

/// Build and execute tmux layout from a LayoutNode tree.
///
/// Creates a new tmux window and configures panes according to the layout.
pub fn build_layout(spec: LayoutSpec) -> anyhow::Result<AgentLaunchRoute> {
    let LayoutSpec {
        common,
        window_name,
        layout,
        restore_automatic_rename,
    } = spec;
    let TmuxSessionSpec {
        session,
        cwd,
        model,
        reasoning_effort,
        prompt,
        engine,
        env_vars,
        background,
    } = common;

    let agent_commands = layout_agent_commands(layout, cwd, engine);
    let codex_launch = CodexLaunch::prepare(engine, prompt, agent_commands.len());

    let prompt_file = prompt.map(write_prompt_file).transpose()?;
    let prompt_path = prompt_file.as_deref();
    let daemon_launch = codex_launch.uses_daemon();
    let commands = build_layout_commands(LayoutCommandsSpec {
        session,
        cwd,
        window_name,
        layout,
        model,
        reasoning_effort: if daemon_launch {
            None
        } else {
            reasoning_effort
        },
        prompt_file: if daemon_launch { None } else { prompt_path },
        engine,
        env_vars,
        background,
        restore_automatic_rename,
    });

    let window_id = if background || daemon_launch {
        execute_layout(&commands, session, window_name, background)?
    } else {
        execute_commands(&commands)?;
        String::new()
    };

    let route = codex_launch.finish(Path::new(cwd), prompt, reasoning_effort);
    match route {
        AgentLaunchRoute::CodexDaemon => {
            if let Some(path) = prompt_path {
                remove_prompt_file(path)?;
            }
            Ok(AgentLaunchRoute::CodexDaemon)
        }
        AgentLaunchRoute::CodexArgvFallback { reason } if daemon_launch => {
            let (pane_index, command) = agent_commands
                .first()
                .context("Codex fallback pane disappeared from the layout")?;
            let fallback_command =
                apply_prompt_if_agent(command, engine, model, reasoning_effort, prompt_path, true);
            let pane_id = format!("{window_id}.{pane_index}");
            super::respawn_pane(&pane_id, &fallback_command).with_context(|| {
                format!("Codex daemon launch failed ({reason}); argv fallback also failed")
            })?;
            Ok(AgentLaunchRoute::CodexArgvFallback { reason })
        }
        route => Ok(route),
    }
}

/// Inputs for `split_pane`.
pub struct SplitSpec<'a> {
    pub common: TmuxSessionSpec<'a>,
    pub target_pane: &'a str,
    pub command: &'a str,
}

pub struct SplitResult {
    pub pane_id: String,
    pub route: AgentLaunchRoute,
}

/// Splits `target_pane` into a new pane within the same window and starts
/// `command` there (typically `claude` or `codex`). Returns the new pane's id.
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

    let codex_launch = CodexLaunch::prepare(engine, prompt, 1);
    let prompt_file = prompt.map(write_prompt_file).transpose()?;
    let daemon_launch = codex_launch.uses_daemon();
    let cmd = apply_prompt_if_agent(
        command,
        engine,
        model,
        if daemon_launch {
            None
        } else {
            reasoning_effort
        },
        if daemon_launch {
            None
        } else {
            prompt_file.as_deref()
        },
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
    let new_pane_id = super::run_tmux_output(&capture_args)?;

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
            let _ = super::run_tmux(&["set-environment", "-u", "-t", session, key]);
        }
    })?;

    let route = match codex_launch.finish(Path::new(cwd), prompt, reasoning_effort) {
        AgentLaunchRoute::CodexDaemon => {
            if let Some(path) = prompt_file.as_deref() {
                remove_prompt_file(path)?;
            }
            AgentLaunchRoute::CodexDaemon
        }
        AgentLaunchRoute::CodexArgvFallback { reason } if daemon_launch => {
            let fallback_command = apply_prompt_if_agent(
                command,
                engine,
                model,
                reasoning_effort,
                prompt_file.as_deref(),
                true,
            );
            super::respawn_pane(&new_pane_id, &fallback_command).with_context(|| {
                format!("Codex daemon launch failed ({reason}); argv fallback also failed")
            })?;
            AgentLaunchRoute::CodexArgvFallback { reason }
        }
        route => route,
    };

    Ok(SplitResult {
        pane_id: new_pane_id,
        route,
    })
}

/// Flattens a sequence of `TmuxCommand` into a single arg list joined by `;`,
/// the wire format tmux uses to chain multiple commands in one invocation.
fn flatten_commands(commands: &[TmuxCommand]) -> Vec<&str> {
    let mut args: Vec<&str> = Vec::new();
    for (i, cmd) in commands.iter().enumerate() {
        if i > 0 {
            args.push(";");
        }
        for arg in &cmd.args {
            args.push(arg);
        }
    }
    args
}

/// Executes a layout and captures the stable window ID for launch fallback.
/// Background layouts run `new-window` first, then rewrite the remaining
/// `{session}:={window_name}.` pane targets to `{window_id}.` before running
/// them. Foreground layouts keep the existing single tmux command sequence.
///
/// This indirection exists because tmux's target parser splits on `.` to
/// separate window from pane, so a session-qualified target is ambiguous when
/// the window name itself contains a `.` (e.g. a branch name like
/// `copier-update/v0.8.13` becomes window name `copier-update-v0.8.13`).
/// Window IDs (e.g. `@42`) never contain `.`, so targeting by ID sidesteps the
/// ambiguity entirely.
fn execute_layout(
    commands: &[TmuxCommand],
    session: &str,
    window_name: &str,
    background: bool,
) -> super::Result<String> {
    let new_window_idx = find_new_window_index(commands).ok_or_else(|| {
        super::TmuxError::Internal("new-window command not found in layout".to_string())
    })?;

    if !background {
        let mut commands = commands.to_vec();
        commands[new_window_idx].args.extend([
            "-P".to_string(),
            "-F".to_string(),
            "#{window_id}".to_string(),
        ]);
        return super::run_tmux_output(&flatten_commands(&commands));
    }

    let setup = &commands[..=new_window_idx];
    let rest = &commands[new_window_idx + 1..];

    let mut setup_args = flatten_commands(setup);
    setup_args.extend(["-P", "-F", "#{window_id}"]);
    let window_id = super::run_tmux_output(&setup_args)?;

    let old_prefix = background_pane_prefix(session, window_name);
    let new_prefix = format!("{window_id}.");
    execute_commands(&rewrite_pane_targets(rest, &old_prefix, &new_prefix))?;
    Ok(window_id)
}

/// Finds the index of the `new-window` command in a layout's command list.
fn find_new_window_index(commands: &[TmuxCommand]) -> Option<usize> {
    commands
        .iter()
        .position(|cmd| cmd.args.first().map(String::as_str) == Some("new-window"))
}

/// Rewrites pane-targeting command args from `{old_prefix}{pane}` to
/// `{new_prefix}{pane}`.
fn rewrite_pane_targets(
    commands: &[TmuxCommand],
    old_prefix: &str,
    new_prefix: &str,
) -> Vec<TmuxCommand> {
    commands
        .iter()
        .map(|cmd| TmuxCommand {
            args: cmd
                .args
                .iter()
                .map(|arg| match arg.strip_prefix(old_prefix) {
                    Some(rest) => format!("{new_prefix}{rest}"),
                    None => arg.clone(),
                })
                .collect(),
        })
        .collect()
}

/// Write prompt to a temp file that persists until the shell command reads it.
fn write_prompt_file(prompt: &str) -> anyhow::Result<std::path::PathBuf> {
    use anyhow::Context;

    let prompt_file = tempfile::Builder::new()
        .prefix("claude-prompt-")
        .suffix(".txt")
        .tempfile()
        .context("Failed to create temp file for prompt")?;

    std::fs::write(prompt_file.path(), prompt).context("Failed to write prompt to temp file")?;

    // Keep the temp file so it persists after this function returns.
    // The shell command will delete it after reading.
    prompt_file
        .into_temp_path()
        .keep()
        .context("Failed to persist prompt temp file")
}

/// Execute a sequence of TmuxCommand by chaining them with ";".
fn execute_commands(commands: &[TmuxCommand]) -> super::Result<()> {
    super::run_tmux(&flatten_commands(commands))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shared::config::{PaneConfig, SplitConfig, SplitDirection};
    use crate::shared::env_var::EnvVars;
    use rstest::{fixture, rstest};
    use std::path::PathBuf;

    /// Helper: create a TmuxCommand from a slice of string slices.
    fn cmd(args: &[&str]) -> TmuxCommand {
        TmuxCommand::new(args)
    }

    /// Helper: create an unfocused leaf pane running `command`.
    fn pane(command: &str) -> Box<LayoutNode> {
        Box::new(LayoutNode::Pane(PaneConfig {
            command: command.to_string(),
            focus: false,
        }))
    }

    #[test]
    fn layout_agent_commands_retargets_the_default_agent_pane() {
        let layout = LayoutNode::default();

        assert_eq!(
            layout_agent_commands(&layout, "/workspace/project-a", Engine::Codex),
            vec![(2, "codex".to_string())],
        );
    }

    #[test]
    fn layout_agent_commands_preserves_explicit_codex_panes() {
        let layout = LayoutNode::Split(SplitConfig {
            direction: SplitDirection::Horizontal,
            first: pane("codex --search"),
            second: pane("claude"),
        });

        assert_eq!(
            layout_agent_commands(&layout, "/workspace/project-a", Engine::Codex),
            vec![(1, "codex --search".to_string())],
        );
    }

    // =========================================================================
    // build_layout_commands: single pane (no split)
    // =========================================================================

    #[test]
    fn single_pane_layout() {
        let layout = LayoutNode::Pane(PaneConfig {
            command: "nvim".to_string(),
            focus: true,
        });

        let commands = build_layout_commands(LayoutCommandsSpec {
            session: "sess",
            cwd: "/tmp",
            window_name: "editor",
            layout: &layout,
            model: None,
            reasoning_effort: None,
            prompt_file: None,
            engine: Engine::Claude,
            env_vars: &[],
            background: false,
            restore_automatic_rename: false,
        });

        assert_eq!(
            commands,
            vec![
                cmd(&["new-window", "-t", "sess", "-c", "/tmp", "-n", "editor"]),
                cmd(&["select-pane", "-t", "1"]),
                cmd(&["send-keys", "-l", "--", "nvim"]),
                cmd(&["send-keys", "C-m"]),
                cmd(&["select-pane", "-t", "1"]),
            ]
        );
    }

    // =========================================================================
    // build_layout_commands: 2 pane horizontal split
    // =========================================================================

    #[test]
    fn two_pane_horizontal_split() {
        let layout = LayoutNode::Split(SplitConfig {
            direction: SplitDirection::Horizontal,
            first: Box::new(LayoutNode::Pane(PaneConfig {
                command: "nvim".to_string(),
                focus: true,
            })),
            second: Box::new(LayoutNode::Pane(PaneConfig {
                command: "claude".to_string(),
                focus: false,
            })),
        });

        let commands = build_layout_commands(LayoutCommandsSpec {
            session: "sess",
            cwd: "/tmp",
            window_name: "dev",
            layout: &layout,
            model: None,
            reasoning_effort: None,
            prompt_file: None,
            engine: Engine::Claude,
            env_vars: &[],
            background: false,
            restore_automatic_rename: false,
        });

        assert_eq!(
            commands,
            vec![
                cmd(&["new-window", "-t", "sess", "-c", "/tmp", "-n", "dev"]),
                cmd(&["split-window", "-h", "-t", "1", "-c", "/tmp"]),
                cmd(&["select-pane", "-t", "1"]),
                cmd(&["send-keys", "-l", "--", "nvim"]),
                cmd(&["send-keys", "C-m"]),
                cmd(&["select-pane", "-t", "2"]),
                cmd(&["send-keys", "-l", "--", "claude"]),
                cmd(&["send-keys", "C-m"]),
                cmd(&["select-pane", "-t", "1"]),
            ]
        );
    }

    // =========================================================================
    // build_layout_commands: 2 pane vertical split
    // =========================================================================

    #[test]
    fn two_pane_vertical_split() {
        let layout = LayoutNode::Split(SplitConfig {
            direction: SplitDirection::Vertical,
            first: Box::new(LayoutNode::Pane(PaneConfig {
                command: "top".to_string(),
                focus: false,
            })),
            second: Box::new(LayoutNode::Pane(PaneConfig {
                command: "bash".to_string(),
                focus: true,
            })),
        });

        let commands = build_layout_commands(LayoutCommandsSpec {
            session: "sess",
            cwd: "/tmp",
            window_name: "monitor",
            layout: &layout,
            model: None,
            reasoning_effort: None,
            prompt_file: None,
            engine: Engine::Claude,
            env_vars: &[],
            background: false,
            restore_automatic_rename: false,
        });

        assert_eq!(
            commands,
            vec![
                cmd(&["new-window", "-t", "sess", "-c", "/tmp", "-n", "monitor"]),
                cmd(&["split-window", "-v", "-t", "1", "-c", "/tmp"]),
                cmd(&["select-pane", "-t", "1"]),
                cmd(&["send-keys", "-l", "--", "top"]),
                cmd(&["send-keys", "C-m"]),
                cmd(&["select-pane", "-t", "2"]),
                cmd(&["send-keys", "-l", "--", "bash"]),
                cmd(&["send-keys", "C-m"]),
                cmd(&["select-pane", "-t", "2"]),
            ]
        );
    }

    // =========================================================================
    // build_layout_commands: 3 pane nested layout
    // (left: nvim, right-top: claude, right-bottom: bash)
    // =========================================================================

    #[test]
    fn three_pane_nested_layout() {
        let layout = LayoutNode::Split(SplitConfig {
            direction: SplitDirection::Horizontal,
            first: Box::new(LayoutNode::Pane(PaneConfig {
                command: "nvim".to_string(),
                focus: true,
            })),
            second: Box::new(LayoutNode::Split(SplitConfig {
                direction: SplitDirection::Vertical,
                first: Box::new(LayoutNode::Pane(PaneConfig {
                    command: "claude".to_string(),
                    focus: false,
                })),
                second: Box::new(LayoutNode::Pane(PaneConfig {
                    command: "bash".to_string(),
                    focus: false,
                })),
            })),
        });

        let commands = build_layout_commands(LayoutCommandsSpec {
            session: "sess",
            cwd: "/tmp",
            window_name: "dev",
            layout: &layout,
            model: None,
            reasoning_effort: None,
            prompt_file: None,
            engine: Engine::Claude,
            env_vars: &[],
            background: false,
            restore_automatic_rename: false,
        });

        assert_eq!(
            commands,
            vec![
                cmd(&["new-window", "-t", "sess", "-c", "/tmp", "-n", "dev"]),
                cmd(&["split-window", "-h", "-t", "1", "-c", "/tmp"]),
                cmd(&["split-window", "-v", "-t", "2", "-c", "/tmp"]),
                cmd(&["select-pane", "-t", "1"]),
                cmd(&["send-keys", "-l", "--", "nvim"]),
                cmd(&["send-keys", "C-m"]),
                cmd(&["select-pane", "-t", "2"]),
                cmd(&["send-keys", "-l", "--", "claude"]),
                cmd(&["send-keys", "C-m"]),
                cmd(&["select-pane", "-t", "3"]),
                cmd(&["send-keys", "-l", "--", "bash"]),
                cmd(&["send-keys", "C-m"]),
                cmd(&["select-pane", "-t", "1"]),
            ]
        );
    }

    // =========================================================================
    // build_layout_commands: prompt file appended to claude commands
    // =========================================================================

    #[test]
    fn prompt_file_appended_to_claude_commands() {
        let prompt_path = PathBuf::from("/tmp/claude-prompt-test.txt");
        let layout = LayoutNode::Split(SplitConfig {
            direction: SplitDirection::Horizontal,
            first: Box::new(LayoutNode::Pane(PaneConfig {
                command: "nvim".to_string(),
                focus: true,
            })),
            second: Box::new(LayoutNode::Pane(PaneConfig {
                command: "claude".to_string(),
                focus: false,
            })),
        });

        let commands = build_layout_commands(LayoutCommandsSpec {
            session: "sess",
            cwd: "/tmp",
            window_name: "dev",
            layout: &layout,
            model: None,
            reasoning_effort: None,
            prompt_file: Some(&prompt_path),
            engine: Engine::Claude,
            env_vars: &[],
            background: false,
            restore_automatic_rename: false,
        });

        assert_eq!(
            commands,
            vec![
                cmd(&["new-window", "-t", "sess", "-c", "/tmp", "-n", "dev"]),
                cmd(&["split-window", "-h", "-t", "1", "-c", "/tmp"]),
                cmd(&["select-pane", "-t", "1"]),
                cmd(&["send-keys", "-l", "--", "nvim"]),
                cmd(&["send-keys", "C-m"]),
                cmd(&["select-pane", "-t", "2"]),
                cmd(&[
                    "send-keys",
                    "-l",
                    "--",
                    "claude \"$(cat /tmp/claude-prompt-test.txt)\" ; rm /tmp/claude-prompt-test.txt",
                ]),
                cmd(&["send-keys", "C-m"]),
                cmd(&["select-pane", "-t", "1"]),
            ]
        );
    }

    // =========================================================================
    // build_layout_commands: focus control
    // =========================================================================

    #[test]
    fn focus_last_focused_pane_wins() {
        // When multiple panes have focus: true, the last one wins
        let layout = LayoutNode::Split(SplitConfig {
            direction: SplitDirection::Horizontal,
            first: Box::new(LayoutNode::Pane(PaneConfig {
                command: "nvim".to_string(),
                focus: true,
            })),
            second: Box::new(LayoutNode::Pane(PaneConfig {
                command: "claude".to_string(),
                focus: true,
            })),
        });

        let commands = build_layout_commands(LayoutCommandsSpec {
            session: "sess",
            cwd: "/tmp",
            window_name: "dev",
            layout: &layout,
            model: None,
            reasoning_effort: None,
            prompt_file: None,
            engine: Engine::Claude,
            env_vars: &[],
            background: false,
            restore_automatic_rename: false,
        });

        // The last select-pane should target pane 2 (the last focused pane)
        let last_cmd = commands.last().unwrap();
        assert_eq!(last_cmd, &cmd(&["select-pane", "-t", "2"]));
    }

    #[test]
    fn no_focus_pane_omits_final_select() {
        // When no pane has focus: true, no final select-pane is emitted
        let layout = LayoutNode::Split(SplitConfig {
            direction: SplitDirection::Horizontal,
            first: Box::new(LayoutNode::Pane(PaneConfig {
                command: "nvim".to_string(),
                focus: false,
            })),
            second: Box::new(LayoutNode::Pane(PaneConfig {
                command: "bash".to_string(),
                focus: false,
            })),
        });

        let commands = build_layout_commands(LayoutCommandsSpec {
            session: "sess",
            cwd: "/tmp",
            window_name: "dev",
            layout: &layout,
            model: None,
            reasoning_effort: None,
            prompt_file: None,
            engine: Engine::Claude,
            env_vars: &[],
            background: false,
            restore_automatic_rename: false,
        });

        // Last command should be a send-keys C-m, not select-pane for focus
        let last_cmd = commands.last().unwrap();
        assert_eq!(last_cmd, &cmd(&["send-keys", "C-m"]));
    }

    // =========================================================================
    // build_layout_commands: left-nested layout (first child is a split)
    // Regression test for: split-window targeting wrong pane when first child
    // is itself a Split node.
    // =========================================================================

    #[test]
    fn left_nested_layout_targets_correct_pane() {
        // Expected: left-top=a, left-bottom=b, right=c (full height)
        let layout = LayoutNode::Split(SplitConfig {
            direction: SplitDirection::Horizontal,
            first: Box::new(LayoutNode::Split(SplitConfig {
                direction: SplitDirection::Vertical,
                first: Box::new(LayoutNode::Pane(PaneConfig {
                    command: "a".to_string(),
                    focus: true,
                })),
                second: Box::new(LayoutNode::Pane(PaneConfig {
                    command: "b".to_string(),
                    focus: false,
                })),
            })),
            second: Box::new(LayoutNode::Pane(PaneConfig {
                command: "c".to_string(),
                focus: false,
            })),
        });

        let commands = build_layout_commands(LayoutCommandsSpec {
            session: "sess",
            cwd: "/tmp",
            window_name: "dev",
            layout: &layout,
            model: None,
            reasoning_effort: None,
            prompt_file: None,
            engine: Engine::Claude,
            env_vars: &[],
            background: false,
            restore_automatic_rename: false,
        });

        assert_eq!(
            commands,
            vec![
                cmd(&["new-window", "-t", "sess", "-c", "/tmp", "-n", "dev"]),
                // Inner vertical split: splits pane 1 vertically -> panes 1, 2
                cmd(&["split-window", "-v", "-t", "1", "-c", "/tmp"]),
                // Outer horizontal split: splits pane 1 (the root of first subtree)
                // horizontally -> creates pane 3 for "c"
                cmd(&["split-window", "-h", "-t", "1", "-c", "/tmp"]),
                cmd(&["select-pane", "-t", "1"]),
                cmd(&["send-keys", "-l", "--", "a"]),
                cmd(&["send-keys", "C-m"]),
                cmd(&["select-pane", "-t", "2"]),
                cmd(&["send-keys", "-l", "--", "b"]),
                cmd(&["send-keys", "C-m"]),
                cmd(&["select-pane", "-t", "3"]),
                cmd(&["send-keys", "-l", "--", "c"]),
                cmd(&["send-keys", "C-m"]),
                cmd(&["select-pane", "-t", "1"]),
            ]
        );
    }

    /// Two claude panes side by side, shared by tests that verify per-pane
    /// command shaping (prompt file cleanup, --model) applies consistently.
    #[fixture]
    fn two_claude_pane_layout() -> LayoutNode {
        LayoutNode::Split(SplitConfig {
            direction: SplitDirection::Horizontal,
            first: Box::new(LayoutNode::Pane(PaneConfig {
                command: "claude -p agent1".to_string(),
                focus: true,
            })),
            second: Box::new(LayoutNode::Pane(PaneConfig {
                command: "claude -p agent2".to_string(),
                focus: false,
            })),
        })
    }

    // =========================================================================
    // build_layout_commands: multiple claude panes with prompt file
    // Only the last agent pane should delete the temp file.
    // =========================================================================

    #[rstest]
    fn multiple_claude_panes_only_last_deletes_prompt_file(two_claude_pane_layout: LayoutNode) {
        let prompt_path = PathBuf::from("/tmp/prompt.txt");
        let layout = two_claude_pane_layout;

        let commands = build_layout_commands(LayoutCommandsSpec {
            session: "sess",
            cwd: "/tmp",
            window_name: "dev",
            layout: &layout,
            model: None,
            reasoning_effort: None,
            prompt_file: Some(&prompt_path),
            engine: Engine::Claude,
            env_vars: &[],
            background: false,
            restore_automatic_rename: false,
        });

        assert_eq!(
            commands,
            vec![
                cmd(&["new-window", "-t", "sess", "-c", "/tmp", "-n", "dev"]),
                cmd(&["split-window", "-h", "-t", "1", "-c", "/tmp"]),
                cmd(&["select-pane", "-t", "1"]),
                // First claude pane: reads prompt but does NOT delete file
                cmd(&[
                    "send-keys",
                    "-l",
                    "--",
                    "claude -p agent1 \"$(cat /tmp/prompt.txt)\"",
                ]),
                cmd(&["send-keys", "C-m"]),
                cmd(&["select-pane", "-t", "2"]),
                // Last claude pane: reads prompt AND deletes file
                cmd(&[
                    "send-keys",
                    "-l",
                    "--",
                    "claude -p agent2 \"$(cat /tmp/prompt.txt)\" ; rm /tmp/prompt.txt",
                ]),
                cmd(&["send-keys", "C-m"]),
                cmd(&["select-pane", "-t", "1"]),
            ]
        );
    }

    // =========================================================================
    // build_layout_commands: claude-only layout launched for another engine
    // The `claude` pane becomes the engine's CLI and gets model, effort and
    // the prompt; other panes are left as written.
    // =========================================================================

    #[test]
    fn claude_layout_is_retargeted_to_codex_session() {
        let prompt_path = PathBuf::from("/tmp/prompt.txt");
        let layout = LayoutNode::Split(SplitConfig {
            direction: SplitDirection::Horizontal,
            first: pane("claude --dangerously-skip-permissions"),
            second: pane("nvim"),
        });

        let commands = build_layout_commands(LayoutCommandsSpec {
            session: "sess",
            cwd: "/tmp",
            window_name: "dev",
            layout: &layout,
            model: Some("gpt-5"),
            reasoning_effort: Some(ReasoningEffort::Max),
            prompt_file: Some(&prompt_path),
            engine: Engine::Codex,
            env_vars: &[],
            background: false,
            restore_automatic_rename: false,
        });

        assert_eq!(
            commands,
            vec![
                cmd(&["new-window", "-t", "sess", "-c", "/tmp", "-n", "dev"]),
                cmd(&["split-window", "-h", "-t", "1", "-c", "/tmp"]),
                cmd(&["select-pane", "-t", "1"]),
                cmd(&[
                    "send-keys",
                    "-l",
                    "--",
                    "codex --model gpt-5 -c model_reasoning_effort=max \"$(cat /tmp/prompt.txt)\" ; rm /tmp/prompt.txt",
                ]),
                cmd(&["send-keys", "C-m"]),
                cmd(&["select-pane", "-t", "2"]),
                cmd(&["send-keys", "-l", "--", "nvim"]),
                cmd(&["send-keys", "C-m"]),
            ]
        );
    }

    // =========================================================================
    // build_layout_commands: claude and codex panes in one layout
    // Only panes running the session's engine get --model and the prompt, and
    // the last of them deletes the temp file. Other panes (including the other
    // engine's CLI) are left as written.
    // =========================================================================

    #[rstest]
    #[case::claude_session(
        Engine::Claude,
        "opus",
        "claude --model opus \"$(cat /tmp/prompt.txt)\" ; rm /tmp/prompt.txt",
        "codex"
    )]
    #[case::codex_session(
        Engine::Codex,
        "gpt-5",
        "claude",
        "codex --model gpt-5 \"$(cat /tmp/prompt.txt)\" ; rm /tmp/prompt.txt"
    )]
    fn mixed_engine_layout_only_touches_session_engine_panes(
        #[case] engine: Engine,
        #[case] model: &str,
        #[case] expected_claude_pane: &str,
        #[case] expected_codex_pane: &str,
    ) {
        let prompt_path = PathBuf::from("/tmp/prompt.txt");
        let layout = LayoutNode::Split(SplitConfig {
            direction: SplitDirection::Horizontal,
            first: pane("claude"),
            second: Box::new(LayoutNode::Split(SplitConfig {
                direction: SplitDirection::Vertical,
                first: pane("codex"),
                second: pane("nvim"),
            })),
        });

        let commands = build_layout_commands(LayoutCommandsSpec {
            session: "sess",
            cwd: "/tmp",
            window_name: "dev",
            layout: &layout,
            model: Some(model),
            reasoning_effort: None,
            prompt_file: Some(&prompt_path),
            engine,
            env_vars: &[],
            background: false,
            restore_automatic_rename: false,
        });

        assert_eq!(
            commands,
            vec![
                cmd(&["new-window", "-t", "sess", "-c", "/tmp", "-n", "dev"]),
                cmd(&["split-window", "-h", "-t", "1", "-c", "/tmp"]),
                cmd(&["split-window", "-v", "-t", "2", "-c", "/tmp"]),
                cmd(&["select-pane", "-t", "1"]),
                cmd(&["send-keys", "-l", "--", expected_claude_pane]),
                cmd(&["send-keys", "C-m"]),
                cmd(&["select-pane", "-t", "2"]),
                cmd(&["send-keys", "-l", "--", expected_codex_pane]),
                cmd(&["send-keys", "C-m"]),
                cmd(&["select-pane", "-t", "3"]),
                cmd(&["send-keys", "-l", "--", "nvim"]),
                cmd(&["send-keys", "C-m"]),
            ]
        );
    }

    // =========================================================================
    // build_layout_commands: --model applies to every claude pane
    // =========================================================================

    #[rstest]
    fn model_applied_to_all_claude_panes(two_claude_pane_layout: LayoutNode) {
        let layout = two_claude_pane_layout;

        let commands = build_layout_commands(LayoutCommandsSpec {
            session: "sess",
            cwd: "/tmp",
            window_name: "dev",
            layout: &layout,
            model: Some("opus"),
            reasoning_effort: None,
            prompt_file: None,
            engine: Engine::Claude,
            env_vars: &[],
            background: false,
            restore_automatic_rename: false,
        });

        assert_eq!(
            commands,
            vec![
                cmd(&["new-window", "-t", "sess", "-c", "/tmp", "-n", "dev"]),
                cmd(&["split-window", "-h", "-t", "1", "-c", "/tmp"]),
                cmd(&["select-pane", "-t", "1"]),
                cmd(&["send-keys", "-l", "--", "claude --model opus -p agent1"]),
                cmd(&["send-keys", "C-m"]),
                cmd(&["select-pane", "-t", "2"]),
                cmd(&["send-keys", "-l", "--", "claude --model opus -p agent2"]),
                cmd(&["send-keys", "C-m"]),
                cmd(&["select-pane", "-t", "1"]),
            ]
        );
    }

    // =========================================================================
    // build_layout_commands: env_vars inject set-environment before new-window
    // =========================================================================

    #[test]
    fn env_vars_prepended_as_set_environment() {
        let layout = LayoutNode::Pane(PaneConfig {
            command: "claude".to_string(),
            focus: true,
        });

        let label_key = EnvVars::session_label_name();
        let ancestors_key = EnvVars::ancestor_session_ids_name();
        let env_vars = [
            (label_key, "my-label"),
            (ancestors_key, "parent-1,parent-2"),
        ];
        let commands = build_layout_commands(LayoutCommandsSpec {
            session: "sess",
            cwd: "/tmp",
            window_name: "dev",
            layout: &layout,
            model: None,
            reasoning_effort: None,
            prompt_file: None,
            engine: Engine::Claude,
            env_vars: &env_vars,
            background: false,
            restore_automatic_rename: false,
        });

        // set-environment commands should come before new-window
        assert_eq!(
            commands[0],
            cmd(&["set-environment", "-t", "sess", label_key, "my-label"])
        );
        assert_eq!(
            commands[1],
            cmd(&[
                "set-environment",
                "-t",
                "sess",
                ancestors_key,
                "parent-1,parent-2",
            ])
        );
        assert_eq!(
            commands[2],
            cmd(&["new-window", "-t", "sess", "-c", "/tmp", "-n", "dev"])
        );

        // Unset commands should come at the end to prevent leaking
        let n = commands.len();
        assert_eq!(
            commands[n - 2],
            cmd(&["set-environment", "-u", "-t", "sess", label_key])
        );
        assert_eq!(
            commands[n - 1],
            cmd(&["set-environment", "-u", "-t", "sess", ancestors_key])
        );
    }

    #[test]
    fn build_layout_commands_background_detaches_new_window_and_qualifies_pane_targets() {
        let layout = LayoutNode::Pane(PaneConfig {
            command: "claude".to_string(),
            focus: true,
        });

        let commands = build_layout_commands(LayoutCommandsSpec {
            session: "sess",
            cwd: "/tmp",
            window_name: "dev",
            layout: &layout,
            model: None,
            reasoning_effort: None,
            prompt_file: None,
            engine: Engine::Claude,
            env_vars: &[],
            background: true,
            restore_automatic_rename: false,
        });

        // new-window must be detached so the attached client's view does not flip.
        assert_eq!(
            commands[0],
            cmd(&["new-window", "-d", "-t", "sess", "-c", "/tmp", "-n", "dev"])
        );

        // Every pane-targeting command must address the new window explicitly
        // via {session}:={window_name}.{pane}, never the bare pane index.
        let qualified = "sess:=dev.";
        for c in &commands[1..] {
            if matches!(
                c.args.first().map(String::as_str),
                Some("select-pane" | "send-keys" | "split-window")
            ) {
                let target_idx = c
                    .args
                    .iter()
                    .position(|a| a == "-t")
                    .expect("pane-addressing command without -t");
                let target = &c.args[target_idx + 1];
                assert!(
                    target.starts_with(qualified),
                    "expected target `{target}` to start with `{qualified}`",
                );
            }
        }
    }

    #[test]
    fn build_layout_commands_background_with_env_vars_places_new_window_at_expected_index() {
        // Documents that build_layout_commands emits exactly one set-environment
        // command per env_var before new-window, in the background + env_vars
        // combination that `a agent new --worktree --agent` exercises in production.
        let layout = LayoutNode::Pane(PaneConfig {
            command: "claude".to_string(),
            focus: true,
        });
        let env_vars = [("KEY1", "v1"), ("KEY2", "v2")];

        let commands = build_layout_commands(LayoutCommandsSpec {
            session: "sess",
            cwd: "/tmp",
            window_name: "dev",
            layout: &layout,
            model: None,
            reasoning_effort: None,
            prompt_file: None,
            engine: Engine::Claude,
            env_vars: &env_vars,
            background: true,
            restore_automatic_rename: false,
        });

        assert_eq!(commands[env_vars.len()].args[0], "new-window");
    }

    #[test]
    fn build_layout_commands_foreground_keeps_unqualified_targets() {
        let layout = LayoutNode::Pane(PaneConfig {
            command: "claude".to_string(),
            focus: true,
        });

        let commands = build_layout_commands(LayoutCommandsSpec {
            session: "sess",
            cwd: "/tmp",
            window_name: "dev",
            layout: &layout,
            model: None,
            reasoning_effort: None,
            prompt_file: None,
            engine: Engine::Claude,
            env_vars: &[],
            background: false,
            restore_automatic_rename: false,
        });

        // new-window is left attached (no `-d`).
        assert_eq!(
            commands[0],
            cmd(&["new-window", "-t", "sess", "-c", "/tmp", "-n", "dev"])
        );

        // No previous-window restore command is appended.
        for c in &commands {
            assert_ne!(c.args.first().map(String::as_str), Some("select-window"));
        }
    }

    // =========================================================================
    // build_layout_commands: restore_automatic_rename
    // =========================================================================

    #[rstest]
    #[case::disabled(false, vec![
        cmd(&["new-window", "-t", "sess", "-c", "/tmp", "-n", "dev"]),
        cmd(&["select-pane", "-t", "1"]),
        cmd(&["send-keys", "-l", "--", "claude"]),
        cmd(&["send-keys", "C-m"]),
        cmd(&["select-pane", "-t", "1"]),
    ])]
    #[case::enabled(true, vec![
        cmd(&["new-window", "-t", "sess", "-c", "/tmp", "-n", "dev"]),
        cmd(&["set-option", "-w", "-t", "sess:=dev", "automatic-rename", "on"]),
        cmd(&["select-pane", "-t", "1"]),
        cmd(&["send-keys", "-l", "--", "claude"]),
        cmd(&["send-keys", "C-m"]),
        cmd(&["select-pane", "-t", "1"]),
    ])]
    fn restore_automatic_rename_inserts_set_option_right_after_new_window(
        #[case] restore_automatic_rename: bool,
        #[case] expected: Vec<TmuxCommand>,
    ) {
        let layout = LayoutNode::Pane(PaneConfig {
            command: "claude".to_string(),
            focus: true,
        });

        let commands = build_layout_commands(LayoutCommandsSpec {
            session: "sess",
            cwd: "/tmp",
            window_name: "dev",
            layout: &layout,
            model: None,
            reasoning_effort: None,
            prompt_file: None,
            engine: Engine::Claude,
            env_vars: &[],
            background: false,
            restore_automatic_rename,
        });

        assert_eq!(commands, expected);
    }

    #[test]
    fn restore_automatic_rename_lands_outside_background_window_id_capture() {
        // execute_background_layout runs everything up to and including
        // new-window with `-P -F "#{window_id}"` appended to capture the real
        // window ID, so the restore command must come strictly after
        // new-window or it would be swept into that capture invocation
        // instead of running as its own command.
        let layout = LayoutNode::Pane(PaneConfig {
            command: "claude".to_string(),
            focus: true,
        });

        let commands = build_layout_commands(LayoutCommandsSpec {
            session: "sess",
            cwd: "/tmp",
            window_name: "dev",
            layout: &layout,
            model: None,
            reasoning_effort: None,
            prompt_file: None,
            engine: Engine::Claude,
            env_vars: &[],
            background: true,
            restore_automatic_rename: true,
        });

        assert_eq!(
            find_new_window_index(&commands),
            Some(0),
            "new-window must remain the first command"
        );
        assert_eq!(
            commands[1],
            cmd(&[
                "set-option",
                "-w",
                "-t",
                "sess:=dev",
                "automatic-rename",
                "on"
            ])
        );
    }

    // =========================================================================
    // rewrite_pane_targets: swaps the session:=window_name. prefix for a
    // window-ID-based one, used once new-window's real ID is known
    // =========================================================================

    #[rstest]
    #[case::swaps_matching_prefix(
        vec![
            cmd(&["select-pane", "-t", "sess:=copier-update-v0.8.13.1"]),
            cmd(&[
                "send-keys",
                "-t",
                "sess:=copier-update-v0.8.13.1",
                "-l",
                "--",
                "claude",
            ]),
            cmd(&["send-keys", "-t", "sess:=copier-update-v0.8.13.1", "C-m"]),
            cmd(&["select-pane", "-t", "sess:=copier-update-v0.8.13.2"]),
        ],
        "sess:=copier-update-v0.8.13.",
        "@42.",
        vec![
            cmd(&["select-pane", "-t", "@42.1"]),
            cmd(&["send-keys", "-t", "@42.1", "-l", "--", "claude"]),
            cmd(&["send-keys", "-t", "@42.1", "C-m"]),
            cmd(&["select-pane", "-t", "@42.2"]),
        ]
    )]
    #[case::leaves_non_matching_args_untouched(
        vec![cmd(&["set-environment", "-u", "-t", "sess", "MY_VAR"])],
        "sess:=dev.",
        "@42.",
        vec![cmd(&["set-environment", "-u", "-t", "sess", "MY_VAR"])]
    )]
    #[case::leaves_window_level_restore_target_untouched(
        vec![cmd(&["set-option", "-w", "-t", "sess:=dev", "automatic-rename", "on"])],
        "sess:=dev.",
        "@42.",
        vec![cmd(&["set-option", "-w", "-t", "sess:=dev", "automatic-rename", "on"])]
    )]
    fn rewrite_pane_targets_cases(
        #[case] commands: Vec<TmuxCommand>,
        #[case] old_prefix: &str,
        #[case] new_prefix: &str,
        #[case] expected: Vec<TmuxCommand>,
    ) {
        assert_eq!(
            rewrite_pane_targets(&commands, old_prefix, new_prefix),
            expected
        );
    }

    // =========================================================================
    // find_new_window_index: locates new-window regardless of what precedes it
    // =========================================================================

    #[rstest]
    #[case::after_set_environment_commands(
        vec![
            cmd(&["set-environment", "-t", "sess", "KEY", "v"]),
            cmd(&["new-window", "-d", "-t", "sess", "-c", "/tmp", "-n", "dev"]),
            cmd(&["select-pane", "-t", "sess:=dev.1"]),
        ],
        Some(1)
    )]
    #[case::as_first_command(
        vec![cmd(&["new-window", "-t", "sess", "-c", "/tmp", "-n", "dev"])],
        Some(0)
    )]
    #[case::absent(vec![cmd(&["select-pane", "-t", "1"])], None)]
    #[case::empty(vec![], None)]
    fn find_new_window_index_cases(
        #[case] commands: Vec<TmuxCommand>,
        #[case] expected: Option<usize>,
    ) {
        assert_eq!(find_new_window_index(&commands), expected);
    }

    #[test]
    fn execute_layout_errors_when_new_window_missing() {
        let commands = vec![cmd(&["select-pane", "-t", "1"])];

        let result = execute_layout(&commands, "sess", "win", true);

        assert_eq!(
            result.unwrap_err().to_string(),
            "new-window command not found in layout"
        );
    }

    #[test]
    fn empty_env_vars_no_set_environment() {
        let layout = LayoutNode::Pane(PaneConfig {
            command: "claude".to_string(),
            focus: true,
        });

        let commands = build_layout_commands(LayoutCommandsSpec {
            session: "sess",
            cwd: "/tmp",
            window_name: "dev",
            layout: &layout,
            model: None,
            reasoning_effort: None,
            prompt_file: None,
            engine: Engine::Claude,
            env_vars: &[],
            background: false,
            restore_automatic_rename: false,
        });

        // First command should be new-window, not set-environment
        assert_eq!(
            commands[0],
            cmd(&["new-window", "-t", "sess", "-c", "/tmp", "-n", "dev"])
        );
    }

    // =========================================================================
    // build_split_pane_setup_commands
    // =========================================================================

    #[rstest]
    #[case::foreground_no_env(
        false,
        &[],
        vec![cmd(&["split-window", "-h", "-t", "%3", "-c", "/tmp"])]
    )]
    #[case::background_no_env(
        true,
        &[],
        vec![cmd(&["split-window", "-d", "-h", "-t", "%3", "-c", "/tmp"])]
    )]
    #[case::foreground_with_env(
        false,
        &[("KEY1", "v1"), ("KEY2", "v2")],
        vec![
            cmd(&["set-environment", "-t", "sess", "KEY1", "v1"]),
            cmd(&["set-environment", "-t", "sess", "KEY2", "v2"]),
            cmd(&["split-window", "-h", "-t", "%3", "-c", "/tmp"]),
        ]
    )]
    fn build_split_pane_setup_commands_cases(
        #[case] background: bool,
        #[case] env_vars: &[(&str, &str)],
        #[case] expected: Vec<TmuxCommand>,
    ) {
        let commands = build_split_pane_setup_commands(SplitPaneSetupSpec {
            session: "sess",
            target_pane: "%3",
            cwd: "/tmp",
            env_vars,
            background,
        });

        assert_eq!(commands, expected);
    }
}
