use std::io::IsTerminal;
use std::process::Output;

use anyhow::Result;
use clap::Args;

use crate::infra::external_tool::ExternalTool;
use crate::shared::config::{Config, Terminal};

#[derive(Args, Clone, PartialEq, Eq)]
pub struct DoctorArgs {}

pub fn run(_args: &DoctorArgs) -> Result<()> {
    let config = crate::shared::config::load_config().unwrap_or_default();
    let tools = selected_tools(&config);
    let rows: Vec<Row> = tools.iter().map(|t| check(*t)).collect();
    let name_width = rows.iter().map(|r| r.name.len()).max().unwrap_or(0);
    let color = use_color();
    for row in &rows {
        print_row(row, name_width, color);
    }
    Ok(())
}

/// Filters [`ExternalTool::ALL`] down to the tools the current config actually
/// uses. Non-selected terminal alternatives, a non-nvim editor, and disabled
/// notifications are dropped so users aren't flagged for tools they will never
/// invoke.
fn selected_tools(config: &Config) -> Vec<ExternalTool> {
    ExternalTool::ALL
        .iter()
        .copied()
        .filter(|tool| match tool {
            ExternalTool::Wezterm => config.editor.terminal == Terminal::Wezterm,
            ExternalTool::Ghostty => config.editor.terminal == Terminal::Ghostty,
            // `editor_command` accepts any executable; doctor only knows how to
            // probe `nvim`, so silently skip when the user picked something else.
            ExternalTool::Nvim => config.editor.editor_command == "nvim",
            ExternalTool::Hammerspoon => config.notification.enabled,
            _ => true,
        })
        .collect()
}

struct Row {
    name: &'static str,
    status: Status,
    purpose: &'static str,
    install_hint: Option<String>,
}

enum Status {
    Found(String),
    FoundNoVersion,
    Missing,
    Skipped(&'static str),
}

fn check(tool: ExternalTool) -> Row {
    let meta = tool.metadata();

    if meta.macos_only && !cfg!(target_os = "macos") {
        return Row {
            name: meta.name,
            status: Status::Skipped("macOS only"),
            purpose: meta.purpose,
            install_hint: None,
        };
    }

    if !tool.is_available() {
        return Row {
            name: meta.name,
            status: Status::Missing,
            purpose: meta.purpose,
            install_hint: Some(tool.install_hint()),
        };
    }

    let status = match run_version(tool) {
        Some(v) => Status::Found(v),
        None => Status::FoundNoVersion,
    };

    Row {
        name: meta.name,
        status,
        purpose: meta.purpose,
        install_hint: None,
    }
}

fn run_version(tool: ExternalTool) -> Option<String> {
    let output = tool
        .command()
        .args(tool.metadata().version_args)
        .output()
        .ok()?;
    parse_version_output(output)
}

fn parse_version_output(output: Output) -> Option<String> {
    if !output.status.success() {
        return None;
    }
    // Some CLIs (older clap-based tools) print `--version` to stderr.
    let bytes = if output.stdout.iter().any(|b| !b.is_ascii_whitespace()) {
        &output.stdout
    } else {
        &output.stderr
    };
    extract_version(&String::from_utf8_lossy(bytes))
}

/// Pulls a human-readable version string out of typical `--version` output.
///
/// Strategies in order: first line stripped of common prefixes ("git version ",
/// "tmux ", "gh version "); otherwise the first whitespace-separated token that
/// looks like a version (digit-led, optionally with `v` prefix); otherwise the
/// raw first line.
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

/// Honors `NO_COLOR` (https://no-color.org) and disables colors when stdout is
/// not a terminal so piped/redirected output stays plain.
fn use_color() -> bool {
    if std::env::var_os("NO_COLOR").is_some() {
        return false;
    }
    std::io::stdout().is_terminal()
}

const GREEN: &str = "\x1b[32m";
const RED: &str = "\x1b[31m";
const DIM: &str = "\x1b[2m";
const RESET: &str = "\x1b[0m";

fn print_row(row: &Row, name_width: usize, color: bool) {
    let (icon, detail, color_code) = match &row.status {
        Status::Found(v) => ("✓", v.clone(), GREEN),
        Status::FoundNoVersion => ("✓", "found".to_string(), GREEN),
        Status::Missing => ("x", "not found".to_string(), RED),
        Status::Skipped(reason) => ("-", format!("skipped ({reason})"), DIM),
    };

    let (on, off) = if color { (color_code, RESET) } else { ("", "") };
    let dim = if color { DIM } else { "" };

    println!(
        "{on}{icon}{off} {name:<width$}  {on}{detail}{off}  {dim}-- {purpose}{off}",
        name = row.name,
        width = name_width,
        detail = detail,
        purpose = row.purpose,
    );

    if let Some(hint) = &row.install_hint {
        println!(
            "  {empty:<width$}  install: {hint}",
            empty = "",
            width = name_width,
        );
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

    #[rstest]
    #[case::default_wezterm(
        Config::default(),
        &[
            ExternalTool::Git, ExternalTool::Gh, ExternalTool::Tmux,
            ExternalTool::Nvim, ExternalTool::Wezterm, ExternalTool::Delta,
            ExternalTool::Claude, ExternalTool::Opencode, ExternalTool::Hammerspoon,
        ],
    )]
    #[case::ghostty_terminal(
        {
            let mut c = Config::default();
            c.editor.terminal = Terminal::Ghostty;
            c
        },
        &[
            ExternalTool::Git, ExternalTool::Gh, ExternalTool::Tmux,
            ExternalTool::Nvim, ExternalTool::Ghostty, ExternalTool::Delta,
            ExternalTool::Claude, ExternalTool::Opencode, ExternalTool::Hammerspoon,
        ],
    )]
    #[case::custom_editor(
        {
            let mut c = Config::default();
            c.editor.editor_command = "helix".to_string();
            c
        },
        &[
            ExternalTool::Git, ExternalTool::Gh, ExternalTool::Tmux,
            ExternalTool::Wezterm, ExternalTool::Delta,
            ExternalTool::Claude, ExternalTool::Opencode, ExternalTool::Hammerspoon,
        ],
    )]
    #[case::notifications_disabled(
        {
            let mut c = Config::default();
            c.notification.enabled = false;
            c
        },
        &[
            ExternalTool::Git, ExternalTool::Gh, ExternalTool::Tmux,
            ExternalTool::Nvim, ExternalTool::Wezterm, ExternalTool::Delta,
            ExternalTool::Claude, ExternalTool::Opencode,
        ],
    )]
    fn selected_tools_cases(#[case] config: Config, #[case] expected: &[ExternalTool]) {
        assert_eq!(selected_tools(&config), expected);
    }
}
