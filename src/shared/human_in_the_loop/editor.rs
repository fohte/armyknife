use std::ffi::{OsStr, OsString};
use std::path::Path;
use std::process::{Command, ExitStatus};

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
/// If `terminal_command` targets WezTerm, uses WezTerm-specific options
/// (window size, decorations, class). Otherwise, constructs a generic
/// command from `terminal_command` prefix + command + args.
pub fn launch_terminal(
    terminal_command: &[String],
    options: &LaunchOptions,
    command: impl AsRef<OsStr>,
    args: &[OsString],
) -> std::io::Result<ExitStatus> {
    if is_wezterm_command(terminal_command) {
        launch_wezterm(terminal_command, options, command, args)
    } else if let Some((program, base_args)) = terminal_command.split_first() {
        let mut cmd = Command::new(program);
        cmd.args(base_args);
        cmd.arg(command);
        cmd.args(args);
        cmd.status()
    } else {
        // Empty terminal_command, fallback to default WezTerm behavior
        launch_wezterm(&default_wezterm_command(), options, command, args)
    }
}

/// Launch WezTerm with WezTerm-specific options.
fn launch_wezterm(
    terminal_command: &[String],
    options: &LaunchOptions,
    command: impl AsRef<OsStr>,
    args: &[OsString],
) -> std::io::Result<ExitStatus> {
    let fallback_program = default_wezterm_program();
    let (program, base_args) = terminal_command
        .split_first()
        .unwrap_or((&fallback_program, &[]));

    let cols_config = format!("initial_cols={}", options.window_cols);
    let rows_config = format!("initial_rows={}", options.window_rows);

    let mut cmd = Command::new(program);
    cmd.args(base_args);
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

fn default_wezterm_program() -> String {
    "wezterm".to_string()
}

fn default_wezterm_command() -> Vec<String> {
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

/// Check if the terminal command targets WezTerm.
fn is_wezterm_command(terminal_command: &[String]) -> bool {
    terminal_command.first().is_some_and(|first| {
        first == "wezterm"
            || first.ends_with("/wezterm")
            || (cfg!(target_os = "macos")
                && terminal_command
                    .iter()
                    .any(|arg| arg == "WezTerm" || arg == "wezterm"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    fn s(val: &str) -> String {
        val.to_string()
    }

    #[rstest]
    #[case::bare_wezterm(&[s("wezterm")], true)]
    #[case::wezterm_with_path(&[s("/usr/bin/wezterm")], true)]
    #[case::alacritty(&[s("alacritty"), s("-e")], false)]
    #[case::kitty(&[s("kitty")], false)]
    #[case::empty(&[], false)]
    fn test_is_wezterm_command(#[case] terminal_command: &[String], #[case] expected: bool) {
        assert_eq!(is_wezterm_command(terminal_command), expected);
    }

    #[cfg(target_os = "macos")]
    #[rstest]
    #[case::macos_open_wezterm(&[s("open"), s("-n"), s("-a"), s("WezTerm"), s("--args")], true)]
    #[case::macos_open_other(&[s("open"), s("-n"), s("-a"), s("Alacritty"), s("--args")], false)]
    fn test_is_wezterm_command_macos(#[case] terminal_command: &[String], #[case] expected: bool) {
        assert_eq!(is_wezterm_command(terminal_command), expected);
    }
}
