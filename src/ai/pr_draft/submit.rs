use clap::Args;
use std::ffi::OsString;
use std::io::Write;
use std::path::PathBuf;

use super::common::{
    DraftFile, GhRunner, PrDraftError, RealGhRunner, RepoInfo, check_is_private, contains_japanese,
};

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

/// Holds the target repository and branch information for PR creation
struct PrTarget {
    owner: String,
    repo: String,
    branch: String,
    is_private: bool,
}

pub fn run(args: &SubmitArgs) -> std::result::Result<(), Box<dyn std::error::Error>> {
    run_with_gh_runner(args, &RealGhRunner)
}

fn run_with_gh_runner(
    args: &SubmitArgs,
    gh_runner: &impl GhRunner,
) -> std::result::Result<(), Box<dyn std::error::Error>> {
    // Get draft path and target repo info
    let (draft_path, target) = match &args.filepath {
        Some(path) => {
            // When filepath is provided, parse it to get owner/repo/branch
            let (owner, repo, branch) = DraftFile::parse_path(path).ok_or_else(|| {
                PrDraftError::CommandFailed(format!("Invalid draft path: {}", path.display()))
            })?;
            let is_private = check_is_private(gh_runner, &owner, &repo)?;
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
            let repo_info = RepoInfo::from_current_dir(gh_runner)?;
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

    // Create temp file for body
    // Write to the existing handle for Windows compatibility (can't reopen path while held)
    let mut body_file = tempfile::Builder::new()
        .prefix("pr-body-")
        .suffix(".md")
        .tempfile()?;

    body_file.write_all(draft.body.as_bytes())?;
    body_file.flush()?;

    // Build gh pr create arguments
    let repo_spec = format!("{}/{}", target.owner, target.repo);
    let mut gh_args: Vec<OsString> = vec![
        "pr".into(),
        "create".into(),
        "--repo".into(),
        repo_spec.clone().into(),
        "--head".into(),
        target.branch.into(),
        "--title".into(),
        draft.frontmatter.title.clone().into(),
        "--body-file".into(),
        body_file.path().as_os_str().to_owned(),
    ];

    if let Some(base) = &args.base {
        gh_args.push("--base".into());
        gh_args.push(base.into());
    }

    if args.draft {
        gh_args.push("--draft".into());
    }

    for arg in &args.gh_args {
        gh_args.push(arg.into());
    }

    // Create PR
    let output = gh_runner
        .run_gh_with_args(&gh_args)
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

    // Open PR in browser using the returned URL
    gh_runner.open_in_browser(&repo_spec, &pr_url);

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::pr_draft::common::test_utils::MockGhRunner;
    use indoc::indoc;
    use std::fs;

    struct TestEnv {
        gh_runner: MockGhRunner,
        owner: String,
        repo: String,
        draft_dir: std::path::PathBuf,
    }

    impl Drop for TestEnv {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.draft_dir);
        }
    }

    fn setup_test_env(owner: &str, repo: &str, _branch: &str) -> TestEnv {
        let gh_runner = MockGhRunner::new();
        let draft_dir = DraftFile::draft_dir().join(owner).join(repo);
        if draft_dir.exists() {
            let _ = fs::remove_dir_all(&draft_dir);
        }
        TestEnv {
            gh_runner,
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

    #[test]
    fn submit_with_filepath_should_not_require_git_repo() {
        // Use unique repo name to avoid conflicts in parallel tests
        let env = setup_test_env("owner", "repo_submit_no_git", "feature/test");
        let draft_path = create_approved_draft(&env, "feature/test", "Ready title", "Body content");

        let args = SubmitArgs {
            filepath: Some(draft_path),
            base: None,
            draft: false,
            gh_args: vec![],
        };

        let result = run_with_gh_runner(&args, &env.gh_runner);

        assert!(
            result.is_ok(),
            "submit should work with MockGhRunner: {:?}",
            result.err()
        );
    }

    #[test]
    fn submit_with_filepath_should_use_draft_branch() {
        // Use unique repo name to avoid conflicts in parallel tests
        let env = setup_test_env("owner", "repo_submit_branch", "feature/missing-head");
        let draft_path =
            create_approved_draft(&env, "feature/missing-head", "Ready title", "Body content");

        let args = SubmitArgs {
            filepath: Some(draft_path),
            base: None,
            draft: false,
            gh_args: vec![],
        };

        let result = run_with_gh_runner(&args, &env.gh_runner);

        assert!(
            result.is_ok(),
            "submit should pass the draft branch to gh when --filepath is used: {:?}",
            result.err()
        );
    }

    #[test]
    fn submit_fails_when_not_approved() {
        let env = setup_test_env("owner", "repo_submit_not_approved", "feature/unapproved");
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
            gh_args: vec![],
        };

        let result = run_with_gh_runner(&args, &env.gh_runner);

        assert!(result.is_err(), "submit should fail when not approved");
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("not approved") || err_msg.contains("NotApproved"),
            "error message should mention approval: {err_msg}"
        );
    }

    #[test]
    fn submit_fails_with_empty_title() {
        let env = setup_test_env("owner", "repo_submit_empty_title", "feature/empty-title");
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
            gh_args: vec![],
        };

        let result = run_with_gh_runner(&args, &env.gh_runner);

        assert!(result.is_err(), "submit should fail with empty title");
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("title"),
            "error message should mention title: {err_msg}"
        );
    }
}
