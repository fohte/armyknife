use clap::Args;

#[derive(Args, Clone, PartialEq, Eq)]
pub struct PushArgs {
    /// Issue number
    pub issue_number: u64,

    /// Target repository (owner/repo)
    #[arg(short = 'R', long)]
    pub repo: Option<String>,

    /// Show what would be changed without applying
    #[arg(long)]
    pub dry_run: bool,

    /// Allow overwriting remote changes (like git push --force)
    #[arg(long)]
    pub force: bool,

    /// Allow editing other users' comments
    #[arg(long)]
    pub edit_others: bool,
}

pub async fn run(_args: &PushArgs) -> Result<(), Box<dyn std::error::Error>> {
    todo!("push command not implemented")
}
