//! Review command for gh-issue-agent.
//!
//! Opens a file in an editor via the HITL framework for approval.
//! The user must set `submit: true` in the frontmatter to approve.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use clap::Args;
use serde::{Deserialize, Serialize};

use crate::shared::config::load_config;
use crate::shared::human_in_the_loop::{
    Document, DocumentSchema, FifoSignalGuard, Result as HilResult, ReviewHandler, complete_review,
    start_review,
};

#[derive(Args, Clone, PartialEq, Eq, Debug)]
pub struct ReviewArgs {
    /// Path to the file to review (e.g., issue.md, comments/new_foo.md)
    pub path: PathBuf,
}

/// Internal command to complete the review process after the editor exits.
/// This is called by the terminal, not directly by users.
#[derive(Args, Clone, PartialEq, Eq, Debug)]
pub struct ReviewCompleteArgs {
    /// Path to the file
    pub path: PathBuf,

    /// tmux session to restore after review
    #[arg(long)]
    pub tmux_target: Option<String>,

    /// Window title for the editor
    #[arg(long)]
    pub window_title: Option<String>,

    /// Internal: FIFO path to signal completion to the waiting start_review process
    #[arg(long, hide = true)]
    pub done_fifo: Option<PathBuf>,
}

/// Frontmatter schema that checks the `submit` field for approval.
///
/// Uses `#[serde(flatten)]` with a catch-all map to accept any additional
/// frontmatter fields (title, labels, etc.) without errors.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SubmitSchema {
    #[serde(default)]
    pub submit: bool,
    #[serde(flatten)]
    _extra: std::collections::HashMap<String, serde_yaml::Value>,
}

impl DocumentSchema for SubmitSchema {
    fn is_approved(&self) -> bool {
        self.submit
    }
}

/// Handler for issue-agent review sessions.
struct IssueReviewHandler;

impl ReviewHandler<SubmitSchema> for IssueReviewHandler {
    fn build_complete_args(
        &self,
        document_path: &Path,
        tmux_target: Option<&str>,
        window_title: &str,
    ) -> Vec<OsString> {
        let mut args: Vec<OsString> = vec![
            "gh".into(),
            "issue-agent".into(),
            "review-complete".into(),
            document_path.as_os_str().to_os_string(),
        ];

        if let Some(target) = tmux_target {
            args.push("--tmux-target".into());
            args.push(target.into());
        }

        args.push("--window-title".into());
        args.push(window_title.into());

        args
    }

    fn on_review_complete(&self, document: &Document<SubmitSchema>) -> HilResult<()> {
        if document.frontmatter.is_approved() {
            document.save_approval()?;
            println!("Approved.");
        } else {
            document.remove_approval()?;
            println!("Not approved. Set 'submit: true' in the frontmatter to approve.");
        }
        Ok(())
    }
}

pub fn run(args: &ReviewArgs) -> anyhow::Result<()> {
    let path = args
        .path
        .canonicalize()
        .map_err(|_| anyhow::anyhow!("File not found: {}", args.path.display()))?;

    let config = load_config()?;
    let file_name = path
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "file".to_string());
    let window_title = format!("Review: {}", file_name);

    let document =
        start_review::<SubmitSchema, _>(&path, &window_title, &IssueReviewHandler, &config.editor)?;

    let approved = document
        .as_ref()
        .is_some_and(|d| d.frontmatter.is_approved());
    if !approved {
        std::process::exit(1);
    }

    Ok(())
}

pub fn run_complete(args: &ReviewCompleteArgs) -> anyhow::Result<()> {
    // Create FIFO guard first to ensure signaling even if load_config fails
    let _fifo_guard = args.done_fifo.as_deref().map(FifoSignalGuard::new);

    let config = load_config()?;

    complete_review::<SubmitSchema, _>(
        &args.path,
        args.tmux_target.as_deref(),
        args.window_title.as_deref(),
        &IssueReviewHandler,
        &config.editor,
    )?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[cfg(unix)]
    #[rstest]
    #[case::with_tmux(
        Some("sess:1.0"),
        "Test Title",
        vec![
            "gh", "issue-agent", "review-complete",
            "/tmp/issue.md",
            "--tmux-target", "sess:1.0",
            "--window-title", "Test Title",
        ],
    )]
    #[case::without_tmux(
        None,
        "Title",
        vec![
            "gh", "issue-agent", "review-complete",
            "/tmp/issue.md",
            "--window-title", "Title",
        ],
    )]
    fn build_review_args(
        #[case] tmux_target: Option<&str>,
        #[case] window_title: &str,
        #[case] expected: Vec<&str>,
    ) {
        let handler = IssueReviewHandler;
        let path = std::path::PathBuf::from("/tmp/issue.md");
        let args = handler.build_complete_args(&path, tmux_target, window_title);

        let args_str: Vec<String> = args
            .iter()
            .map(|a| a.to_string_lossy().to_string())
            .collect();
        assert_eq!(args_str, expected);
    }
}
