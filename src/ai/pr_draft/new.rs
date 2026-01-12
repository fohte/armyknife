use clap::Args;
use similar::{ChangeTag, TextDiff};
use std::fs;

use super::common::{
    DraftFile, GhRunner, PrDraftError, RealGhRunner, RepoInfo, check_is_private,
    generate_frontmatter, read_stdin_if_available,
};

#[derive(Args, Clone, PartialEq, Eq)]
pub struct NewArgs {
    /// PR title
    #[arg(long)]
    pub title: Option<String>,

    /// Overwrite existing draft file if it exists
    #[arg(long)]
    pub force: bool,
}

pub fn run(args: &NewArgs) -> std::result::Result<(), Box<dyn std::error::Error>> {
    run_with_gh_runner(args, &RealGhRunner)
}

fn run_with_gh_runner(
    args: &NewArgs,
    gh_runner: &impl GhRunner,
) -> std::result::Result<(), Box<dyn std::error::Error>> {
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
    let is_private = match check_is_private(gh_runner, &repo_info.owner, &repo_info.repo) {
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
        print_diff(old, &content);
        eprintln!();
    }

    fs::write(&draft_path, content)?;

    println!("{}", draft_path.display());

    Ok(())
}

fn print_diff(old: &str, new: &str) {
    eprint!("{}", format_diff(old, new, true));
}

fn format_diff(old: &str, new: &str, use_color: bool) -> String {
    let diff = TextDiff::from_lines(old, new);
    let mut output = String::new();

    for change in diff.iter_all_changes() {
        let (sign, prefix, suffix) = match change.tag() {
            ChangeTag::Delete if use_color => ('-', "\x1b[31m", "\x1b[0m"),
            ChangeTag::Insert if use_color => ('+', "\x1b[32m", "\x1b[0m"),
            ChangeTag::Delete => ('-', "", ""),
            ChangeTag::Insert => ('+', "", ""),
            ChangeTag::Equal => (' ', "", ""),
        };
        output.push_str(&format!("{prefix}{sign}{change}{suffix}"));
    }

    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::pr_draft::common::test_utils::MockGhRunner;
    use crate::git::test_utils::TempRepo;
    use rstest::rstest;
    use std::fs;
    use std::path::Path;

    struct TestEnv {
        gh_runner: MockGhRunner,
        temp_repo: TempRepo,
        draft_dir: std::path::PathBuf,
    }

    impl Drop for TestEnv {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.draft_dir);
        }
    }

    fn setup_test_env(owner: &str, repo: &str, is_private: Option<bool>) -> TestEnv {
        let gh_runner = MockGhRunner::new().with_private(is_private);
        let temp_repo = TempRepo::new(owner, repo, "feature-test");

        let draft_dir = DraftFile::draft_dir().join(owner).join(repo);
        if draft_dir.exists() {
            let _ = fs::remove_dir_all(&draft_dir);
        }
        TestEnv {
            gh_runner,
            temp_repo,
            draft_dir,
        }
    }

    /// Run new command with a specific repo path (for testing)
    fn run_with_path(
        args: &NewArgs,
        repo_path: &Path,
        gh_runner: &impl GhRunner,
    ) -> std::result::Result<(), Box<dyn std::error::Error>> {
        // First get repo info without network (is_private = false)
        let mut repo_info = RepoInfo::from_path(repo_path, None::<&MockGhRunner>)?;

        // Then check is_private, defaulting to true on error (same as production code)
        repo_info.is_private =
            check_is_private(gh_runner, &repo_info.owner, &repo_info.repo).unwrap_or(true);

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

        let is_private = repo_info.is_private;

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
            print_diff(old, &content);
            eprintln!();
        }

        fs::write(&draft_path, content)?;

        println!("{}", draft_path.display());

        Ok(())
    }

    #[rstest]
    #[case::offline(None, false)]
    #[case::private(Some(true), false)]
    #[case::public(Some(false), true)]
    fn new_generates_correct_frontmatter(
        #[case] is_private: Option<bool>,
        #[case] expect_ready_for_translation: bool,
    ) {
        // Use unique repo name to avoid conflicts in parallel tests
        let repo = format!(
            "repo_frontmatter_{}",
            is_private.map_or("offline".to_string(), |b| b.to_string())
        );
        let env = setup_test_env("owner", &repo, is_private);

        run_with_path(
            &NewArgs {
                title: Some("Test Title".to_string()),
                force: false,
            },
            &env.temp_repo.path(),
            &env.gh_runner,
        )
        .expect("run should succeed");

        let repo_info = RepoInfo::from_path(&env.temp_repo.path(), None::<&MockGhRunner>).unwrap();
        let draft_path = DraftFile::path_for(&repo_info);
        let content = fs::read_to_string(&draft_path).expect("read draft");

        assert_eq!(
            content.contains("ready-for-translation"),
            expect_ready_for_translation,
            "expected ready-for-translation={expect_ready_for_translation}, got:\n{content}"
        );
    }

    #[test]
    fn new_fails_when_file_exists_without_force() {
        let env = setup_test_env("owner", "repo_exists_no_force", Some(true));

        run_with_path(
            &NewArgs {
                title: Some("First Title".to_string()),
                force: false,
            },
            &env.temp_repo.path(),
            &env.gh_runner,
        )
        .expect("first run should succeed");

        let repo_info = RepoInfo::from_path(&env.temp_repo.path(), None::<&MockGhRunner>).unwrap();
        let draft_path = DraftFile::path_for(&repo_info);
        assert!(
            draft_path.exists(),
            "draft file should exist after first run"
        );

        let result = run_with_path(
            &NewArgs {
                title: Some("Second Title".to_string()),
                force: false,
            },
            &env.temp_repo.path(),
            &env.gh_runner,
        );
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
        assert!(result.contains("\x1b[31m"), "should contain red color code");
        assert!(
            result.contains("\x1b[32m"),
            "should contain green color code"
        );
        assert!(result.contains("\x1b[0m"), "should contain reset code");
    }

    #[test]
    fn new_overwrites_when_file_exists_with_force() {
        let env = setup_test_env("owner", "repo_overwrite", Some(true));

        run_with_path(
            &NewArgs {
                title: Some("First Title".to_string()),
                force: false,
            },
            &env.temp_repo.path(),
            &env.gh_runner,
        )
        .expect("first run should succeed");

        run_with_path(
            &NewArgs {
                title: Some("Second Title".to_string()),
                force: true,
            },
            &env.temp_repo.path(),
            &env.gh_runner,
        )
        .expect("second run with --force should succeed");

        let repo_info = RepoInfo::from_path(&env.temp_repo.path(), None::<&MockGhRunner>).unwrap();
        let draft_path = DraftFile::path_for(&repo_info);
        let content = fs::read_to_string(&draft_path).expect("read draft");
        assert!(
            content.contains("Second Title"),
            "content should be overwritten: {content}"
        );
    }
}
