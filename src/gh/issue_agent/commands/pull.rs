use clap::Args;

#[derive(Args, Clone, PartialEq, Eq)]
pub struct PullArgs {
    /// Issue number
    pub issue_number: u64,

    /// Target repository (owner/repo)
    #[arg(short = 'R', long)]
    pub repo: Option<String>,
}

pub async fn run(_args: &PullArgs) -> Result<(), Box<dyn std::error::Error>> {
    todo!("pull command not implemented")
}
