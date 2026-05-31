//! Central registry of external CLI tools armyknife may invoke.
//!
//! Every tool that the rest of the codebase spawns should be declared here.
//! `a doctor` enumerates [`ExternalTool::ALL`] to report availability and
//! versions, so leaving a callsite outside the registry means the user has no
//! way to discover that dependency before it fails at runtime.

use std::path::PathBuf;
use std::process::Command;

use crate::shared::command;

const HS_BUNDLED_PATH: &str = "/Applications/Hammerspoon.app/Contents/Frameworks/hs/hs";

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
            },
            Self::Gh => Metadata {
                name: "gh",
                binary: "gh",
                purpose: "GitHub integration",
                version_args: &["--version"],
                macos_only: false,
                brew_pkg: None,
            },
            Self::Tmux => Metadata {
                name: "tmux",
                binary: "tmux",
                purpose: "session monitor, worktree windows",
                version_args: &["-V"],
                macos_only: false,
                brew_pkg: None,
            },
            Self::Nvim => Metadata {
                name: "nvim",
                binary: "nvim",
                purpose: "default editor for human-in-the-loop reviews",
                version_args: &["--version"],
                macos_only: false,
                brew_pkg: Some("neovim"),
            },
            Self::Wezterm => Metadata {
                name: "wezterm",
                binary: "wezterm",
                purpose: "terminal emulator for review windows",
                version_args: &["--version"],
                macos_only: false,
                brew_pkg: Some("--cask wezterm"),
            },
            Self::Ghostty => Metadata {
                name: "ghostty",
                binary: "ghostty",
                purpose: "alternative terminal emulator for review windows",
                version_args: &["--version"],
                macos_only: false,
                brew_pkg: Some("--cask ghostty"),
            },
            Self::Delta => Metadata {
                name: "delta",
                binary: "delta",
                purpose: "diff pager for `a gh pr-review`",
                version_args: &["--version"],
                macos_only: false,
                brew_pkg: Some("git-delta"),
            },
            Self::Claude => Metadata {
                name: "claude",
                binary: "claude",
                purpose: "Claude Code backend for `a name-branch`",
                version_args: &["--version"],
                macos_only: false,
                brew_pkg: None,
            },
            Self::Opencode => Metadata {
                name: "opencode",
                binary: "opencode",
                purpose: "opencode backend for `a name-branch`",
                version_args: &["--version"],
                macos_only: false,
                brew_pkg: None,
            },
            Self::Hammerspoon => Metadata {
                name: "hammerspoon",
                binary: "hs",
                purpose: "desktop notifications",
                version_args: &["-c", "print(hs.processInfo.version)"],
                macos_only: true,
                brew_pkg: Some("--cask hammerspoon"),
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

    /// Resolves the tool to an absolute path. For Hammerspoon, falls back to
    /// the bundled CLI location at [`HS_BUNDLED_PATH`].
    pub fn resolve_path(self) -> Option<PathBuf> {
        if let Some(p) = command::find_command_path(self.binary()) {
            return Some(p);
        }
        if matches!(self, Self::Hammerspoon) {
            let bundled = PathBuf::from(HS_BUNDLED_PATH);
            if bundled.is_file() {
                return Some(bundled);
            }
        }
        None
    }

    pub fn is_available(self) -> bool {
        self.resolve_path().is_some()
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
