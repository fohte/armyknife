use clap::Args;
use std::process::Command;

use super::error::WmError;

#[derive(Args, Clone, PartialEq, Eq)]
pub struct ListArgs {}

pub fn run(_args: &ListArgs) -> Result<(), Box<dyn std::error::Error>> {
    let status = Command::new("git").args(["worktree", "list"]).status()?;

    if !status.success() {
        return Err(Box::new(WmError::CommandFailed(
            "git worktree list failed".into(),
        )));
    }

    Ok(())
}
