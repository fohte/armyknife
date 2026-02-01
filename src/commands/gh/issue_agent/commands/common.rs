//! Common utilities shared across issue-agent commands.

use std::path::Path;

use super::IssueArgs;
use crate::commands::gh::issue_agent::models::{Comment, Issue, IssueMetadata};
use crate::commands::gh::issue_agent::storage::{IssueStorage, LocalComment};
use crate::infra::git;
use crate::infra::github::OctocrabClient;

// Re-export git::parse_repo for convenience.
// Note: This returns git::Result, callers using Box<dyn Error> can use `?` directly.
pub use git::parse_repo;

// Re-export diff utilities from shared module
pub use crate::shared::diff::{print_colored_line, print_diff, write_diff};

/// Get repository from argument or git remote origin.
///
/// If a repo argument is provided, returns it directly.
/// Otherwise, attempts to determine the repository from git remote origin.
pub fn get_repo_from_arg_or_git(repo_arg: &Option<String>) -> anyhow::Result<String> {
    if let Some(repo) = repo_arg {
        return Ok(repo.clone());
    }

    // Get from git remote origin
    let (owner, repo) = git::get_owner_repo().ok_or_else(|| {
        anyhow::anyhow!("Failed to determine current repository. Use -R to specify.")
    })?;

    Ok(format!("{}/{}", owner, repo))
}

/// Context for issue operations containing all necessary state.
pub struct IssueContext {
    pub owner: String,
    pub repo_name: String,
    pub issue_number: u64,
    pub storage: IssueStorage,
    pub current_user: String,
}

/// Remote state fetched from GitHub.
pub struct RemoteData {
    pub issue: Issue,
    pub comments: Vec<Comment>,
}

/// Local state loaded from storage.
pub struct LocalData {
    pub metadata: IssueMetadata,
    pub body: String,
    pub comments: Vec<LocalComment>,
}

impl IssueContext {
    /// Initialize context from IssueArgs, validating inputs and fetching current user.
    pub async fn from_args(args: &IssueArgs) -> anyhow::Result<(Self, &'static OctocrabClient)> {
        let repo = get_repo_from_arg_or_git(&args.repo)?;
        let issue_number = args.issue_number;

        // Validate repo format before making any API calls
        let (owner, repo_name) = parse_repo(&repo)?;

        let storage = IssueStorage::new(&repo, issue_number as i64);

        // Check if local cache exists
        if !storage.dir().exists() {
            anyhow::bail!(
                "Issue #{} not found locally. Run 'a gh issue-agent pull {}' first.",
                issue_number,
                issue_number
            );
        }

        println!("Fetching latest from GitHub...");

        let client = OctocrabClient::get()?;
        let current_user = client.get_current_user().await?;

        let ctx = Self {
            owner: owner.to_string(),
            repo_name: repo_name.to_string(),
            issue_number,
            storage,
            current_user,
        };

        Ok((ctx, client))
    }

    /// Fetch remote state from GitHub.
    pub async fn fetch_remote(&self, client: &OctocrabClient) -> anyhow::Result<RemoteData> {
        let issue = client
            .get_issue(&self.owner, &self.repo_name, self.issue_number)
            .await?;
        let comments = client
            .get_comments(&self.owner, &self.repo_name, self.issue_number)
            .await?;
        Ok(RemoteData { issue, comments })
    }

    /// Load local state from storage.
    pub fn load_local(&self) -> anyhow::Result<LocalData> {
        let metadata = self.storage.read_metadata()?;
        let body = self.storage.read_body()?;
        let comments = self.storage.read_comments()?;
        Ok(LocalData {
            metadata,
            body,
            comments,
        })
    }
}

/// Print success message after fetching issue.
pub fn print_fetch_success(issue_number: u64, title: &str, dir: &Path) {
    eprintln!();
    eprintln!(
        "Done! Issue #{issue_number} has been saved to {}/",
        dir.display()
    );
    eprintln!();
    eprintln!("Title: {title}");
    eprintln!();
    eprintln!("Files:");
    eprintln!(
        "  {}/issue.md          - Issue body (editable)",
        dir.display()
    );
    eprintln!(
        "  {}/metadata.json     - Metadata (editable: title, labels, assignees)",
        dir.display()
    );
    eprintln!(
        "  {}/comments/         - Comments (only your own comments are editable)",
        dir.display()
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    // parse_repo tests are in src/git/repo.rs

    mod get_repo_tests {
        use super::*;

        #[rstest]
        #[case::simple("owner/repo")]
        #[case::real_repo("fohte/armyknife")]
        #[case::with_special_chars("my-org/my_repo.rs")]
        fn test_with_arg_returns_as_is(#[case] repo: &str) {
            let result = get_repo_from_arg_or_git(&Some(repo.to_string())).unwrap();
            assert_eq!(result, repo);
        }
    }

    // diff tests are now in src/shared/diff.rs
}
