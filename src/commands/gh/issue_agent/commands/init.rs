//! Init command for creating new issue/comment boilerplate files.

use clap::{Args, Subcommand};

use super::common::{get_repo_from_arg_or_git, parse_repo};
use crate::commands::gh::issue_agent::storage::IssueStorage;

/// Arguments for the init command.
#[derive(Args, Clone, PartialEq, Eq, Debug)]
pub struct InitArgs {
    #[command(subcommand)]
    pub command: InitCommands,
}

/// Subcommands for init.
#[derive(Subcommand, Clone, PartialEq, Eq, Debug)]
pub enum InitCommands {
    /// Create a new issue boilerplate file
    Issue(InitIssueArgs),

    /// Create a new comment boilerplate file for an existing issue
    Comment(InitCommentArgs),
}

/// Arguments for `init issue`.
#[derive(Args, Clone, PartialEq, Eq, Debug)]
pub struct InitIssueArgs {
    /// Target repository (owner/repo)
    #[arg(short = 'R', long)]
    pub repo: Option<String>,
}

/// Arguments for `init comment`.
#[derive(Args, Clone, PartialEq, Eq, Debug)]
pub struct InitCommentArgs {
    /// Issue number
    pub issue_number: u64,

    /// Target repository (owner/repo)
    #[arg(short = 'R', long)]
    pub repo: Option<String>,

    /// Name for the comment file (default: timestamp)
    #[arg(long)]
    pub name: Option<String>,
}

/// Run the init command.
pub fn run(args: &InitArgs) -> anyhow::Result<()> {
    match &args.command {
        InitCommands::Issue(issue_args) => run_init_issue(issue_args),
        InitCommands::Comment(comment_args) => run_init_comment(comment_args),
    }
}

/// Initialize a new issue boilerplate file.
fn run_init_issue(args: &InitIssueArgs) -> anyhow::Result<()> {
    let repo = get_repo_from_arg_or_git(&args.repo)?;
    // Validate repo format to prevent path traversal
    let _ = parse_repo(&repo)?;
    let storage = IssueStorage::new_for_new_issue(&repo);

    let path = storage.init_new_issue()?;

    eprintln!("Created: {}", path.display());
    eprintln!();
    eprintln!(
        "Edit the file and run: a gh issue-agent push {}",
        storage.dir().display()
    );

    Ok(())
}

/// Validate comment name to prevent path traversal.
fn validate_comment_name(name: &str) -> anyhow::Result<()> {
    if name.contains('/') || name.contains('\\') || name.contains("..") {
        anyhow::bail!("Invalid comment name: must not contain '/', '\\', or '..'");
    }
    Ok(())
}

/// Initialize a new comment boilerplate file.
fn run_init_comment(args: &InitCommentArgs) -> anyhow::Result<()> {
    let repo = get_repo_from_arg_or_git(&args.repo)?;
    // Validate repo format to prevent path traversal
    let _ = parse_repo(&repo)?;

    // Validate comment name if provided
    if let Some(name) = &args.name {
        validate_comment_name(name)?;
    }

    let storage = IssueStorage::new(&repo, args.issue_number as i64);

    // Check if issue exists locally
    if !storage.dir().exists() {
        anyhow::bail!(
            "Issue #{} not found locally. Run 'a gh issue-agent pull {}' first.",
            args.issue_number,
            args.issue_number
        );
    }

    let path = storage.init_new_comment(args.name.as_deref())?;

    eprintln!("Created: {}", path.display());
    eprintln!();
    eprintln!(
        "Edit the file and run: a gh issue-agent push {}",
        args.issue_number
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;
    use std::fs;

    #[rstest]
    fn test_run_init_issue() {
        let dir = tempfile::tempdir().unwrap();
        let storage = IssueStorage::from_dir(dir.path());

        let result = storage.init_new_issue();
        assert!(result.is_ok());

        let path = result.unwrap();
        assert!(path.exists());

        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("labels: []"));
    }

    #[rstest]
    fn test_run_init_comment() {
        let dir = tempfile::tempdir().unwrap();
        let storage = IssueStorage::from_dir(dir.path());

        let result = storage.init_new_comment(Some("test"));
        assert!(result.is_ok());

        let path = result.unwrap();
        assert!(path.exists());
        assert!(path.to_string_lossy().contains("new_test.md"));
    }
}
