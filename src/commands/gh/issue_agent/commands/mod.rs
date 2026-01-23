mod common;
mod pull;
mod push;
#[cfg(test)]
mod test_helpers;
mod view;

use clap::Args;

pub use pull::PullArgs;
pub use pull::run as run_pull;
pub use push::PushArgs;
pub use push::run as run_push;
pub use view::ViewArgs;
pub use view::run as run_view;

/// Common arguments shared across all issue-agent commands
#[derive(Args, Clone, PartialEq, Eq, Debug)]
pub struct IssueArgs {
    /// Issue number
    pub issue_number: u64,

    /// Target repository (owner/repo)
    #[arg(short = 'R', long)]
    pub repo: Option<String>,
}
