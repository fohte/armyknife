//! Review command for gh-issue-agent.
//!
//! Generates a review document showing the changeset and opens it in an editor
//! via the HITL framework. The user must set `submit: true` to approve the push.

use std::ffi::OsString;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use clap::Args;
use indoc::formatdoc;
use serde::{Deserialize, Serialize};

use super::common::IssueContext;
use super::push::changeset::{ChangeSet, DetectOptions, LocalState, RemoteState};
use super::push::detect::{ConflictCheckInput, check_conflicts};
use crate::shared::config::load_config;
use crate::shared::diff::write_diff;
use crate::shared::human_in_the_loop::{
    Document, DocumentSchema, FifoSignalGuard, Result as HilResult, ReviewHandler, complete_review,
    start_review,
};

/// Review file name stored alongside issue data.
const REVIEW_FILENAME: &str = "review.md";

#[derive(Args, Clone, PartialEq, Eq, Debug)]
pub struct ReviewArgs {
    /// Issue number or path to new issue directory
    pub target: String,

    /// Target repository (owner/repo)
    #[arg(short = 'R', long)]
    pub repo: Option<String>,

    /// Allow overwriting remote changes (like git push --force)
    #[arg(long)]
    pub force: bool,

    /// Allow editing other users' comments
    #[arg(long)]
    pub edit_others: bool,

    /// Allow deleting comments from GitHub
    #[arg(long)]
    pub allow_delete: bool,
}

/// Internal command to complete the review process after the editor exits.
/// This is called by the terminal, not directly by users.
#[derive(Args, Clone, PartialEq, Eq, Debug)]
pub struct ReviewCompleteArgs {
    /// Path to the review.md file
    pub filepath: PathBuf,

    /// Issue number
    #[arg(long)]
    pub issue_number: u64,

    /// Target repository (owner/repo)
    #[arg(long)]
    pub repo: String,

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

/// Frontmatter for push review documents.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PushReviewFrontmatter {
    #[serde(default)]
    pub submit: bool,
}

impl DocumentSchema for PushReviewFrontmatter {
    fn is_approved(&self) -> bool {
        self.submit
    }
}

/// Handler for push review sessions.
struct PushReviewHandler {
    issue_number: u64,
    repo_slug: String,
}

impl ReviewHandler<PushReviewFrontmatter> for PushReviewHandler {
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
            "--issue-number".into(),
            self.issue_number.to_string().into(),
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

    fn on_review_complete(&self, document: &Document<PushReviewFrontmatter>) -> HilResult<()> {
        if document.frontmatter.is_approved() {
            document.save_approval()?;
            println!(
                "{}",
                formatdoc! {"
                    Push approved. Run the following command to push:

                        a gh issue-agent push {issue_number}
                ",
                    issue_number = self.issue_number,
                }
            );
        } else {
            document.remove_approval()?;
            println!("Push not approved. Set 'submit: true' and save to approve.");
        }

        Ok(())
    }
}

/// Get the review.md path for an issue.
pub fn review_path_for(storage_dir: &Path) -> PathBuf {
    storage_dir.join(REVIEW_FILENAME)
}

pub async fn run(args: &ReviewArgs) -> anyhow::Result<()> {
    let issue_number: u64 = args
        .target
        .parse()
        .map_err(|_| anyhow::anyhow!("Invalid issue number: '{}'", args.target))?;

    let issue_args = super::IssueArgs {
        issue_number,
        repo: args.repo.clone(),
    };
    let (ctx, client) = IssueContext::from_args(&issue_args).await?;

    // Fetch remote and local state
    let remote = ctx.fetch_remote(client).await?;
    let local = ctx.load_local()?;

    // Check for conflicts unless --force
    if !args.force {
        let conflict_input = ConflictCheckInput {
            local_metadata: &local.metadata,
            local_body: &local.body,
            local_comments: &local.comments,
            remote_issue: &remote.issue,
            remote_comments: &remote.comments,
        };
        let conflicts = check_conflicts(&conflict_input);

        if !conflicts.is_empty() {
            eprintln!();
            eprintln!(
                "Conflict detected! The following fields were edited both locally and remotely:"
            );
            for conflict in &conflicts {
                eprintln!("  - {}", conflict);
            }
            eprintln!();
            anyhow::bail!(
                "Remote has changed. Use --force to overwrite, or 'pull --force' to update local copy."
            );
        }
    }

    // Detect changes
    let local_state = LocalState {
        metadata: &local.metadata,
        body: &local.body,
        comments: &local.comments,
    };
    let remote_state = RemoteState {
        issue: &remote.issue,
        comments: &remote.comments,
    };
    let options = DetectOptions {
        current_user: &ctx.current_user,
        edit_others: args.edit_others,
        allow_delete: args.allow_delete,
    };
    let changeset = ChangeSet::detect(&local_state, &remote_state, &options)?;

    if !changeset.has_changes() {
        println!("No changes to push.");
        return Ok(());
    }

    // Generate review document
    let review_content = generate_review_document(&changeset, &ctx)?;
    let review_path = review_path_for(ctx.storage.dir());
    std::fs::write(&review_path, &review_content)?;

    // Start HITL review session
    let config = load_config()?;
    let repo_slug = format!("{}/{}", ctx.owner, ctx.repo_name);
    let window_title = format!("Push Review: {}#{}", repo_slug, ctx.issue_number);

    let handler = PushReviewHandler {
        issue_number: ctx.issue_number,
        repo_slug,
    };

    start_review::<PushReviewFrontmatter, _>(
        &review_path,
        &window_title,
        &handler,
        &config.editor,
    )?;

    Ok(())
}

pub fn run_complete(args: &ReviewCompleteArgs) -> anyhow::Result<()> {
    // Create FIFO guard first to ensure signaling even if load_config fails
    let _fifo_guard = args.done_fifo.as_deref().map(FifoSignalGuard::new);

    let config = load_config()?;

    let handler = PushReviewHandler {
        issue_number: args.issue_number,
        repo_slug: args.repo.clone(),
    };

    complete_review::<PushReviewFrontmatter, _>(
        &args.filepath,
        args.tmux_target.as_deref(),
        args.window_title.as_deref(),
        &handler,
        &config.editor,
    )?;

    Ok(())
}

/// Generate the review document content with changeset diff.
fn generate_review_document(
    changeset: &ChangeSet<'_>,
    ctx: &IssueContext,
) -> anyhow::Result<String> {
    let mut buf: Vec<u8> = Vec::new();

    // Frontmatter
    writeln!(buf, "---")?;
    writeln!(buf, "submit: false")?;
    writeln!(buf, "---")?;
    writeln!(buf)?;
    writeln!(
        buf,
        "# Push Review: {}/{}#{}",
        ctx.owner, ctx.repo_name, ctx.issue_number
    )?;
    writeln!(buf)?;
    writeln!(
        buf,
        "<!-- Review the changes below. Set 'submit: true' in the frontmatter to approve. -->"
    )?;

    if let Some(change) = &changeset.title {
        writeln!(buf)?;
        writeln!(buf, "## Title")?;
        writeln!(buf)?;
        writeln!(buf, "```diff")?;
        write_diff(&mut buf, change.remote, change.local, false)?;
        writeln!(buf, "```")?;
    }

    if let Some(change) = &changeset.body {
        writeln!(buf)?;
        writeln!(buf, "## Issue Body")?;
        writeln!(buf)?;
        writeln!(buf, "```diff")?;
        write_diff(&mut buf, change.remote, change.local, false)?;
        writeln!(buf, "```")?;
    }

    if let Some(change) = &changeset.labels {
        writeln!(buf)?;
        writeln!(buf, "## Labels")?;
        writeln!(buf)?;
        writeln!(buf, "```diff")?;
        writeln!(buf, "- {:?}", change.remote_sorted)?;
        writeln!(buf, "+ {:?}", change.local_sorted)?;
        writeln!(buf, "```")?;
    }

    if let Some(change) = &changeset.sub_issues {
        writeln!(buf)?;
        writeln!(buf, "## Sub-issues")?;
        writeln!(buf)?;
        writeln!(buf, "```diff")?;
        writeln!(buf, "- {:?}", change.remote_sorted)?;
        writeln!(buf, "+ {:?}", change.local_sorted)?;
        writeln!(buf, "```")?;
    }

    if let Some(change) = &changeset.parent_issue {
        writeln!(buf)?;
        writeln!(buf, "## Parent Issue")?;
        writeln!(buf)?;
        writeln!(buf, "```diff")?;
        writeln!(buf, "- {}", change.remote.as_deref().unwrap_or("(none)"))?;
        writeln!(buf, "+ {}", change.local.as_deref().unwrap_or("(none)"))?;
        writeln!(buf, "```")?;
    }

    for change in &changeset.comments {
        use super::push::changeset::CommentChange;
        match change {
            CommentChange::New { filename, body } => {
                writeln!(buf)?;
                writeln!(buf, "## New Comment: {}", filename)?;
                writeln!(buf)?;
                writeln!(buf, "```")?;
                writeln!(buf, "{}", body)?;
                writeln!(buf, "```")?;
            }
            CommentChange::Updated {
                filename,
                local_body,
                remote_body,
                author,
                current_user,
                ..
            } => {
                writeln!(buf)?;
                if author != current_user {
                    writeln!(buf, "## Comment: {} (author: {})", filename, author)?;
                } else {
                    writeln!(buf, "## Comment: {}", filename)?;
                }
                writeln!(buf)?;
                writeln!(buf, "```diff")?;
                write_diff(&mut buf, remote_body, local_body, false)?;
                writeln!(buf, "```")?;
            }
            CommentChange::Deleted {
                database_id,
                body,
                author,
            } => {
                writeln!(buf)?;
                writeln!(
                    buf,
                    "## Delete Comment: database_id={} (author: {})",
                    database_id, author
                )?;
                writeln!(buf)?;
                writeln!(buf, "```diff")?;
                for line in body.lines() {
                    writeln!(buf, "- {}", line)?;
                }
                writeln!(buf, "```")?;
            }
        }
    }

    Ok(String::from_utf8(buf)?)
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
            "gh", "issue-agent", "review-complete",
            "/tmp/review.md", "--issue-number", "42",
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
            "gh", "issue-agent", "review-complete",
            "/tmp/review.md", "--issue-number", "10",
            "--repo", "fohte/armyknife",
            "--window-title", "Title",
        ],
    )]
    fn build_review_args(
        #[case] issue_number: u64,
        #[case] tmux_target: Option<&str>,
        #[case] window_title: &str,
        #[case] expected: Vec<&str>,
    ) {
        let handler = PushReviewHandler {
            issue_number,
            repo_slug: "fohte/armyknife".to_string(),
        };

        let path = std::path::PathBuf::from("/tmp/review.md");
        let args = handler.build_complete_args(&path, tmux_target, window_title);

        let args_str: Vec<String> = args
            .iter()
            .map(|a| a.to_string_lossy().to_string())
            .collect();
        assert_eq!(args_str, expected);
    }

    #[rstest::rstest]
    fn test_review_path_for() {
        let dir = PathBuf::from("/tmp/cache/owner/repo/42");
        assert_eq!(
            review_path_for(&dir),
            PathBuf::from("/tmp/cache/owner/repo/42/review.md")
        );
    }
}
