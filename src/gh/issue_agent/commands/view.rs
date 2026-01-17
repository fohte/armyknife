use clap::Args;

#[derive(Args, Clone, PartialEq, Eq, Debug)]
pub struct ViewArgs {
    #[command(flatten)]
    pub issue: super::IssueArgs,
}

pub async fn run(_args: &ViewArgs) -> Result<(), Box<dyn std::error::Error>> {
    todo!("view command not implemented")
}
