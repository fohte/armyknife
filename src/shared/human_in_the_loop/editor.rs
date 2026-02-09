use std::ffi::{OsStr, OsString};
use std::path::Path;
use std::process::{Command, ExitStatus};

use crate::shared::config::Terminal;

/// Options for launching a terminal window.
pub struct LaunchOptions {
    pub window_title: String,
    pub window_cols: u32,
    pub window_rows: u32,
}

impl Default for LaunchOptions {
    fn default() -> Self {
        Self {
            window_title: "Editor".to_string(),
            window_cols: 120,
            window_rows: 40,
        }
    }
}

/// Launch an editor to edit a file.
///
/// The editor command is taken from config (default: "nvim").
/// If the editor is nvim and `window_title` is provided, sets the titlestring option.
/// Blocks until the user closes the editor.
pub fn run_editor(
    editor_command: &str,
    file_path: &Path,
    window_title: Option<&str>,
) -> std::io::Result<ExitStatus> {
    let mut cmd = Command::new(editor_command);

    // Apply nvim-specific titlestring option
    if (editor_command == "nvim" || editor_command.ends_with("/nvim"))
        && let Some(title) = window_title
    {
        let escaped_title = title.replace('\'', "''");
        cmd.args(["-c", &format!("let &titlestring = '{escaped_title}'")]);
    }

    cmd.arg(file_path).status()
}

/// Launch a terminal emulator to run the specified command.
///
/// Dispatches to WezTerm or Ghostty based on the `Terminal` enum,
/// using terminal-specific options for window size and title.
pub fn launch_terminal(
    terminal: &Terminal,
    options: &LaunchOptions,
    command: impl AsRef<OsStr>,
    args: &[OsString],
) -> std::io::Result<ExitStatus> {
    match terminal {
        Terminal::Wezterm => launch_wezterm(options, command, args),
        Terminal::Ghostty => launch_ghostty(options, command, args),
    }
}

/// Launch WezTerm with WezTerm-specific options.
fn launch_wezterm(
    options: &LaunchOptions,
    command: impl AsRef<OsStr>,
    args: &[OsString],
) -> std::io::Result<ExitStatus> {
    let base_command = wezterm_base_command();

    let cols_config = format!("initial_cols={}", options.window_cols);
    let rows_config = format!("initial_rows={}", options.window_rows);

    let mut cmd = Command::new(&base_command[0]);
    cmd.args(&base_command[1..]);
    cmd.args([
        "--config",
        "window_decorations=\"TITLE | RESIZE\"",
        "--config",
        &cols_config,
        "--config",
        &rows_config,
        "start",
        "--class",
        &options.window_title,
        "--",
    ]);
    cmd.arg(command);
    cmd.args(args);
    cmd.status()
}

/// Returns the platform-specific base command for launching WezTerm.
fn wezterm_base_command() -> Vec<String> {
    if cfg!(target_os = "macos") {
        vec![
            "open".to_string(),
            "-n".to_string(),
            "-a".to_string(),
            "WezTerm".to_string(),
            "--args".to_string(),
        ]
    } else {
        vec!["wezterm".to_string()]
    }
}

/// Launch Ghostty with Ghostty-specific options.
fn launch_ghostty(
    options: &LaunchOptions,
    command: impl AsRef<OsStr>,
    args: &[OsString],
) -> std::io::Result<ExitStatus> {
    let base_command = ghostty_base_command();

    let width_flag = format!("--window-width={}", options.window_cols);
    let height_flag = format!("--window-height={}", options.window_rows);
    let title_flag = format!("--title={}", options.window_title);

    let mut cmd = Command::new(&base_command[0]);
    cmd.args(&base_command[1..]);
    cmd.args([&width_flag, &height_flag, &title_flag, "-e"]);
    cmd.arg(command);
    cmd.args(args);
    cmd.status()
}

/// Returns the platform-specific base command for launching Ghostty.
fn ghostty_base_command() -> Vec<String> {
    if cfg!(target_os = "macos") {
        vec![
            "open".to_string(),
            "-na".to_string(),
            "Ghostty".to_string(),
            "--args".to_string(),
        ]
    } else {
        vec!["ghostty".to_string()]
    }
}
