use anyhow::Result;
use clap::Args;

use super::tui;

#[derive(Args, Clone, PartialEq, Eq)]
pub struct WatchArgs {}

/// Runs the watch command.
/// Launches the TUI for real-time session monitoring.
pub fn run(_args: &WatchArgs) -> Result<()> {
    tui::run()
}
