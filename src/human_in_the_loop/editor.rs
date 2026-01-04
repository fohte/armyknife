use std::ffi::OsString;
use std::path::Path;
use std::process::{Command, ExitStatus};

/// Options for launching WezTerm.
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

/// Launch Neovim to edit a file.
///
/// Blocks until the user closes Neovim.
pub fn run_neovim(file_path: &Path) -> std::io::Result<ExitStatus> {
    Command::new("nvim").arg(file_path).status()
}

#[cfg(target_os = "macos")]
pub fn launch_wezterm(
    options: &LaunchOptions,
    exe_path: &Path,
    args: &[OsString],
) -> std::io::Result<ExitStatus> {
    let cols_config = format!("initial_cols={}", options.window_cols);
    let rows_config = format!("initial_rows={}", options.window_rows);

    let mut cmd = Command::new("open");
    cmd.args([
        "-n",
        "-a",
        "WezTerm",
        "--args",
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
    cmd.arg(exe_path);
    cmd.args(args);
    cmd.status()
}

#[cfg(not(target_os = "macos"))]
pub fn launch_wezterm(
    options: &LaunchOptions,
    exe_path: &Path,
    args: &[OsString],
) -> std::io::Result<ExitStatus> {
    let cols_config = format!("initial_cols={}", options.window_cols);
    let rows_config = format!("initial_rows={}", options.window_rows);

    let mut cmd = Command::new("wezterm");
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
    cmd.arg(exe_path);
    cmd.args(args);
    cmd.status()
}
