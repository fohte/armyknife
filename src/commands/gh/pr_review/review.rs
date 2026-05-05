use std::ffi::OsString;
use std::path::Path;

use clap::Args;
use indoc::formatdoc;

use super::markdown::serializer::ThreadsFrontmatter;
use super::storage::ThreadStorage;
use crate::infra::git;
use crate::shared::config::load_config;
use crate::shared::diff::write_diff;
use crate::shared::human_in_the_loop::{
    Document, DocumentSchema, FifoSignalGuard, Result as HilResult, ReviewHandler, complete_review,
    start_review,
};

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
}

/// Handler for PR review reply review sessions.
struct PrReviewReplyHandler {
    pr_number: u64,
    repo_slug: String,
}

impl ReviewHandler<ThreadsFrontmatter> for PrReviewReplyHandler {
    fn build_complete_args(
        &self,
        document_path: &Path,
        tmux_target: Option<&str>,
        window_title: &str,
    ) -> Vec<OsString> {
        let mut args: Vec<OsString> = vec![
            "gh".into(),
            "pr-review".into(),
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
        if document.frontmatter.is_approved() {
            document.save_approval()?;
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
            document.remove_approval()?;
            println!("Not approved. Set 'submit: true' and save to approve.");
        }

        Ok(())
    }
}

/// Emit a diff between `pre` and `post` to `writer`, or a "(no edits)" notice
/// when they match. Used after the editor exits to summarize what changed
/// without forcing the caller to re-read the file.
fn write_edit_diff<W: std::io::Write>(
    writer: &mut W,
    pre: &str,
    post: &str,
    use_color: bool,
) -> std::io::Result<()> {
    if pre == post {
        writeln!(writer, "(no edits)")?;
        return Ok(());
    }
    write_diff(writer, pre, post, use_color)
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

    // Snapshot the document before launching the editor so we can emit a diff
    // afterwards. The skill consumer can then learn what changed without
    // re-reading the entire file.
    let pre_edit = std::fs::read_to_string(&threads_path)?;

    let document =
        exit_code::exit_on_terminal_launch_failure(start_review::<ThreadsFrontmatter, _>(
            &threads_path,
            &window_title,
            &handler,
            &config.editor,
        ))?;

    if document.is_none() {
        std::process::exit(exit_code::ALREADY_OPEN);
    }

    let post_edit = std::fs::read_to_string(&threads_path)?;
    {
        use crossterm::tty::IsTty;
        let use_color = std::io::stdout().is_tty();
        let mut stdout = std::io::stdout().lock();
        if let Err(e) = write_edit_diff(&mut stdout, &pre_edit, &post_edit, use_color)
            && e.kind() != std::io::ErrorKind::BrokenPipe
        {
            return Err(e.into());
        }
    }

    let approved = document
        .as_ref()
        .is_some_and(|d| d.frontmatter.is_approved());
    if !approved {
        eprintln!("Review not approved.");
        std::process::exit(exit_code::NOT_APPROVED);
    }

    Ok(())
}

pub fn run_review_complete(args: &ReviewCompleteArgs) -> anyhow::Result<()> {
    // Create FIFO guard first to ensure signaling even if load_config fails
    let _fifo_guard = args.done_fifo.as_deref().map(FifoSignalGuard::new);

    let config = load_config()?;

    let handler = PrReviewReplyHandler {
        pr_number: args.pr_number,
        repo_slug: args.repo.clone(),
    };

    complete_review::<ThreadsFrontmatter, _>(
        &args.filepath,
        args.tmux_target.as_deref(),
        args.window_title.as_deref(),
        &handler,
        &config.editor,
    )?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn build_review_args(
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
        assert_eq!(args_str, expected);
    }

    #[rstest::rstest]
    #[case::no_change("same\n", "same\n", "(no edits)\n")]
    #[case::added_line("a\n", indoc::indoc! {"
        a
        b
    "}, indoc::indoc! {"
         a
        +b
    "})]
    #[case::deleted_line(indoc::indoc! {"
        a
        b
    "}, "a\n", indoc::indoc! {"
         a
        -b
    "})]
    #[case::modified_line("old\n", "new\n", indoc::indoc! {"
        -old
        +new
    "})]
    #[case::both_empty("", "", "(no edits)\n")]
    fn write_edit_diff_emits_expected_output(
        #[case] pre: &str,
        #[case] post: &str,
        #[case] expected: &str,
    ) {
        let mut buf = Vec::new();
        write_edit_diff(&mut buf, pre, post, false).expect("write");
        assert_eq!(String::from_utf8(buf).expect("utf8"), expected);
    }
}
