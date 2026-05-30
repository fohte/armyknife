use std::path::{Path, PathBuf};

use anyhow::Result;
use clap::Args;

use crate::shared::command;

const HS_BUNDLED_PATH: &str = "/Applications/Hammerspoon.app/Contents/Frameworks/hs/hs";

fn find_hs_path() -> Option<PathBuf> {
    if let Some(p) = command::find_command_path("hs") {
        return Some(p);
    }
    let bundled = PathBuf::from(HS_BUNDLED_PATH);
    bundled.is_file().then_some(bundled)
}

#[derive(Args, Clone, PartialEq, Eq)]
pub struct DoctorArgs {}

/// External tool armyknife may invoke.
struct Tool {
    /// Display name (also used as the install hint package name unless `brew` is set).
    name: &'static str,
    /// Arguments passed to fetch the version (e.g., `["--version"]`, `["-V"]`).
    version_args: &'static [&'static str],
    /// One-line description of what armyknife uses this tool for.
    purpose: &'static str,
    /// Whether the tool is required for core functionality.
    /// macOS-only tools are skipped on other platforms.
    macos_only: bool,
    /// Override the `brew install` package name (defaults to `name`).
    brew: Option<&'static str>,
}

const TOOLS: &[Tool] = &[
    Tool {
        name: "git",
        version_args: &["--version"],
        purpose: "git operations",
        macos_only: false,
        brew: None,
    },
    Tool {
        name: "gh",
        version_args: &["--version"],
        purpose: "GitHub integration",
        macos_only: false,
        brew: None,
    },
    Tool {
        name: "tmux",
        version_args: &["-V"],
        purpose: "session monitor, worktree windows",
        macos_only: false,
        brew: None,
    },
    Tool {
        name: "nvim",
        version_args: &["--version"],
        purpose: "default editor for human-in-the-loop reviews",
        macos_only: false,
        brew: Some("neovim"),
    },
    Tool {
        name: "wezterm",
        version_args: &["--version"],
        purpose: "terminal emulator for review windows",
        macos_only: false,
        brew: Some("--cask wezterm"),
    },
    Tool {
        name: "ghostty",
        version_args: &["--version"],
        purpose: "alternative terminal emulator for review windows",
        macos_only: false,
        brew: Some("--cask ghostty"),
    },
    Tool {
        name: "delta",
        version_args: &["--version"],
        purpose: "diff pager for `a gh pr-review`",
        macos_only: false,
        brew: Some("git-delta"),
    },
    Tool {
        name: "claude",
        version_args: &["--version"],
        purpose: "Claude Code backend for `a name-branch`",
        macos_only: false,
        brew: None,
    },
    Tool {
        name: "opencode",
        version_args: &["--version"],
        purpose: "opencode backend for `a name-branch`",
        macos_only: false,
        brew: None,
    },
];

pub fn run(_args: &DoctorArgs) -> Result<()> {
    let rows: Vec<Row> = TOOLS
        .iter()
        .map(check_tool)
        .chain(check_hammerspoon())
        .collect();

    let name_width = rows.iter().map(|r| r.name.len()).max().unwrap_or(0);

    for row in &rows {
        print_row(row, name_width);
    }

    Ok(())
}

struct Row {
    name: String,
    status: Status,
    purpose: String,
    install_hint: Option<String>,
}

enum Status {
    Found(String),
    FoundNoVersion,
    Missing,
    Skipped(&'static str),
}

fn check_tool(tool: &Tool) -> Row {
    if tool.macos_only && !cfg!(target_os = "macos") {
        return Row {
            name: tool.name.to_string(),
            status: Status::Skipped("macOS only"),
            purpose: tool.purpose.to_string(),
            install_hint: None,
        };
    }

    if !command::is_command_available(tool.name) {
        return Row {
            name: tool.name.to_string(),
            status: Status::Missing,
            purpose: tool.purpose.to_string(),
            install_hint: Some(install_hint(tool)),
        };
    }

    let version = run_version(tool.name, tool.version_args);
    let status = match version {
        Some(v) => Status::Found(v),
        None => Status::FoundNoVersion,
    };

    Row {
        name: tool.name.to_string(),
        status,
        purpose: tool.purpose.to_string(),
        install_hint: None,
    }
}

fn check_hammerspoon() -> Option<Row> {
    if !cfg!(target_os = "macos") {
        return Some(Row {
            name: "hammerspoon".to_string(),
            status: Status::Skipped("macOS only"),
            purpose: "desktop notifications".to_string(),
            install_hint: None,
        });
    }

    match find_hs_path() {
        Some(path) => {
            let version = run_version_at(&path, &["-c", "print(hs.processInfo.version)"]);
            let status = match version {
                Some(v) => Status::Found(v),
                None => Status::FoundNoVersion,
            };
            Some(Row {
                name: "hammerspoon".to_string(),
                status,
                purpose: "desktop notifications".to_string(),
                install_hint: None,
            })
        }
        None => Some(Row {
            name: "hammerspoon".to_string(),
            status: Status::Missing,
            purpose: "desktop notifications".to_string(),
            install_hint: Some("brew install --cask hammerspoon".to_string()),
        }),
    }
}

fn install_hint(tool: &Tool) -> String {
    let pkg = tool.brew.unwrap_or(tool.name);
    format!("brew install {pkg}")
}

fn run_version(program: &str, args: &[&str]) -> Option<String> {
    let output = command::new(program).args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    extract_version(&String::from_utf8_lossy(&output.stdout))
}

fn run_version_at(program: &Path, args: &[&str]) -> Option<String> {
    let output = std::process::Command::new(program)
        .args(args)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    extract_version(&String::from_utf8_lossy(&output.stdout))
}

/// Pulls a human-readable version string out of typical `--version` output.
///
/// Strategies in order: first line stripped of common prefixes ("git version ",
/// "tmux ", "gh version "); otherwise the first whitespace-separated token that
/// starts with a digit; otherwise the raw first line.
fn extract_version(output: &str) -> Option<String> {
    let line = output.lines().next()?.trim();
    if line.is_empty() {
        return None;
    }

    for prefix in ["git version ", "gh version ", "tmux ", "wezterm "] {
        if let Some(rest) = line.strip_prefix(prefix) {
            return Some(rest.split_whitespace().next()?.to_string());
        }
    }

    if let Some(tok) = line.split_whitespace().find(|t| {
        t.trim_start_matches('v')
            .starts_with(|c: char| c.is_ascii_digit())
    }) {
        return Some(tok.trim_start_matches('v').to_string());
    }

    Some(line.to_string())
}

fn print_row(row: &Row, name_width: usize) {
    let (icon, detail) = match &row.status {
        Status::Found(v) => ("✅", v.clone()),
        Status::FoundNoVersion => ("✅", "found".to_string()),
        Status::Missing => ("❌", "not found".to_string()),
        Status::Skipped(reason) => ("➖", format!("skipped ({reason})")),
    };

    println!(
        "{icon} {name:<width$}  {detail}  -- {purpose}",
        name = row.name,
        width = name_width,
        detail = detail,
        purpose = row.purpose,
    );

    if let Some(hint) = &row.install_hint {
        println!("   {:width$}  install: {hint}", "", width = name_width);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[rstest]
    #[case::git("git version 2.50.0", Some("2.50.0"))]
    #[case::gh("gh version 2.60.0 (2024-01-01)", Some("2.60.0"))]
    #[case::tmux("tmux 3.5", Some("3.5"))]
    #[case::nvim("NVIM v0.10.2", Some("0.10.2"))]
    #[case::delta("delta 0.18.2", Some("0.18.2"))]
    #[case::wezterm("wezterm 20240203-110809-5046fc22", Some("20240203-110809-5046fc22"))]
    #[case::no_digit("hello world", Some("hello world"))]
    #[case::empty("", None)]
    fn extract_version_cases(#[case] input: &str, #[case] expected: Option<&str>) {
        assert_eq!(extract_version(input).as_deref(), expected);
    }

    #[test]
    fn install_hint_uses_brew_override() {
        let tool = Tool {
            name: "nvim",
            version_args: &["--version"],
            purpose: "",
            macos_only: false,
            brew: Some("neovim"),
        };
        assert_eq!(install_hint(&tool), "brew install neovim");
    }

    #[test]
    fn install_hint_defaults_to_name() {
        let tool = Tool {
            name: "git",
            version_args: &["--version"],
            purpose: "",
            macos_only: false,
            brew: None,
        };
        assert_eq!(install_hint(&tool), "brew install git");
    }
}
