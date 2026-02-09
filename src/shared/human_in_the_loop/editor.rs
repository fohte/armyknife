use std::ffi::{OsStr, OsString};
use std::io::Write;
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
///
/// On macOS, uses a wrapper script to avoid `open` mishandling multiple `-e`
/// arguments (which causes permission dialogs, window size issues, and tmux
/// interference). On Linux, invokes `ghostty` directly.
fn launch_ghostty(
    options: &LaunchOptions,
    command: impl AsRef<OsStr>,
    args: &[OsString],
) -> std::io::Result<ExitStatus> {
    if cfg!(target_os = "macos") {
        launch_ghostty_macos(options, command, args)
    } else {
        launch_ghostty_linux(options, command, args)
    }
}

/// Launch Ghostty on Linux by invoking the binary directly.
fn launch_ghostty_linux(
    options: &LaunchOptions,
    command: impl AsRef<OsStr>,
    args: &[OsString],
) -> std::io::Result<ExitStatus> {
    let width_flag = format!("--window-width={}", options.window_cols);
    let height_flag = format!("--window-height={}", options.window_rows);
    let title_flag = format!("--title={}", options.window_title);

    let mut cmd = Command::new("ghostty");
    cmd.args([&width_flag, &height_flag, &title_flag, "-e"]);
    cmd.arg(command);
    cmd.args(args);
    cmd.status()
}

/// Launch Ghostty on macOS via `open -nWa`.
///
/// Wraps the target command in a temporary shell script so that only a single
/// path is passed after `-e`, avoiding the bug where `open` misroutes multiple
/// arguments and triggers Ghostty's security dialog.
fn launch_ghostty_macos(
    options: &LaunchOptions,
    command: impl AsRef<OsStr>,
    args: &[OsString],
) -> std::io::Result<ExitStatus> {
    let wrapper = create_ghostty_wrapper_script(command, args)?;
    let wrapper_path = wrapper.path().to_path_buf();

    let width_flag = format!("--window-width={}", options.window_cols);
    let height_flag = format!("--window-height={}", options.window_rows);
    let title_flag = format!("--title={}", options.window_title);

    let mut ghostty_args = vec![width_flag, height_flag, title_flag];

    // Center the window on screen if we can determine the display resolution
    if let Some((pos_x, pos_y)) =
        compute_centered_position(options.window_cols, options.window_rows)
    {
        ghostty_args.push(format!("--window-position-x={pos_x}"));
        ghostty_args.push(format!("--window-position-y={pos_y}"));
    }

    ghostty_args.push("-e".to_string());

    let mut cmd = Command::new("open");
    // -n: new instance, -W: wait for exit, -a: application name
    cmd.args(["-nWa", "Ghostty", "--args"]);
    cmd.args(&ghostty_args);
    cmd.arg(&wrapper_path);
    let status = cmd.status();

    // Clean up the wrapper script (best-effort; tempfile also cleans on drop)
    drop(wrapper);

    status
}

/// Create a temporary shell script that executes the given command with arguments.
///
/// The script is marked executable and uses `exec` to replace the shell process
/// so that Ghostty's exit tracking works correctly.
fn create_ghostty_wrapper_script(
    command: impl AsRef<OsStr>,
    args: &[OsString],
) -> std::io::Result<tempfile::NamedTempFile> {
    let mut wrapper = tempfile::Builder::new()
        .prefix("armyknife-ghostty-")
        .suffix(".sh")
        .tempfile()?;

    let command_str = command.as_ref().to_string_lossy();

    let quoted_cmd = shlex::try_quote(&command_str)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
    let mut script = format!("#!/bin/bash\nexec {quoted_cmd}");
    for arg in args {
        let arg_str = arg.to_string_lossy();
        let quoted_arg = shlex::try_quote(&arg_str)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
        script.push(' ');
        script.push_str(&quoted_arg);
    }
    script.push('\n');

    wrapper.write_all(script.as_bytes())?;
    wrapper.flush()?;

    // Make the wrapper script executable
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let metadata = wrapper.as_file().metadata()?;
        let mut perms = metadata.permissions();
        perms.set_mode(0o755);
        wrapper.as_file().set_permissions(perms)?;
    }

    Ok(wrapper)
}

/// Compute window position to center a Ghostty window on the primary display.
///
/// Uses `system_profiler SPDisplaysDataType` to get the screen resolution, and
/// approximates pixel dimensions from character cell counts (8px wide, 16px tall).
/// Returns `None` if the resolution cannot be determined.
fn compute_centered_position(cols: u32, rows: u32) -> Option<(u32, u32)> {
    let output = Command::new("system_profiler")
        .arg("SPDisplaysDataType")
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_display_resolution(&stdout).map(|(screen_w, screen_h)| {
        // Approximate pixel size of the terminal window from cell dimensions
        let window_w = cols * 8;
        let window_h = rows * 16;

        let pos_x = screen_w.saturating_sub(window_w) / 2;
        let pos_y = screen_h.saturating_sub(window_h) / 2;

        (pos_x, pos_y)
    })
}

/// Parse the primary display resolution from `system_profiler SPDisplaysDataType` output.
///
/// Looks for lines like "Resolution: 1920 x 1080" and returns the first match.
fn parse_display_resolution(output: &str) -> Option<(u32, u32)> {
    for line in output.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("Resolution:") {
            // Format: "1920 x 1080 (QHD/FHD - ...)" or "1920 x 1080"
            let parts: Vec<&str> = rest.split_whitespace().collect();
            if parts.len() >= 3
                && parts[1] == "x"
                && let (Ok(w), Ok(h)) = (parts[0].parse::<u32>(), parts[2].parse::<u32>())
            {
                return Some((w, h));
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use std::io::Read;

    use rstest::rstest;

    use super::*;

    #[rstest]
    #[case::bare_resolution(
        "          Resolution: 1920 x 1080\n",
        Some((1920, 1080))
    )]
    #[case::resolution_with_suffix(
        "          Resolution: 3840 x 2160 (4K UHD - 2160p)\n",
        Some((3840, 2160))
    )]
    #[case::retina_display(
        "          Resolution: 3024 x 1964 (Retina)\n",
        Some((3024, 1964))
    )]
    #[case::no_resolution_line("          Vendor: Apple\n", None)]
    #[case::empty("", None)]
    fn test_parse_display_resolution(#[case] input: &str, #[case] expected: Option<(u32, u32)>) {
        assert_eq!(parse_display_resolution(input), expected);
    }

    #[rstest]
    #[case::simple_command(
        "/usr/bin/echo",
        &[],
        "#!/bin/bash\nexec /usr/bin/echo\n"
    )]
    #[case::command_with_args(
        "/usr/bin/env",
        &["bash", "-c", "echo hello"],
        "#!/bin/bash\nexec /usr/bin/env bash -c 'echo hello'\n"
    )]
    #[case::args_with_special_chars(
        "/bin/bash",
        &["-c", "echo 'it works' && exit"],
        "#!/bin/bash\nexec /bin/bash -c \"echo 'it works' && exit\"\n"
    )]
    fn test_create_ghostty_wrapper_script(
        #[case] command: &str,
        #[case] args: &[&str],
        #[case] expected_content: &str,
    ) {
        let os_args: Vec<OsString> = args.iter().map(OsString::from).collect();
        let wrapper = create_ghostty_wrapper_script(command, &os_args).unwrap();

        let mut content = String::new();
        let mut file = std::fs::File::open(wrapper.path()).unwrap();
        file.read_to_string(&mut content).unwrap();

        assert_eq!(content, expected_content);

        // Verify the script is executable
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = file.metadata().unwrap().permissions().mode();
            assert_eq!(mode & 0o755, 0o755);
        }
    }
}
