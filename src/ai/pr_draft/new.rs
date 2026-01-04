use clap::Args;
use std::fs;

use super::common::{DraftFile, RepoInfo, generate_frontmatter, read_stdin_if_available};

#[derive(Args, Clone, PartialEq, Eq)]
pub struct NewArgs {
    /// PR title
    #[arg(long)]
    pub title: Option<String>,
}

pub fn run(args: &NewArgs) -> std::result::Result<(), Box<dyn std::error::Error>> {
    let repo_info = RepoInfo::from_current_dir()?;
    let draft_path = DraftFile::path_for(&repo_info);

    // Create parent directories
    if let Some(parent) = draft_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let title = args.title.as_deref().unwrap_or("");
    let frontmatter = generate_frontmatter(title, repo_info.is_private);

    let body = read_stdin_if_available().unwrap_or_default();
    let content = format!("{frontmatter}{body}");

    fs::write(&draft_path, content)?;

    println!("{}", draft_path.display());

    Ok(())
}
