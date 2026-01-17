use clap::Args;

#[derive(Args, Clone, PartialEq, Eq, Debug)]
pub struct PullArgs {
    #[command(flatten)]
    pub issue: super::IssueArgs,
}

pub async fn run(_args: &PullArgs) -> Result<(), Box<dyn std::error::Error>> {
    todo!("pull command not implemented")
}
