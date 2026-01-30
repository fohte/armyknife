use clap::Args;
use std::fs;

use super::common::{
    DraftFile, PrDraftError, RepoInfo, generate_frontmatter, read_stdin_if_available,
};
use crate::infra::github::{OctocrabClient, RepoClient};
use crate::shared::diff::eprint_diff;

#[derive(Args, Clone, PartialEq, Eq)]
pub struct NewArgs {
    /// PR title
    #[arg(long)]
    pub title: Option<String>,

    /// Overwrite existing draft file if it exists
    #[arg(long)]
    pub force: bool,
}

pub async fn run(args: &NewArgs) -> anyhow::Result<()> {
    let client = OctocrabClient::get()?;
    run_impl(args, client).await
}

async fn run_impl(args: &NewArgs, gh_client: &impl RepoClient) -> anyhow::Result<()> {
    let repo_info = RepoInfo::from_git_only()?;
    let draft_path = DraftFile::path_for(&repo_info);

    // Check if the draft file already exists
    let old_content = if draft_path.exists() {
        if !args.force {
            return Err(PrDraftError::FileAlreadyExists(draft_path).into());
        }
        Some(fs::read_to_string(&draft_path)?)
    } else {
        None
    };

    // Create parent directories
    if let Some(parent) = draft_path.parent() {
        fs::create_dir_all(parent)?;
    }

    // Check if the repo is private (defaults to true if network is unavailable)
    let is_private = match gh_client
        .is_repo_private(&repo_info.owner, &repo_info.repo)
        .await
    {
        Ok(private) => private,
        Err(e) => {
            eprintln!("Warning: Failed to check repository visibility, assuming private: {e}");
            true
        }
    };

    let title = args.title.as_deref().unwrap_or("");
    let frontmatter = generate_frontmatter(title, is_private);

    let body = read_stdin_if_available().unwrap_or_default();
    let content = format!("{frontmatter}{body}");

    // Show warning and diff when overwriting
    if let Some(old) = &old_content {
        eprintln!(
            "Warning: Overwriting existing draft file: {}",
            draft_path.display()
        );
        eprintln!();
        eprint_diff(old, &content);
        eprintln!();
    }

    fs::write(&draft_path, content)?;

    println!("{}", draft_path.display());

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::git::test_utils::TempRepo;
    use crate::infra::github::GitHubMockServer;
    use crate::shared::diff::format_diff;
    use rstest::rstest;
    use std::fs;
    use std::path::Path;

    struct TestEnv {
        mock: GitHubMockServer,
        temp_repo: TempRepo,
        draft_dir: std::path::PathBuf,
    }

    impl Drop for TestEnv {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.draft_dir);
        }
    }

    async fn setup_test_env(owner: &str, repo: &str, is_private: bool) -> TestEnv {
        let mock = GitHubMockServer::start().await;
        mock.repo(owner, repo)
            .repo_info()
            .private(is_private)
            .get()
            .await;
        let temp_repo = TempRepo::new(owner, repo, "feature-test");

        let draft_dir = DraftFile::draft_dir().join(owner).join(repo);
        if draft_dir.exists() {
            let _ = fs::remove_dir_all(&draft_dir);
        }
        TestEnv {
            mock,
            temp_repo,
            draft_dir,
        }
    }

    /// Run new command with a specific repo path using mock GitHub client
    async fn run_with_mock(
        args: &NewArgs,
        repo_path: &Path,
        gh_client: &impl RepoClient,
    ) -> std::result::Result<(), Box<dyn std::error::Error>> {
        let repo_info = RepoInfo::from_path(repo_path)?;
        let draft_path = DraftFile::path_for(&repo_info);

        // Check if the draft file already exists
        let old_content = if draft_path.exists() {
            if !args.force {
                return Err(PrDraftError::FileAlreadyExists(draft_path).into());
            }
            Some(fs::read_to_string(&draft_path)?)
        } else {
            None
        };

        // Create parent directories
        if let Some(parent) = draft_path.parent() {
            fs::create_dir_all(parent)?;
        }

        // Check if the repo is private using mock client
        let is_private = gh_client
            .is_repo_private(&repo_info.owner, &repo_info.repo)
            .await
            .unwrap_or(true);

        let title = args.title.as_deref().unwrap_or("");
        let frontmatter = generate_frontmatter(title, is_private);

        let body = read_stdin_if_available().unwrap_or_default();
        let content = format!("{frontmatter}{body}");

        // Show warning and diff when overwriting
        if let Some(old) = &old_content {
            eprintln!(
                "Warning: Overwriting existing draft file: {}",
                draft_path.display()
            );
            eprintln!();
            eprint_diff(old, &content);
            eprintln!();
        }

        fs::write(&draft_path, content)?;

        println!("{}", draft_path.display());

        Ok(())
    }

    #[rstest]
    #[case::private(true, false)]
    #[case::public(false, true)]
    #[tokio::test]
    async fn new_generates_correct_frontmatter(
        #[case] is_private: bool,
        #[case] expect_ready_for_translation: bool,
    ) {
        // Use unique repo name to avoid conflicts in parallel tests
        let repo = format!("repo_frontmatter_{}", is_private);
        let env = setup_test_env("owner", &repo, is_private).await;

        let client = env.mock.client();
        run_with_mock(
            &NewArgs {
                title: Some("Test Title".to_string()),
                force: false,
            },
            &env.temp_repo.path(),
            &client,
        )
        .await
        .expect("run should succeed");

        let repo_info = RepoInfo::from_path(&env.temp_repo.path()).unwrap();
        let draft_path = DraftFile::path_for(&repo_info);
        let content = fs::read_to_string(&draft_path).expect("read draft");

        assert_eq!(
            content.contains("ready-for-translation"),
            expect_ready_for_translation,
            "expected ready-for-translation={expect_ready_for_translation}, got:\n{content}"
        );
    }

    #[tokio::test]
    async fn new_fails_when_file_exists_without_force() {
        let env = setup_test_env("owner", "repo_exists_no_force", true).await;

        let client = env.mock.client();
        run_with_mock(
            &NewArgs {
                title: Some("First Title".to_string()),
                force: false,
            },
            &env.temp_repo.path(),
            &client,
        )
        .await
        .expect("first run should succeed");

        let repo_info = RepoInfo::from_path(&env.temp_repo.path()).unwrap();
        let draft_path = DraftFile::path_for(&repo_info);
        assert!(
            draft_path.exists(),
            "draft file should exist after first run"
        );

        let result = run_with_mock(
            &NewArgs {
                title: Some("Second Title".to_string()),
                force: false,
            },
            &env.temp_repo.path(),
            &client,
        )
        .await;
        assert!(result.is_err(), "second run without --force should fail");

        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("already exists"),
            "error message should mention 'already exists': {err_msg}"
        );
        assert!(
            err_msg.contains("--force"),
            "error message should mention '--force': {err_msg}"
        );

        let content = fs::read_to_string(&draft_path).expect("read draft");
        assert!(
            content.contains("First Title"),
            "original content should be preserved: {content}"
        );
    }

    #[rstest]
    #[case::line_changed("old line\n", "new line\n", "-old line\n+new line\n")]
    #[case::line_added("line1\n", "line1\nline2\n", " line1\n+line2\n")]
    #[case::line_removed("line1\nline2\n", "line1\n", " line1\n-line2\n")]
    #[case::identical("same\n", "same\n", " same\n")]
    fn format_diff_generates_unified_diff(
        #[case] old: &str,
        #[case] new: &str,
        #[case] expected: &str,
    ) {
        let result = format_diff(old, new, false);
        assert_eq!(result, expected);
    }

    #[test]
    fn format_diff_with_color_includes_ansi_codes() {
        let result = format_diff("old\n", "new\n", true);
        // Should contain ANSI escape sequences
        assert!(result.contains("\x1b["));
    }

    #[tokio::test]
    async fn new_overwrites_when_file_exists_with_force() {
        let env = setup_test_env("owner", "repo_overwrite", true).await;

        let client = env.mock.client();
        run_with_mock(
            &NewArgs {
                title: Some("First Title".to_string()),
                force: false,
            },
            &env.temp_repo.path(),
            &client,
        )
        .await
        .expect("first run should succeed");

        run_with_mock(
            &NewArgs {
                title: Some("Second Title".to_string()),
                force: true,
            },
            &env.temp_repo.path(),
            &client,
        )
        .await
        .expect("second run with --force should succeed");

        let repo_info = RepoInfo::from_path(&env.temp_repo.path()).unwrap();
        let draft_path = DraftFile::path_for(&repo_info);
        let content = fs::read_to_string(&draft_path).expect("read draft");
        assert!(
            content.contains("Second Title"),
            "content should be overwritten: {content}"
        );
    }
}
