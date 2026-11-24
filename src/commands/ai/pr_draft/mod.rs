mod common;
mod hooks;
mod new;
mod review;
mod stack;
mod submit;

use clap::Subcommand;

pub(crate) async fn submit_draft(filepath: std::path::PathBuf) -> anyhow::Result<()> {
    submit::run(&submit::SubmitArgs {
        filepath: Some(filepath),
        base: None,
        draft: false,
        no_review: false,
    })
    .await
}

pub(crate) fn draft_dir() -> std::path::PathBuf {
    common::DraftFile::draft_dir()
}

pub(crate) fn parse_draft_path_under(
    path: &std::path::Path,
    draft_dir: &std::path::Path,
) -> Option<(String, String, String)> {
    common::DraftFile::parse_path_under(path, draft_dir)
}

#[derive(Subcommand, Clone, PartialEq, Eq)]
pub enum PrDraftCommands {
    /// Create a new PR body draft file
    New(new::NewArgs),

    /// Open the draft file in an editor for review
    Review(review::ReviewArgs),

    /// Internal: Complete the review process after Neovim exits
    #[command(hide = true)]
    ReviewComplete(review::ReviewCompleteArgs),

    /// Submit the draft as a pull request
    Submit(submit::SubmitArgs),
}

impl PrDraftCommands {
    pub async fn run(&self) -> anyhow::Result<()> {
        match self {
            Self::New(args) => new::run(args).await,
            Self::Review(args) => review::run(args),
            Self::ReviewComplete(args) => review::run_complete(args),
            Self::Submit(args) => submit::run(args).await,
        }
    }
}
