use clap::Args;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

use super::common::{DraftFile, PrDraftError, RepoInfo, contains_japanese};

#[derive(Args, Clone, PartialEq, Eq)]
pub struct SubmitArgs {
    /// Path to the draft file (auto-detected if not specified)
    pub filepath: Option<PathBuf>,

    /// Base branch for the PR
    #[arg(long)]
    pub base: Option<String>,

    /// Create as draft PR
    #[arg(long)]
    pub draft: bool,

    /// Additional arguments to pass to `gh pr create`
    #[arg(last = true)]
    pub gh_args: Vec<String>,
}

pub fn run(args: &SubmitArgs) -> std::result::Result<(), Box<dyn std::error::Error>> {
    let repo_info = RepoInfo::from_current_dir()?;

    let draft_path = match &args.filepath {
        Some(path) => path.clone(),
        None => DraftFile::path_for(&repo_info),
    };

    // Check for lock (editor still open)
    if DraftFile::lock_path(&draft_path).exists() {
        return Err(Box::new(PrDraftError::CommandFailed(
            "Please close the editor before submitting the PR.".to_string(),
        )));
    }

    let draft = DraftFile::from_path(draft_path.clone())?;

    // Verify approval
    draft.verify_approval()?;

    // Validate title
    if draft.frontmatter.title.trim().is_empty() {
        return Err(Box::new(PrDraftError::EmptyTitle));
    }

    // For public repos, check for Japanese characters
    if !repo_info.is_private {
        if contains_japanese(&draft.frontmatter.title) {
            return Err(Box::new(PrDraftError::JapaneseInTitle));
        }
        if contains_japanese(&draft.body) {
            return Err(Box::new(PrDraftError::JapaneseInBody));
        }
    }

    // Create temp file for body
    let body_file = tempfile::Builder::new()
        .prefix("pr-body-")
        .suffix(".md")
        .tempfile()?;

    fs::write(body_file.path(), &draft.body)?;

    // Build gh pr create command
    let mut gh_args = vec![
        "pr".to_string(),
        "create".to_string(),
        "--title".to_string(),
        draft.frontmatter.title.clone(),
        "--body-file".to_string(),
        body_file.path().display().to_string(),
    ];

    if let Some(base) = &args.base {
        gh_args.push("--base".to_string());
        gh_args.push(base.clone());
    }

    if args.draft {
        gh_args.push("--draft".to_string());
    }

    gh_args.extend(args.gh_args.clone());

    // Create PR
    let output = Command::new("gh")
        .args(&gh_args)
        .output()
        .map_err(|e| PrDraftError::CommandFailed(format!("Failed to run gh: {e}")))?;

    if !output.status.success() {
        return Err(Box::new(PrDraftError::CommandFailed(format!(
            "gh pr create failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ))));
    }

    let pr_url = String::from_utf8_lossy(&output.stdout).trim().to_string();
    println!("{pr_url}");

    // Cleanup
    draft.cleanup()?;

    // Open PR in browser
    let _ = Command::new("gh").args(["pr", "view", "--web"]).status();

    Ok(())
}
