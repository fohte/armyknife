use clap::Args;

#[derive(Args, Clone, PartialEq, Eq, Debug)]
pub struct RefreshArgs {
    #[command(flatten)]
    pub issue: super::IssueArgs,
}

pub async fn run(_args: &RefreshArgs) -> Result<(), Box<dyn std::error::Error>> {
    todo!("refresh command not implemented")
}
