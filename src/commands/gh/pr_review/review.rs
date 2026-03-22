use std::ffi::OsString;
use std::path::Path;

use clap::Args;

use super::markdown::serializer::ThreadsFrontmatter;
use super::reply;
use super::storage::ThreadStorage;
use crate::infra::git;
use crate::shared::config::load_config;
use crate::shared::human_in_the_loop::{
    Document, DocumentSchema, ReviewHandler, complete_review, start_review,
};

#[derive(Args, Clone, PartialEq, Eq)]
pub struct ReviewArgs {
    /// PR number
    pub pr_number: u64,

    /// Target repository (owner/repo)
    #[arg(short = 'R', long = "repo")]
    pub repo: Option<String>,

    /// Include resolved threads
    #[arg(long = "include-resolved")]
    pub include_resolved: bool,
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
}

/// Handler for PR review reply review sessions.
struct PrReviewReplyHandler {
    owner: String,
    repo: String,
    pr_number: u64,
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
            format!("{}/{}", self.owner, self.repo).into(),
        ];

        if let Some(target) = tmux_target {
            args.push("--tmux-target".into());
            args.push(target.into());
        }

        args.push("--window-title".into());
        args.push(window_title.into());

        args
    }

    // on_review_complete uses the default no-op implementation.
    // Push logic is handled after complete_review returns in run_review_complete.
}

pub async fn run_review(args: &ReviewArgs) -> anyhow::Result<()> {
    let (owner, repo) = git::get_repo_owner_and_name(args.repo.as_deref())?;

    // Pull first to ensure we have the latest data
    let pull_args = reply::ReplyPullArgs {
        pr_number: args.pr_number,
        repo: args.repo.clone(),
        include_resolved: args.include_resolved,
        force: false,
    };
    reply::run_pull(&pull_args).await?;

    let storage = ThreadStorage::new(&owner, &repo, args.pr_number);
    let threads_path = storage.threads_path();

    let config = load_config()?;
    let window_title = format!("PR Review: {owner}/{repo} #{}", args.pr_number);

    let handler = PrReviewReplyHandler {
        owner,
        repo,
        pr_number: args.pr_number,
    };

    start_review::<ThreadsFrontmatter, _>(&threads_path, &window_title, &handler, &config.editor)?;

    Ok(())
}

pub async fn run_review_complete(args: &ReviewCompleteArgs) -> anyhow::Result<()> {
    let config = load_config()?;

    let (owner, repo) = args
        .repo
        .split_once('/')
        .ok_or_else(|| anyhow::anyhow!("Invalid repo format: {}", args.repo))?;

    let handler = PrReviewReplyHandler {
        owner: owner.to_string(),
        repo: repo.to_string(),
        pr_number: args.pr_number,
    };

    complete_review::<ThreadsFrontmatter, _>(
        &args.filepath,
        args.tmux_target.as_deref(),
        args.window_title.as_deref(),
        &handler,
        &config.editor,
    )?;

    // After editor closes, check if user approved and push
    let document = Document::<ThreadsFrontmatter>::from_path(args.filepath.clone())?;

    if document.frontmatter.is_approved() {
        println!("Approved. Pushing replies to GitHub...");

        let push_args = reply::ReplyPushArgs {
            pr_number: args.pr_number,
            repo: Some(args.repo.clone()),
            dry_run: false,
            force: false,
        };

        if let Err(e) = reply::run_push(&push_args).await {
            eprintln!("Push failed: {e}");
        }
    } else {
        println!("Not approved. Set 'submit: true' in the frontmatter and save to approve.");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn build_review_args_should_include_pr_and_repo() {
        let handler = PrReviewReplyHandler {
            owner: "fohte".to_string(),
            repo: "armyknife".to_string(),
            pr_number: 42,
        };

        let path = std::path::PathBuf::from("/tmp/threads.md");
        let args = handler.build_complete_args(&path, Some("sess:1.0"), "Test Title");

        assert_eq!(args[0], "gh");
        assert_eq!(args[1], "pr-review");
        assert_eq!(args[2], "reply");
        assert_eq!(args[3], "review-complete");
        assert_eq!(args[4], std::ffi::OsStr::new("/tmp/threads.md"));
        assert_eq!(args[5], "--pr-number");
        assert_eq!(args[6], "42");
        assert_eq!(args[7], "--repo");
        assert_eq!(args[8], "fohte/armyknife");
        assert_eq!(args[9], "--tmux-target");
        assert_eq!(args[10], "sess:1.0");
        assert_eq!(args[11], "--window-title");
        assert_eq!(args[12], "Test Title");
    }

    #[cfg(unix)]
    #[test]
    fn build_review_args_without_tmux() {
        let handler = PrReviewReplyHandler {
            owner: "fohte".to_string(),
            repo: "armyknife".to_string(),
            pr_number: 10,
        };

        let path = std::path::PathBuf::from("/tmp/threads.md");
        let args = handler.build_complete_args(&path, None, "Title");

        assert_eq!(args.len(), 11);
        assert_eq!(args[9], "--window-title");
        assert_eq!(args[10], "Title");
    }
}
