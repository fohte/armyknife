use clap::Args;

#[derive(Args, Clone, PartialEq, Eq)]
pub struct ViewArgs {
    /// Issue number
    pub issue_number: u64,

    /// Target repository (owner/repo)
    #[arg(short = 'R', long)]
    pub repo: Option<String>,
}

pub async fn run(_args: &ViewArgs) -> Result<(), Box<dyn std::error::Error>> {
    todo!("view command not implemented")
}
