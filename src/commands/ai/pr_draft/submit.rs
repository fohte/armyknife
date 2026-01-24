use clap::Args;
use std::path::PathBuf;

use super::common::{DraftFile, PrDraftError, RepoInfo, contains_japanese};
use crate::infra::github::{CreatePrParams, OctocrabClient, PrClient, RepoClient};

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
}

/// Holds the target repository and branch information for PR creation
struct PrTarget {
    owner: String,
    repo: String,
    branch: String,
    is_private: bool,
}

pub async fn run(args: &SubmitArgs) -> anyhow::Result<()> {
    let client = OctocrabClient::get()?;
    run_impl(args, client).await
}

async fn run_impl(
    args: &SubmitArgs,
    gh_client: &(impl PrClient + RepoClient),
) -> anyhow::Result<()> {
    // Get draft path and target repo info
    let (draft_path, target) = match &args.filepath {
        Some(path) => {
            // When filepath is provided, parse it to get owner/repo/branch
            let (owner, repo, branch) = DraftFile::parse_path(path).ok_or_else(|| {
                PrDraftError::CommandFailed(format!("Invalid draft path: {}", path.display()))
            })?;
            let is_private = gh_client.is_repo_private(&owner, &repo).await?;
            (
                path.clone(),
                PrTarget {
                    owner,
                    repo,
                    branch,
                    is_private,
                },
            )
        }
        None => {
            // Auto-detect from current git repo
            let repo_info = RepoInfo::from_current_dir_async(gh_client).await?;
            let target = PrTarget {
                owner: repo_info.owner.clone(),
                repo: repo_info.repo.clone(),
                branch: repo_info.branch.clone(),
                is_private: repo_info.is_private,
            };
            (DraftFile::path_for(&repo_info), target)
        }
    };
    let is_private = target.is_private;

    // Check for lock (editor still open)
    if DraftFile::lock_path(&draft_path).exists() {
        anyhow::bail!("Please close the editor before submitting the PR.");
    }

    let draft = DraftFile::from_path(draft_path.clone())?;

    // Verify approval
    draft.verify_approval()?;

    // Validate title
    if draft.frontmatter.title.trim().is_empty() {
        return Err(PrDraftError::EmptyTitle.into());
    }

    // For public repos, check for Japanese characters
    if !is_private {
        if contains_japanese(&draft.frontmatter.title) {
            return Err(PrDraftError::JapaneseInTitle.into());
        }
        if contains_japanese(&draft.body) {
            return Err(PrDraftError::JapaneseInBody.into());
        }
    }

    // Create PR using GitHub API
    let pr_url = gh_client
        .create_pull_request(CreatePrParams {
            owner: target.owner.clone(),
            repo: target.repo.clone(),
            title: draft.frontmatter.title.clone(),
            body: draft.body.clone(),
            head: target.branch.clone(),
            base: args.base.clone(),
            draft: args.draft,
        })
        .await?;

    println!("{pr_url}");

    // Cleanup
    draft.cleanup()?;

    // Open PR in browser
    gh_client.open_in_browser(&pr_url);

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::github::GitHubMockServer;
    use indoc::indoc;
    use std::fs;

    struct TestEnv {
        mock: GitHubMockServer,
        owner: String,
        repo: String,
        draft_dir: std::path::PathBuf,
    }

    impl Drop for TestEnv {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.draft_dir);
        }
    }

    async fn setup_test_env(owner: &str, repo: &str) -> TestEnv {
        let mock = GitHubMockServer::start().await;
        let ctx = mock.repo(owner, repo);
        ctx.repo_info().private(true).get().await;
        ctx.pull_request(1).create().await;
        let draft_dir = DraftFile::draft_dir().join(owner).join(repo);
        if draft_dir.exists() {
            let _ = fs::remove_dir_all(&draft_dir);
        }
        TestEnv {
            mock,
            owner: owner.to_string(),
            repo: repo.to_string(),
            draft_dir,
        }
    }

    fn create_approved_draft(
        owner: &str,
        repo: &str,
        branch: &str,
        title: &str,
        body: &str,
    ) -> PathBuf {
        let repo_info = RepoInfo {
            owner: owner.to_string(),
            repo: repo.to_string(),
            branch: branch.to_string(),
            is_private: true,
        };
        let draft_path = DraftFile::path_for(&repo_info);
        if let Some(parent) = draft_path.parent() {
            fs::create_dir_all(parent).expect("create parent");
        }
        let content = format!(
            indoc! {r#"
                ---
                title: "{title}"
                steps:
                  submit: true
                ---
                {body}
            "#},
            title = title,
            body = body
        );
        fs::write(&draft_path, content).expect("write draft");
        DraftFile::from_path(draft_path.clone())
            .expect("draft file")
            .save_approval()
            .expect("save approval");
        draft_path
    }

    #[tokio::test]
    async fn submit_with_filepath_should_not_require_git_repo() {
        let env = setup_test_env("owner", "repo_submit_no_git").await;
        let draft_path = create_approved_draft(
            &env.owner,
            &env.repo,
            "feature/test",
            "Ready title",
            "Body content",
        );

        let args = SubmitArgs {
            filepath: Some(draft_path),
            base: None,
            draft: false,
        };

        let client = env.mock.client();
        let result = run_impl(&args, &client).await;

        assert!(
            result.is_ok(),
            "submit should work with GitHubMockServer: {:?}",
            result.err()
        );
    }

    #[tokio::test]
    async fn submit_with_filepath_should_use_draft_branch() {
        let env = setup_test_env("owner", "repo_submit_branch").await;
        let draft_path = create_approved_draft(
            &env.owner,
            &env.repo,
            "feature/missing-head",
            "Ready title",
            "Body content",
        );

        let args = SubmitArgs {
            filepath: Some(draft_path),
            base: None,
            draft: false,
        };

        let client = env.mock.client();
        let result = run_impl(&args, &client).await;

        assert!(
            result.is_ok(),
            "submit should pass the draft branch when --filepath is used: {:?}",
            result.err()
        );
    }

    #[tokio::test]
    async fn submit_fails_when_not_approved() {
        let env = setup_test_env("owner", "repo_submit_not_approved").await;
        let repo_info = RepoInfo {
            owner: env.owner.clone(),
            repo: env.repo.clone(),
            branch: "feature/unapproved".to_string(),
            is_private: true,
        };
        let draft_path = DraftFile::path_for(&repo_info);
        if let Some(parent) = draft_path.parent() {
            fs::create_dir_all(parent).expect("create parent");
        }
        fs::write(
            &draft_path,
            indoc! {r#"
                ---
                title: "Not approved"
                steps:
                  submit: false
                ---
                Body
            "#},
        )
        .expect("write draft");

        let args = SubmitArgs {
            filepath: Some(draft_path),
            base: None,
            draft: false,
        };

        let client = env.mock.client();
        let result = run_impl(&args, &client).await;

        assert!(result.is_err(), "submit should fail when not approved");
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("not approved") || err_msg.contains("NotApproved"),
            "error message should mention approval: {err_msg}"
        );
    }

    #[tokio::test]
    async fn submit_fails_with_empty_title() {
        let env = setup_test_env("owner", "repo_submit_empty_title").await;
        let repo_info = RepoInfo {
            owner: env.owner.clone(),
            repo: env.repo.clone(),
            branch: "feature/empty-title".to_string(),
            is_private: true,
        };
        let draft_path = DraftFile::path_for(&repo_info);
        if let Some(parent) = draft_path.parent() {
            fs::create_dir_all(parent).expect("create parent");
        }
        fs::write(
            &draft_path,
            indoc! {r#"
                ---
                title: ""
                steps:
                  submit: true
                ---
                Body
            "#},
        )
        .expect("write draft");
        DraftFile::from_path(draft_path.clone())
            .expect("draft file")
            .save_approval()
            .expect("save approval");

        let args = SubmitArgs {
            filepath: Some(draft_path),
            base: None,
            draft: false,
        };

        let client = env.mock.client();
        let result = run_impl(&args, &client).await;

        assert!(result.is_err(), "submit should fail with empty title");
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("title"),
            "error message should mention title: {err_msg}"
        );
    }
}
