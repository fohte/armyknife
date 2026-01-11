use clap::Args;
use indoc::formatdoc;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use super::common::{DraftFile, Frontmatter, PrDraftError, RealCommandRunner, RepoInfo};
use crate::human_in_the_loop::{
    Document, DocumentSchema, Result as HilResult, ReviewHandler, complete_review, start_review,
};

#[derive(Args, Clone, PartialEq, Eq)]
pub struct ReviewArgs {
    /// Path to the draft file (auto-detected if not specified)
    pub filepath: Option<PathBuf>,
}

/// Internal command to complete the review process after Neovim exits.
/// This is called by WezTerm, not directly by users.
#[derive(Args, Clone, PartialEq, Eq)]
pub struct ReviewCompleteArgs {
    /// Path to the draft file
    pub filepath: PathBuf,

    /// tmux session to restore after review
    #[arg(long)]
    pub tmux_target: Option<String>,

    /// Window title for Neovim
    #[arg(long)]
    pub window_title: Option<String>,
}

/// Handler for PR draft review sessions.
pub struct PrDraftReviewHandler;

impl ReviewHandler<Frontmatter> for PrDraftReviewHandler {
    fn build_complete_args(
        &self,
        document_path: &Path,
        tmux_target: Option<&str>,
        window_title: &str,
    ) -> Vec<OsString> {
        let mut args: Vec<OsString> = vec![
            "ai".into(),
            "pr-draft".into(),
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

    fn on_review_complete(&self, document: &Document<Frontmatter>) -> HilResult<()> {
        if document.frontmatter.is_approved() {
            document.save_approval()?;
            println!(
                "{}",
                formatdoc! {"
                    PR approved. Run the following command to create the PR:

                        a ai pr-draft submit
                "}
            );
        } else {
            document.remove_approval()?;
            println!("PR not approved. Set 'steps.submit: true' and save to approve.");
        }

        Ok(())
    }
}

pub fn run(args: &ReviewArgs) -> Result<(), Box<dyn std::error::Error>> {
    let (draft_path, owner, repo, branch) = match &args.filepath {
        Some(path) => {
            let (owner, repo, branch) = DraftFile::parse_path(path).ok_or_else(|| {
                let display = path.display();
                PrDraftError::CommandFailed(format!("Invalid draft path: {display}"))
            })?;
            (path.clone(), owner, repo, branch)
        }
        None => {
            let repo_info = RepoInfo::from_git_only(&RealCommandRunner)?;
            let path = DraftFile::path_for(&repo_info);
            (path, repo_info.owner, repo_info.repo, repo_info.branch)
        }
    };

    let window_title = format!("PR: {owner}/{repo} @ {branch}");

    start_review::<Frontmatter, _>(&draft_path, &window_title, &PrDraftReviewHandler)?;

    Ok(())
}

pub fn run_complete(args: &ReviewCompleteArgs) -> Result<(), Box<dyn std::error::Error>> {
    complete_review::<Frontmatter, _>(
        &args.filepath,
        args.tmux_target.as_deref(),
        args.window_title.as_deref(),
        &PrDraftReviewHandler,
    )?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn build_review_args_should_roundtrip_non_utf_paths() {
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt;

        let mut filename = OsString::from_vec(vec![b'b', b'r', b'a', b'n', b'c', b'h', 0xff]);
        filename.push(".md");
        let draft_path = DraftFile::draft_dir()
            .join("owner")
            .join("repo")
            .join(std::path::PathBuf::from(filename));

        let handler = PrDraftReviewHandler;
        let args = handler.build_complete_args(&draft_path, Some("sess:1.0"), "Test Title");
        let restored = std::path::Path::new(&args[3]);
        assert_eq!(
            restored.as_os_str(),
            draft_path.as_os_str(),
            "Path should survive argument building without loss"
        );
    }
}
