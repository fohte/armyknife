use clap::Args;

#[derive(Args, Clone, PartialEq, Eq)]
pub struct RefreshArgs {
    /// Issue number
    pub issue_number: u64,

    /// Target repository (owner/repo)
    #[arg(short = 'R', long)]
    pub repo: Option<String>,
}

pub async fn run(_args: &RefreshArgs) -> Result<(), Box<dyn std::error::Error>> {
    todo!("refresh command not implemented")
}
