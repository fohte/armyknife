use std::ffi::{OsStr, OsString};
use std::io::Write;
use std::path::Path;
use std::process::{Command, ExitStatus};

use indoc::formatdoc;

use crate::shared::config::Terminal;

/// Approximate width of a terminal character cell in pixels.
const APPROX_CHAR_WIDTH_PX: u32 = 8;
/// Approximate height of a terminal character cell in pixels.
const APPROX_CHAR_HEIGHT_PX: u32 = 16;

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

/// Launch Ghostty on macOS via `open -na`.
///
/// Uses `--command` with a wrapper script instead of `-e` to bypass the
/// security permission dialog introduced in Ghostty v1.2.0 (GHSA-q9fg-cpmh-c78x).
/// The `-e` flag triggers an "Allow Ghostty to execute ...?" dialog on every
/// invocation, leaks arguments into existing tmux sessions, and temporarily
/// resizes tmux panes. The `--command` flag treats the command as a Ghostty
/// config setting rather than an external execution request, avoiding all of
/// these issues.
fn launch_ghostty_macos(
    options: &LaunchOptions,
    command: impl AsRef<OsStr>,
    args: &[OsString],
) -> std::io::Result<ExitStatus> {
    let wrapper_path = create_ghostty_wrapper_script(command, args)?;

    let width_flag = format!("--window-width={}", options.window_cols);
    let height_flag = format!("--window-height={}", options.window_rows);
    let title_flag = format!("--title={}", options.window_title);
    let command_flag = format!("--command={}", wrapper_path.display());

    let mut ghostty_args = vec![width_flag, height_flag, title_flag, command_flag];

    // Center the window on screen if we can determine the display resolution
    if let Some((pos_x, pos_y)) =
        compute_centered_position(options.window_cols, options.window_rows)
    {
        ghostty_args.push(format!("--window-position-x={pos_x}"));
        ghostty_args.push(format!("--window-position-y={pos_y}"));
    }

    let mut cmd = Command::new("open");
    // -n: new instance, -a: application name
    // NOTE: -W (wait) is intentionally omitted because it waits for the entire
    // Ghostty *application* to exit, not just the spawned window, blocking
    // indefinitely when another Ghostty instance is already running.
    cmd.args(["-na", "Ghostty", "--args"]);
    cmd.args(&ghostty_args);
    cmd.status()
}

/// Create a temporary shell script that executes the given command with arguments.
///
/// The script is persisted to disk (not auto-deleted) because `open -na` returns
/// immediately before Ghostty reads it. The script removes itself after execution.
fn create_ghostty_wrapper_script(
    command: impl AsRef<OsStr>,
    args: &[OsString],
) -> std::io::Result<std::path::PathBuf> {
    let wrapper = tempfile::Builder::new()
        .prefix("armyknife-ghostty-")
        .suffix(".sh")
        .tempfile()?;

    // Persist so the file survives after this function returns
    let (mut file, path) = wrapper.keep().map_err(|e| e.error)?;

    let command_str = command.as_ref().to_string_lossy();

    let quoted_cmd = shlex::try_quote(&command_str)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;

    // Self-cleanup: the script removes itself via a trap after the command exits.
    // `exec` is not used because it replaces the shell process and would discard
    // the trap handler. The path is stored in a variable to avoid quoting issues
    // inside the trap string.
    let self_path = path.display().to_string();
    let quoted_self = shlex::try_quote(&self_path)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
    let mut script = formatdoc! {"
        #!/bin/bash
        _SELF={quoted_self}
        trap 'rm -f \"$_SELF\"' EXIT
        {quoted_cmd}"};
    for arg in args {
        let arg_str = arg.to_string_lossy();
        let quoted_arg = shlex::try_quote(&arg_str)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
        script.push(' ');
        script.push_str(&quoted_arg);
    }
    script.push('\n');

    file.write_all(script.as_bytes())?;
    file.flush()?;

    // Make the wrapper script executable
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let metadata = file.metadata()?;
        let mut perms = metadata.permissions();
        perms.set_mode(0o755);
        file.set_permissions(perms)?;
    }

    Ok(path)
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
        let window_w = cols * APPROX_CHAR_WIDTH_PX;
        let window_h = rows * APPROX_CHAR_HEIGHT_PX;

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
    #[case::simple_command("/usr/bin/echo", &[], "/usr/bin/echo\n")]
    #[case::command_with_args(
        "/usr/bin/env",
        &["bash", "-c", "echo hello"],
        "/usr/bin/env bash -c 'echo hello'\n"
    )]
    #[case::args_with_special_chars(
        "/bin/bash",
        &["-c", "echo 'it works' && exit"],
        "/bin/bash -c \"echo 'it works' && exit\"\n"
    )]
    fn test_create_ghostty_wrapper_script(
        #[case] command: &str,
        #[case] args: &[&str],
        #[case] expected_cmd_line: &str,
    ) {
        let os_args: Vec<OsString> = args.iter().map(OsString::from).collect();
        let wrapper_path = create_ghostty_wrapper_script(command, &os_args).unwrap();

        let mut content = String::new();
        let mut file = std::fs::File::open(&wrapper_path).unwrap();
        file.read_to_string(&mut content).unwrap();

        // Verify shebang, self-cleanup variable, trap, and command line
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines[0], "#!/bin/bash");
        assert!(lines[1].starts_with("_SELF="));
        assert_eq!(lines[2], r#"trap 'rm -f "$_SELF"' EXIT"#);
        assert_eq!(format!("{}\n", lines[3]), expected_cmd_line);

        // Verify the script is executable
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = file.metadata().unwrap().permissions().mode();
            assert_eq!(mode & 0o755, 0o755);
        }

        // Clean up persisted file
        std::fs::remove_file(&wrapper_path).unwrap();
    }
}
