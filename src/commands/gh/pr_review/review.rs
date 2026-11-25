use std::ffi::OsString;
use std::path::Path;

use clap::Args;
use indoc::formatdoc;

use super::markdown::serializer::ThreadsFrontmatter;
use super::storage::ThreadStorage;
use crate::infra::git;
use crate::shared::config::load_config;
use crate::shared::human_in_the_loop::{
    CleanupGuard, Document, DocumentSchema, FifoSignalGuard, HumanInTheLoopError,
    Result as HilResult, ReviewFinishReason, ReviewHandler, complete_review,
    start_review_with_completion,
};
use crate::shared::yaml_frontmatter;

#[derive(Args, Clone, PartialEq, Eq)]
pub struct ReviewArgs {
    /// PR number
    pub pr_number: u64,

    /// Target repository (owner/repo)
    #[arg(short = 'R', long = "repo")]
    pub repo: Option<String>,
}

/// Internal command to complete the review process after the editor exits.
#[derive(Args, Clone, PartialEq, Eq)]
pub struct ReviewCompleteArgs {
    /// Path to the threads.md file
    pub filepath: std::path::PathBuf,

    /// PR number
    #[arg(long)]
    pub pr_number: u64,

    /// Target repository (owner/repo)
    #[arg(long)]
    pub repo: String,

    /// tmux session to restore after review
    #[arg(long)]
    pub tmux_target: Option<String>,

    /// Window title for Neovim
    #[arg(long)]
    pub window_title: Option<String>,

    /// Internal: FIFO path to signal completion to the waiting start_review process
    #[arg(long, hide = true)]
    pub done_fifo: Option<std::path::PathBuf>,

    /// Internal: FIFO path for watch-web to request review completion
    #[arg(long, hide = true)]
    pub finish_fifo: Option<std::path::PathBuf>,

    /// Internal: identifier of the lock created by the review launcher
    #[arg(long, hide = true)]
    pub review_lock_id: Option<String>,
}

/// Handler for PR review reply review sessions.
struct PrReviewReplyHandler {
    pr_number: u64,
    repo_slug: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReplyReviewOutcome {
    Approved,
    NotApproved,
    FinishedByPush,
}

fn reply_review_outcome(
    document: &Document<ThreadsFrontmatter>,
    finish_reason: Option<ReviewFinishReason>,
) -> ReplyReviewOutcome {
    if finish_reason == Some(ReviewFinishReason::RepliesPushed) {
        ReplyReviewOutcome::FinishedByPush
    } else if document.frontmatter.is_approved() {
        ReplyReviewOutcome::Approved
    } else {
        ReplyReviewOutcome::NotApproved
    }
}

impl ReviewHandler<ThreadsFrontmatter> for PrReviewReplyHandler {
    const NOTIFICATION_COMMAND_NAME: &'static str = "pr-review";

    fn build_complete_args(
        &self,
        document_path: &Path,
        tmux_target: Option<&str>,
        window_title: &str,
    ) -> Vec<OsString> {
        let mut args: Vec<OsString> = vec![
            "gh".into(),
            Self::NOTIFICATION_COMMAND_NAME.into(),
            "reply".into(),
            "review-complete".into(),
            document_path.as_os_str().to_os_string(),
            "--pr-number".into(),
            self.pr_number.to_string().into(),
            "--repo".into(),
            self.repo_slug.clone().into(),
        ];

        if let Some(target) = tmux_target {
            args.push("--tmux-target".into());
            args.push(target.into());
        }

        args.push("--window-title".into());
        args.push(window_title.into());

        args
    }

    fn on_review_complete(&self, document: &Document<ThreadsFrontmatter>) -> HilResult<()> {
        document.persist_approval_state()?;
        if document.frontmatter.is_approved() {
            println!(
                "{}",
                formatdoc! {"
                    Replies approved. Run the following command to push:

                        a gh pr-review reply push {pr_number} -R {repo}
                ",
                    pr_number = self.pr_number,
                    repo = self.repo_slug,
                }
            );
        } else {
            println!("Not approved. Set 'submit: true' and save to approve.");
        }

        Ok(())
    }

    fn on_watch_web_review_complete(
        &self,
        document: &Document<ThreadsFrontmatter>,
    ) -> HilResult<()> {
        document.persist_approval_state()
    }

    fn on_review_finished(
        &self,
        document: &Document<ThreadsFrontmatter>,
        reason: ReviewFinishReason,
    ) -> HilResult<()> {
        if reason == ReviewFinishReason::RepliesPushed {
            let content = std::fs::read_to_string(&document.path)?;
            let updated_content = yaml_frontmatter::reset_bool_fields(&content, &["submit"]);
            if document.frontmatter.submit && updated_content == content {
                return Err(HumanInTheLoopError::CommandFailed(format!(
                    "Failed to reset submit in frontmatter at {}",
                    document.path.display(),
                )));
            }
            if updated_content != content {
                std::fs::write(&document.path, updated_content)?;
            }
            document.remove_approval()?;
            return Ok(());
        }

        self.on_review_complete(document)
    }
}

pub fn run_review(args: &ReviewArgs) -> anyhow::Result<()> {
    let (owner, repo) = git::get_repo_owner_and_name(args.repo.as_deref())?;

    let storage = ThreadStorage::new(&owner, &repo, args.pr_number);
    let threads_path = storage.threads_path();

    if !storage.exists() {
        return Err(super::error::PrReviewError::NoPulledData.into());
    }

    let config = load_config()?;
    let window_title = format!("PR Review: {owner}/{repo} #{}", args.pr_number);

    let handler = PrReviewReplyHandler {
        pr_number: args.pr_number,
        repo_slug: format!("{owner}/{repo}"),
    };

    use crate::shared::human_in_the_loop::exit_code;

    let review =
        exit_code::exit_on_review_failure(start_review_with_completion::<ThreadsFrontmatter, _>(
            &threads_path,
            &window_title,
            &handler,
            &config.editor,
            config.notification.enabled,
        ))?;

    let Some(review) = review else {
        std::process::exit(exit_code::ALREADY_OPEN);
    };

    match reply_review_outcome(&review.document, review.finish_reason) {
        ReplyReviewOutcome::FinishedByPush => {
            println!("Reply review finished because push succeeded.");
            Ok(())
        }
        ReplyReviewOutcome::Approved => Ok(()),
        ReplyReviewOutcome::NotApproved => {
            eprintln!("Review not approved.");
            std::process::exit(exit_code::NOT_APPROVED);
        }
    }
}

pub fn run_review_complete(args: &ReviewCompleteArgs) -> anyhow::Result<()> {
    // Create FIFO guard first to ensure signaling even if load_config fails
    let mut fifo_guard = args.done_fifo.as_deref().map(FifoSignalGuard::new);
    let cleanup_guard = CleanupGuard::from_launcher_args(
        &args.filepath,
        args.review_lock_id.as_deref(),
        args.tmux_target.clone(),
    )?;

    let config = load_config()?;

    let handler = PrReviewReplyHandler {
        pr_number: args.pr_number,
        repo_slug: args.repo.clone(),
    };

    complete_review::<ThreadsFrontmatter, _, _>(
        &args.filepath,
        args.window_title.as_deref(),
        &handler,
        &config.editor,
        args.finish_fifo.as_deref(),
        &cleanup_guard,
        fifo_guard.as_mut(),
    )?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use indoc::indoc;

    #[cfg(unix)]
    #[rstest::rstest]
    #[case::with_tmux(
        42,
        Some("sess:1.0"),
        "Test Title",
        vec![
            "gh", "pr-review", "reply", "review-complete",
            "/tmp/threads.md", "--pr-number", "42",
            "--repo", "fohte/armyknife",
            "--tmux-target", "sess:1.0",
            "--window-title", "Test Title",
        ],
    )]
    #[case::without_tmux(
        10,
        None,
        "Title",
        vec![
            "gh", "pr-review", "reply", "review-complete",
            "/tmp/threads.md", "--pr-number", "10",
            "--repo", "fohte/armyknife",
            "--window-title", "Title",
        ],
    )]
    fn review_handler_command_and_build_review_args(
        #[case] pr_number: u64,
        #[case] tmux_target: Option<&str>,
        #[case] window_title: &str,
        #[case] expected: Vec<&str>,
    ) {
        let handler = PrReviewReplyHandler {
            pr_number,
            repo_slug: "fohte/armyknife".to_string(),
        };

        let path = std::path::PathBuf::from("/tmp/threads.md");
        let args = handler.build_complete_args(&path, tmux_target, window_title);

        let args_str: Vec<String> = args
            .iter()
            .map(|a| a.to_string_lossy().to_string())
            .collect();
        let expected: Vec<String> = expected.iter().map(|arg| arg.to_string()).collect();
        assert_eq!(
            (PrReviewReplyHandler::NOTIFICATION_COMMAND_NAME, args_str),
            ("pr-review", expected),
        );
    }

    #[rstest::rstest]
    #[case::approved(true, None, ReplyReviewOutcome::Approved)]
    #[case::not_approved(false, None, ReplyReviewOutcome::NotApproved)]
    #[case::external_finish_not_approved(
        false,
        Some(ReviewFinishReason::ExternalRequest),
        ReplyReviewOutcome::NotApproved
    )]
    #[case::finished_by_push(
        false,
        Some(ReviewFinishReason::RepliesPushed),
        ReplyReviewOutcome::FinishedByPush
    )]
    fn reply_review_outcome_reports_push_completion(
        #[case] approved: bool,
        #[case] finish_reason: Option<ReviewFinishReason>,
        #[case] expected: ReplyReviewOutcome,
    ) {
        let content = if approved {
            indoc! {"
                ---
                pr: 123
                repo: example/repo
                pulled_at: '2024-01-15T10:00:00Z'
                submit: true
                ---
                Draft reply
            "}
        } else {
            indoc! {"
                ---
                pr: 123
                repo: example/repo
                pulled_at: '2024-01-15T10:00:00Z'
                submit: false
                ---
                Draft reply
            "}
        };
        let document = Document::<ThreadsFrontmatter>::from_content(
            std::path::PathBuf::from("threads.md"),
            content,
        )
        .expect("threads document should parse");

        assert_eq!(reply_review_outcome(&document, finish_reason), expected);
    }

    #[rstest::rstest]
    fn push_completion_resets_submit_and_removes_approval() {
        use tempfile::TempDir;

        crate::shared::testing::init_approval_dir();
        let temp_dir = TempDir::new().expect("temporary review directory");
        let path = temp_dir.path().join("threads.md");
        let content = indoc! {"
            ---
            pr: 123
            repo: example/repo
            pulled_at: '2024-01-15T10:00:00Z'
            submit: true
            ---
            Draft reply
        "};
        std::fs::write(&path, content).expect("write approved reply document");
        let document = Document::<ThreadsFrontmatter>::from_path(path.clone())
            .expect("load approved reply document");
        document.save_approval().expect("save approval");
        let handler = PrReviewReplyHandler {
            pr_number: 123,
            repo_slug: "example/repo".to_string(),
        };

        let outcome = (
            handler
                .on_review_finished(&document, ReviewFinishReason::RepliesPushed)
                .map_err(|error| error.to_string()),
            std::fs::read_to_string(path).expect("read reset reply document"),
            document.approval_manager().exists(),
        );

        assert_eq!(
            outcome,
            (
                Ok(()),
                indoc! {"
                    ---
                    pr: 123
                    repo: example/repo
                    pulled_at: '2024-01-15T10:00:00Z'
                    submit: false
                    ---
                    Draft reply
                "}
                .to_string(),
                false,
            ),
        );
    }
}
