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

pub async fn run(args: &SubmitArgs) -> std::result::Result<(), Box<dyn std::error::Error>> {
    let client = OctocrabClient::get()?;
    run_impl(args, client).await
}

async fn run_impl(
    args: &SubmitArgs,
    gh_client: &(impl PrClient + RepoClient),
) -> std::result::Result<(), Box<dyn std::error::Error>> {
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
    if !is_private {
        if contains_japanese(&draft.frontmatter.title) {
            return Err(Box::new(PrDraftError::JapaneseInTitle));
        }
        if contains_japanese(&draft.body) {
            return Err(Box::new(PrDraftError::JapaneseInBody));
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
    use crate::infra::github::MockGitHubClient;
    use indoc::indoc;
    use std::fs;

    struct TestEnv {
        gh_client: MockGitHubClient,
        owner: String,
        repo: String,
        draft_dir: std::path::PathBuf,
    }

    impl Drop for TestEnv {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.draft_dir);
        }
    }

    fn setup_test_env(owner: &str, repo: &str) -> TestEnv {
        let gh_client = MockGitHubClient::new().with_private(owner, repo, true);
        let draft_dir = DraftFile::draft_dir().join(owner).join(repo);
        if draft_dir.exists() {
            let _ = fs::remove_dir_all(&draft_dir);
        }
        TestEnv {
            gh_client,
            owner: owner.to_string(),
            repo: repo.to_string(),
            draft_dir,
        }
    }

    fn create_approved_draft(env: &TestEnv, branch: &str, title: &str, body: &str) -> PathBuf {
        let repo_info = RepoInfo {
            owner: env.owner.clone(),
            repo: env.repo.clone(),
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
        let env = setup_test_env("owner", "repo_submit_no_git");
        let draft_path = create_approved_draft(&env, "feature/test", "Ready title", "Body content");

        let args = SubmitArgs {
            filepath: Some(draft_path),
            base: None,
            draft: false,
        };

        let result = run_impl(&args, &env.gh_client).await;

        assert!(
            result.is_ok(),
            "submit should work with MockGitHubClient: {:?}",
            result.err()
        );

        // Verify PR was created
        let created = env.gh_client.created_prs.lock().unwrap();
        assert_eq!(created.len(), 1);
        assert_eq!(created[0].title, "Ready title");
    }

    #[tokio::test]
    async fn submit_with_filepath_should_use_draft_branch() {
        let env = setup_test_env("owner", "repo_submit_branch");
        let draft_path =
            create_approved_draft(&env, "feature/missing-head", "Ready title", "Body content");

        let args = SubmitArgs {
            filepath: Some(draft_path),
            base: None,
            draft: false,
        };

        let result = run_impl(&args, &env.gh_client).await;

        assert!(
            result.is_ok(),
            "submit should pass the draft branch when --filepath is used: {:?}",
            result.err()
        );

        // Verify correct branch was used
        let created = env.gh_client.created_prs.lock().unwrap();
        assert_eq!(created[0].head, "feature/missing-head");
    }

    #[tokio::test]
    async fn submit_fails_when_not_approved() {
        let env = setup_test_env("owner", "repo_submit_not_approved");
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

        let result = run_impl(&args, &env.gh_client).await;

        assert!(result.is_err(), "submit should fail when not approved");
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("not approved") || err_msg.contains("NotApproved"),
            "error message should mention approval: {err_msg}"
        );
    }

    #[tokio::test]
    async fn submit_fails_with_empty_title() {
        let env = setup_test_env("owner", "repo_submit_empty_title");
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

        let result = run_impl(&args, &env.gh_client).await;

        assert!(result.is_err(), "submit should fail with empty title");
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("title"),
            "error message should mention title: {err_msg}"
        );
    }
}
