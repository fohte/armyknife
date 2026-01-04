use clap::Args;
use std::io::Write;
use std::path::PathBuf;
use std::process::Command;

use super::common::{DraftFile, PrDraftError, RepoInfo, check_is_private, contains_japanese};

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
    // Get draft path and target repo info
    let (draft_path, target) = match &args.filepath {
        Some(path) => {
            // When filepath is provided, parse it to get owner/repo/branch
            let (owner, repo, branch) = DraftFile::parse_path(path).ok_or_else(|| {
                PrDraftError::CommandFailed(format!("Invalid draft path: {}", path.display()))
            })?;
            let is_private = check_is_private(&owner, &repo)?;
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
            let repo_info = RepoInfo::from_current_dir()?;
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

    // Build gh pr create command using builder pattern
    let repo_spec = format!("{}/{}", target.owner, target.repo);
    let mut gh_cmd = Command::new("gh");
    gh_cmd
        .args([
            "pr",
            "create",
            "--repo",
            &repo_spec,
            "--head",
            &target.branch,
        ])
        .arg("--title")
        .arg(&draft.frontmatter.title)
        .arg("--body-file")
        .arg(body_file.path());

    if let Some(base) = &args.base {
        gh_cmd.arg("--base").arg(base);
    }

    if args.draft {
        gh_cmd.arg("--draft");
    }

    gh_cmd.args(&args.gh_args);

    // Create PR
    let output = gh_cmd
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

    // Open PR in browser using the returned URL
    let _ = Command::new("gh")
        .args(["pr", "view", "--web", "--repo", &repo_spec, &pr_url])
        .status();

    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use indoc::indoc;
    use serial_test::serial;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use tempfile::tempdir;

    #[test]
    #[serial]
    fn submit_with_filepath_should_not_require_git_repo() {
        let temp_cwd = tempdir().expect("tempdir");
        let _cwd_guard = WorkingDirGuard::change(temp_cwd.path());

        let gh_stub_dir = tempdir().expect("gh stub dir");
        let gh_stub_path = gh_stub_dir.path().join("gh");
        fs::write(
            &gh_stub_path,
            indoc! {r#"
            #!/bin/sh
            # fake gh
            if [ "$1" = "repo" ] && [ "$2" = "view" ]; then
              echo "true"
              exit 0
            fi
            if [ "$1" = "pr" ] && [ "$2" = "create" ]; then
              echo https://example.com/pr/1
              exit 0
            fi
            if [ "$1" = "pr" ] && [ "$2" = "view" ]; then
              exit 0
            fi
            exit 0
        "#},
        )
        .expect("write gh stub");
        let mut perms = fs::metadata(&gh_stub_path).expect("metadata").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&gh_stub_path, perms).expect("chmod");
        let _path_guard = PathGuard::prepend(gh_stub_dir.path());

        let repo_info = RepoInfo {
            owner: "owner".into(),
            repo: "repo".into(),
            branch: "feature/test".into(),
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
                title: "Ready title"
                steps:
                  submit: true
                ---
                Body content
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

        let result = run(&args);

        assert!(
            result.is_ok(),
            "submit should work even when cwd is not inside a git repo"
        );
    }

    #[test]
    #[serial]
    fn submit_with_filepath_should_use_draft_branch() {
        let temp_cwd = tempdir().expect("tempdir");
        let _cwd_guard = WorkingDirGuard::change(temp_cwd.path());

        let gh_stub_dir = tempdir().expect("gh stub dir");
        let gh_stub_path = gh_stub_dir.path().join("gh");
        fs::write(
            &gh_stub_path,
            indoc! {r#"
            #!/bin/sh
            # fake gh that requires --head
            if [ "$1" = "repo" ] && [ "$2" = "view" ]; then
              echo "true"
              exit 0
            fi
            if [ "$1" = "pr" ] && [ "$2" = "create" ]; then
              head_flag=0
              while [ "$#" -gt 0 ]; do
                if [ "$1" = "--head" ]; then
                  head_flag=1
                fi
                shift
              done
              if [ "$head_flag" -eq 0 ]; then
                echo "missing --head argument" >&2
                exit 42
              fi
              echo https://example.com/pr/1
              exit 0
            fi
            if [ "$1" = "pr" ] && [ "$2" = "view" ]; then
              exit 0
            fi
            echo "unexpected gh invocation: $@" >&2
            exit 1
        "#},
        )
        .expect("write gh stub");
        let mut perms = fs::metadata(&gh_stub_path).expect("metadata").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&gh_stub_path, perms).expect("chmod");
        let _path_guard = PathGuard::prepend(gh_stub_dir.path());

        let repo_info = RepoInfo {
            owner: "owner".into(),
            repo: "repo".into(),
            branch: "feature/missing-head".into(),
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
                title: "Ready title"
                steps:
                  submit: true
                ---
                Body content
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

        let result = run(&args);

        assert!(
            result.is_ok(),
            "submit should pass the draft branch to gh when --filepath is used"
        );
    }

    struct WorkingDirGuard {
        original: std::path::PathBuf,
    }

    impl WorkingDirGuard {
        fn change(path: &std::path::Path) -> Self {
            let original = std::env::current_dir().expect("cwd");
            std::env::set_current_dir(path).expect("set cwd");
            Self { original }
        }
    }

    impl Drop for WorkingDirGuard {
        fn drop(&mut self) {
            let _ = std::env::set_current_dir(&self.original);
        }
    }

    struct PathGuard {
        original: Option<String>,
    }

    impl PathGuard {
        fn prepend(dir: &std::path::Path) -> Self {
            let original = std::env::var("PATH").ok();
            let new_path = if let Some(ref current) = original {
                format!("{}:{}", dir.display(), current)
            } else {
                dir.display().to_string()
            };
            unsafe {
                std::env::set_var("PATH", new_path);
            }
            Self { original }
        }
    }

    impl Drop for PathGuard {
        fn drop(&mut self) {
            if let Some(ref original) = self.original {
                unsafe {
                    std::env::set_var("PATH", original);
                }
            } else {
                unsafe {
                    std::env::remove_var("PATH");
                }
            }
        }
    }
}
