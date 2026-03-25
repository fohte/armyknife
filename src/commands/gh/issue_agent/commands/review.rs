//! Review command for gh-issue-agent.
//!
//! Opens a file in an editor via the HITL framework for approval.
//! The user must set `submit: true` in the frontmatter to approve.
//!
//! For files without YAML frontmatter (e.g., comment files using HTML comment
//! metadata), a temporary `submit: false` frontmatter is prepended before
//! opening the editor and stripped after the review completes.

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
            // Strip the temporary frontmatter before saving approval hash.
            // The hash must match the file content at push time, so we save
            // the hash of the file *after* stripping.
            strip_temporary_frontmatter(&document.path)?;
            document.save_approval()?;
            println!("Approved.");
        } else {
            strip_temporary_frontmatter(&document.path)?;
            document.remove_approval()?;
            println!("Not approved. Set 'submit: true' in the frontmatter to approve.");
        }
        Ok(())
    }
}

/// Check if a file has YAML frontmatter (starts with `---\n`).
fn has_yaml_frontmatter(content: &str) -> bool {
    content.starts_with("---\n")
}

/// Prepend a temporary `submit: false` frontmatter to a file that lacks one.
fn prepend_temporary_frontmatter(path: &Path) -> std::io::Result<()> {
    let content = std::fs::read_to_string(path)?;
    let new_content = indoc::formatdoc! {"
        ---
        submit: false
        ---
        {content}"};

    std::fs::write(path, new_content)?;
    Ok(())
}

/// Strip the temporary frontmatter added by `prepend_temporary_frontmatter`.
///
/// Only strips if the frontmatter contains *only* the `submit` field
/// (with any boolean value). Frontmatter with additional fields (e.g.,
/// issue.md with title, labels, etc.) is left intact.
fn strip_temporary_frontmatter(path: &Path) -> HilResult<()> {
    let content = std::fs::read_to_string(path)?;
    if !content.starts_with("---\n") {
        return Ok(());
    }

    // Find the closing `---`
    let rest = &content[4..]; // skip opening "---\n"
    let Some(end) = rest.find("---\n") else {
        return Ok(());
    };

    let yaml_block = rest[..end].trim();

    // Only strip if the frontmatter contains exactly `submit: true` or `submit: false`
    if yaml_block == "submit: true" || yaml_block == "submit: false" {
        let body = &rest[end + 4..]; // skip closing "---\n"
        std::fs::write(path, body)?;
    }

    Ok(())
}

pub fn run(args: &ReviewArgs) -> anyhow::Result<()> {
    let path = args
        .path
        .canonicalize()
        .map_err(|_| anyhow::anyhow!("File not found: {}", args.path.display()))?;

    // If the file has no YAML frontmatter, prepend a temporary one
    let content = std::fs::read_to_string(&path)?;
    if !has_yaml_frontmatter(&content) {
        prepend_temporary_frontmatter(&path)?;
    }

    let config = load_config()?;
    let file_name = path
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "file".to_string());
    let window_title = format!("Review: {}", file_name);

    use crate::shared::human_in_the_loop::exit_code;

    let document =
        start_review::<SubmitSchema, _>(&path, &window_title, &IssueReviewHandler, &config.editor)?;

    if document.is_none() {
        std::process::exit(exit_code::ALREADY_OPEN);
    }

    // Check approval via .approve file rather than the document's frontmatter,
    // because on_review_complete may have stripped the temporary frontmatter
    // (for comment files), making the re-read document appear unapproved.
    let approval = crate::shared::human_in_the_loop::ApprovalManager::new(&path);
    if !approval.exists() {
        std::process::exit(exit_code::NOT_APPROVED);
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
    use indoc::indoc;
    use rstest::rstest;
    use tempfile::TempDir;

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

    #[rstest]
    #[case::with_frontmatter(indoc! {"
        ---
        title: Test
        ---
        Body"}, true)]
    #[case::without_frontmatter("<!-- author: user -->\nBody", false)]
    #[case::empty("", false)]
    fn test_has_yaml_frontmatter(#[case] content: &str, #[case] expected: bool) {
        assert_eq!(has_yaml_frontmatter(content), expected);
    }

    #[rstest]
    fn test_prepend_temporary_frontmatter() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("comment.md");
        std::fs::write(&path, "<!-- author: user -->\nBody").unwrap();

        prepend_temporary_frontmatter(&path).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(
            content,
            indoc! {"
                ---
                submit: false
                ---
                <!-- author: user -->
                Body"}
        );
    }

    #[rstest]
    #[case::submit_true(
        indoc! {"
            ---
            submit: true
            ---
            <!-- author: user -->
            Body"},
        indoc! {"
            <!-- author: user -->
            Body"}
    )]
    #[case::submit_false(
        indoc! {"
            ---
            submit: false
            ---
            <!-- author: user -->
            Body"},
        indoc! {"
            <!-- author: user -->
            Body"}
    )]
    #[case::with_extra_fields(
        indoc! {"
            ---
            title: Test
            submit: true
            ---
            Body"},
        indoc! {"
            ---
            title: Test
            submit: true
            ---
            Body"}
    )]
    #[case::no_frontmatter(
        indoc! {"
            <!-- author: user -->
            Body"},
        indoc! {"
            <!-- author: user -->
            Body"}
    )]
    fn test_strip_temporary_frontmatter(#[case] input: &str, #[case] expected: &str) {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("file.md");
        std::fs::write(&path, input).unwrap();

        strip_temporary_frontmatter(&path).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content, expected);
    }
}
