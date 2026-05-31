//! Central registry of external CLI tools armyknife may invoke.
//!
//! Every tool that the rest of the codebase spawns should be declared here.
//! `a doctor` enumerates [`ExternalTool::ALL`] to report availability and
//! versions, so leaving a callsite outside the registry means the user has no
//! way to discover that dependency before it fails at runtime.

use std::path::PathBuf;
use std::process::Command;

use crate::shared::command;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExternalTool {
    Git,
    Gh,
    Tmux,
    Nvim,
    Wezterm,
    Ghostty,
    Delta,
    Claude,
    Opencode,
    Hammerspoon,
}

pub struct Metadata {
    /// Display name shown by `a doctor`.
    pub name: &'static str,
    /// Executable name searched on `PATH`. Often matches `name`, but differs
    /// for tools like Hammerspoon whose CLI is `hs`.
    pub binary: &'static str,
    /// One-line description of what armyknife uses this tool for.
    pub purpose: &'static str,
    /// Arguments to print the tool's version.
    pub version_args: &'static [&'static str],
    /// macOS-only tools are reported as `skipped` on other platforms.
    pub macos_only: bool,
    /// Tail of the install hint shown after `brew install` when missing.
    pub brew_pkg: Option<&'static str>,
    /// Absolute path to a `.app` bundle whose existence proves the tool is
    /// installed on macOS even when no CLI is on `PATH` (e.g. WezTerm launched
    /// via `open -a`, or Hammerspoon's bundled `hs` binary).
    pub macos_app_path: Option<&'static str>,
}

impl ExternalTool {
    pub const ALL: &'static [Self] = &[
        Self::Git,
        Self::Gh,
        Self::Tmux,
        Self::Nvim,
        Self::Wezterm,
        Self::Ghostty,
        Self::Delta,
        Self::Claude,
        Self::Opencode,
        Self::Hammerspoon,
    ];

    pub const fn metadata(self) -> Metadata {
        match self {
            Self::Git => Metadata {
                name: "git",
                binary: "git",
                purpose: "git operations",
                version_args: &["--version"],
                macos_only: false,
                brew_pkg: None,
                macos_app_path: None,
            },
            Self::Gh => Metadata {
                name: "gh",
                binary: "gh",
                purpose: "GitHub integration",
                version_args: &["--version"],
                macos_only: false,
                brew_pkg: None,
                macos_app_path: None,
            },
            Self::Tmux => Metadata {
                name: "tmux",
                binary: "tmux",
                purpose: "session monitor, worktree windows",
                version_args: &["-V"],
                macos_only: false,
                brew_pkg: None,
                macos_app_path: None,
            },
            Self::Nvim => Metadata {
                name: "nvim",
                binary: "nvim",
                purpose: "default editor for human-in-the-loop reviews",
                version_args: &["--version"],
                macos_only: false,
                brew_pkg: Some("neovim"),
                macos_app_path: None,
            },
            Self::Wezterm => Metadata {
                name: "wezterm",
                binary: "wezterm",
                purpose: "terminal emulator for review windows",
                version_args: &["--version"],
                macos_only: false,
                brew_pkg: Some("--cask wezterm"),
                macos_app_path: Some("/Applications/WezTerm.app"),
            },
            Self::Ghostty => Metadata {
                name: "ghostty",
                binary: "ghostty",
                purpose: "alternative terminal emulator for review windows",
                version_args: &["--version"],
                macos_only: false,
                brew_pkg: Some("--cask ghostty"),
                macos_app_path: Some("/Applications/Ghostty.app"),
            },
            Self::Delta => Metadata {
                name: "delta",
                binary: "delta",
                purpose: "diff pager for `a gh pr-review`",
                version_args: &["--version"],
                macos_only: false,
                brew_pkg: Some("git-delta"),
                macos_app_path: None,
            },
            Self::Claude => Metadata {
                name: "claude",
                binary: "claude",
                purpose: "Claude Code backend for `a name-branch`",
                version_args: &["--version"],
                macos_only: false,
                brew_pkg: None,
                macos_app_path: None,
            },
            Self::Opencode => Metadata {
                name: "opencode",
                binary: "opencode",
                purpose: "opencode backend for `a name-branch`",
                version_args: &["--version"],
                macos_only: false,
                brew_pkg: None,
                macos_app_path: None,
            },
            Self::Hammerspoon => Metadata {
                name: "hammerspoon",
                binary: "hs",
                purpose: "desktop notifications",
                version_args: &["-c", "print(hs.processInfo.version)"],
                macos_only: true,
                brew_pkg: Some("--cask hammerspoon"),
                macos_app_path: Some("/Applications/Hammerspoon.app/Contents/Frameworks/hs/hs"),
            },
        }
    }

    pub const fn name(self) -> &'static str {
        self.metadata().name
    }

    pub const fn binary(self) -> &'static str {
        self.metadata().binary
    }

    /// Returns the install hint shown when the tool is missing.
    pub fn install_hint(self) -> String {
        let pkg = self.metadata().brew_pkg.unwrap_or(self.binary());
        format!("brew install {pkg}")
    }

    /// Resolves the tool's CLI to an absolute path via PATH, falling back to
    /// `macos_app_path` on macOS when the CLI is not on PATH but the .app
    /// bundle (or bundled binary) exists.
    pub fn resolve_path(self) -> Option<PathBuf> {
        if let Some(p) = command::find_command_path(self.binary()) {
            return Some(p);
        }
        if cfg!(target_os = "macos")
            && let Some(app_path) = self.metadata().macos_app_path
        {
            let p = PathBuf::from(app_path);
            if p.is_file() {
                return Some(p);
            }
        }
        None
    }

    /// Returns true if the tool is usable, including the macOS case where only
    /// the `.app` bundle is installed and the spawn path is something like
    /// `open -a` (i.e. no CLI on PATH).
    pub fn is_available(self) -> bool {
        if self.resolve_path().is_some() {
            return true;
        }
        if cfg!(target_os = "macos")
            && let Some(app_path) = self.metadata().macos_app_path
        {
            return std::path::Path::new(app_path).exists();
        }
        false
    }

    /// Builds a [`Command`] for this tool, using the resolved absolute path
    /// when available, otherwise the bare binary name (so the OS can produce
    /// a clear "not found" error on spawn).
    pub fn command(self) -> Command {
        match self.resolve_path() {
            Some(p) => Command::new(p),
            None => Command::new(self.binary()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_lists_every_variant_exactly_once() {
        // Sorted for stable comparison; ExternalTool::ALL is intentionally
        // ordered for doctor display, so we sort both sides before asserting.
        let mut got: Vec<&str> = ExternalTool::ALL.iter().map(|t| t.name()).collect();
        got.sort_unstable();

        let mut want = vec![
            "git",
            "gh",
            "tmux",
            "nvim",
            "wezterm",
            "ghostty",
            "delta",
            "claude",
            "opencode",
            "hammerspoon",
        ];
        want.sort_unstable();

        assert_eq!(got, want);
    }

    #[test]
    fn install_hint_uses_brew_override() {
        assert_eq!(ExternalTool::Nvim.install_hint(), "brew install neovim");
    }

    #[test]
    fn install_hint_defaults_to_binary_name() {
        assert_eq!(ExternalTool::Git.install_hint(), "brew install git");
    }

    #[test]
    fn install_hint_for_hammerspoon_uses_cask() {
        assert_eq!(
            ExternalTool::Hammerspoon.install_hint(),
            "brew install --cask hammerspoon"
        );
    }
}
