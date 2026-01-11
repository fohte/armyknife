use clap::Args;
use similar::{ChangeTag, TextDiff};
use std::fs;

use super::common::{
    DraftFile, PrDraftError, RepoInfo, check_is_private, generate_frontmatter,
    read_stdin_if_available,
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
    let is_private = match check_is_private(&repo_info.owner, &repo_info.repo) {
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

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use indoc::indoc;
    use rstest::rstest;
    use serial_test::serial;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use tempfile::tempdir;

    fn create_git_stub(stub_dir: &std::path::Path, test_id: &str) {
        let git_stub = stub_dir.join("git");
        let script = format!(
            r#"#!/bin/sh
if [ "$1" = "rev-parse" ]; then
  echo "feature/test"
  exit 0
fi
if [ "$1" = "remote" ] && [ "$2" = "get-url" ]; then
  echo "https://github.com/owner/{}.git"
  exit 0
fi
echo "unexpected git command: $@" >&2
exit 1
"#,
            test_id
        );
        fs::write(&git_stub, script).expect("write git stub");
        let mut perms = fs::metadata(&git_stub).expect("metadata").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&git_stub, perms).expect("chmod");
    }

    fn create_gh_stub(stub_dir: &std::path::Path, is_private: Option<bool>) {
        let gh_stub = stub_dir.join("gh");
        let script = match is_private {
            Some(true) => indoc! {r#"
                #!/bin/sh
                echo "true"
                exit 0
            "#},
            Some(false) => indoc! {r#"
                #!/bin/sh
                echo "false"
                exit 0
            "#},
            None => indoc! {r#"
                #!/bin/sh
                echo "offline" >&2
                exit 2
            "#},
        };
        fs::write(&gh_stub, script).expect("write gh stub");
        let mut perms = fs::metadata(&gh_stub).expect("metadata").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&gh_stub, perms).expect("chmod");
    }

    struct TestEnv {
        test_id: String,
        _temp_cwd: tempfile::TempDir,
        _cwd_guard: WorkingDirGuard,
        _stub_dir: tempfile::TempDir,
        _git_guard: EnvGuard,
        _gh_guard: EnvGuard,
    }

    impl Drop for TestEnv {
        fn drop(&mut self) {
            // Clean up the test-specific draft directory
            let draft_dir = DraftFile::draft_dir().join("owner").join(&self.test_id);
            let _ = fs::remove_dir_all(&draft_dir);
        }
    }

    fn setup_test_env(test_id: &str, gh_response: Option<bool>) -> TestEnv {
        // Clean up any existing draft file from previous test runs
        let draft_dir = DraftFile::draft_dir().join("owner").join(test_id);
        if draft_dir.exists() {
            let _ = fs::remove_dir_all(&draft_dir);
        }

        let temp_cwd = tempdir().expect("tempdir");
        let cwd_guard = WorkingDirGuard::change(temp_cwd.path());

        let stub_dir = tempdir().expect("stub dir");
        create_git_stub(stub_dir.path(), test_id);
        create_gh_stub(stub_dir.path(), gh_response);

        let git_guard = EnvGuard::set("ARMYKNIFE_GIT_PATH", stub_dir.path().join("git"));
        let gh_guard = EnvGuard::set("ARMYKNIFE_GH_PATH", stub_dir.path().join("gh"));

        TestEnv {
            test_id: test_id.to_string(),
            _temp_cwd: temp_cwd,
            _cwd_guard: cwd_guard,
            _stub_dir: stub_dir,
            _git_guard: git_guard,
            _gh_guard: gh_guard,
        }
    }

    #[rstest]
    #[case::offline("frontmatter_offline", None, false)]
    #[case::private("frontmatter_private", Some(true), false)]
    #[case::public("frontmatter_public", Some(false), true)]
    #[serial(pr_draft)]
    fn new_generates_correct_frontmatter(
        #[case] test_id: &str,
        #[case] gh_response: Option<bool>,
        #[case] expect_ready_for_translation: bool,
    ) {
        let _env = setup_test_env(test_id, gh_response);

        run(&NewArgs {
            title: Some("Test Title".to_string()),
            force: false,
        })
        .expect("run should succeed");

        let draft_path = DraftFile::path_for(&RepoInfo::from_git_only().unwrap());
        let content = fs::read_to_string(&draft_path).expect("read draft");

        assert_eq!(
            content.contains("ready-for-translation"),
            expect_ready_for_translation,
            "expected ready-for-translation={expect_ready_for_translation}, got:\n{content}"
        );
    }

    #[test]
    #[serial(pr_draft)]
    fn new_fails_when_file_exists_without_force() {
        let _env = setup_test_env("fails_without_force", Some(true));

        run(&NewArgs {
            title: Some("First Title".to_string()),
            force: false,
        })
        .expect("first run should succeed");

        let draft_path = DraftFile::path_for(&RepoInfo::from_git_only().unwrap());
        assert!(
            draft_path.exists(),
            "draft file should exist after first run"
        );

        let result = run(&NewArgs {
            title: Some("Second Title".to_string()),
            force: false,
        });
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
    #[serial(pr_draft)]
    fn new_overwrites_when_file_exists_with_force() {
        let _env = setup_test_env("overwrites_with_force", Some(true));

        run(&NewArgs {
            title: Some("First Title".to_string()),
            force: false,
        })
        .expect("first run should succeed");

        run(&NewArgs {
            title: Some("Second Title".to_string()),
            force: true,
        })
        .expect("second run with --force should succeed");

        let draft_path = DraftFile::path_for(&RepoInfo::from_git_only().unwrap());
        let content = fs::read_to_string(&draft_path).expect("read draft");
        assert!(
            content.contains("Second Title"),
            "content should be overwritten: {content}"
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

    struct EnvGuard {
        key: &'static str,
        original: Option<std::ffi::OsString>,
    }

    impl EnvGuard {
        fn set(key: &'static str, value: impl AsRef<std::ffi::OsStr>) -> Self {
            let original = std::env::var_os(key);
            unsafe {
                std::env::set_var(key, value);
            }
            Self { key, original }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            if let Some(ref original) = self.original {
                unsafe {
                    std::env::set_var(self.key, original);
                }
            } else {
                unsafe {
                    std::env::remove_var(self.key);
                }
            }
        }
    }
}
