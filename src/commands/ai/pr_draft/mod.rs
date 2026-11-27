mod common;
mod hooks;
mod new;
mod review;
mod stack;
mod submit;
mod submit_lock;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub(crate) enum ExpectedPullRequestTarget {
    New,
    Existing { number: u64 },
    Unknown,
}

#[derive(Debug, thiserror::Error)]
#[error(
    "The pull request target changed after approval. Send the draft back for review before submitting again."
)]
pub(crate) struct ExpectedPullRequestTargetMismatch;

use clap::Subcommand;

pub(crate) async fn submit_draft_for_review(
    filepath: std::path::PathBuf,
    session_cwd: &std::path::Path,
    target_repo_id: &str,
    expected_pr_target: Option<ExpectedPullRequestTarget>,
) -> anyhow::Result<String> {
    submit::run_for_review(
        &submit::SubmitArgs {
            filepath: Some(filepath),
            base: None,
            draft: false,
            no_review: false,
        },
        session_cwd,
        target_repo_id,
        expected_pr_target,
    )
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
