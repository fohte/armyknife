use clap::Args;
use std::fs;

use super::common::{
    DraftFile, RepoInfo, check_is_private, generate_frontmatter, read_stdin_if_available,
};

#[derive(Args, Clone, PartialEq, Eq)]
pub struct NewArgs {
    /// PR title
    #[arg(long)]
    pub title: Option<String>,
}

pub fn run(args: &NewArgs) -> std::result::Result<(), Box<dyn std::error::Error>> {
    let repo_info = RepoInfo::from_git_only()?;
    let draft_path = DraftFile::path_for(&repo_info);

    // Create parent directories
    if let Some(parent) = draft_path.parent() {
        fs::create_dir_all(parent)?;
    }

    // Check if the repo is private (defaults to true if network is unavailable)
    let is_private = check_is_private(&repo_info.owner, &repo_info.repo).unwrap_or(true);

    let title = args.title.as_deref().unwrap_or("");
    let frontmatter = generate_frontmatter(title, is_private);

    let body = read_stdin_if_available().unwrap_or_default();
    let content = format!("{frontmatter}{body}");

    fs::write(&draft_path, content)?;

    println!("{}", draft_path.display());

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

    fn create_git_stub(stub_dir: &std::path::Path) -> std::path::PathBuf {
        let git_stub = stub_dir.join("git");
        fs::write(
            &git_stub,
            indoc! {r#"
            #!/bin/sh
            if [ "$1" = "rev-parse" ]; then
              echo "feature/test"
              exit 0
            fi
            if [ "$1" = "remote" ] && [ "$2" = "get-url" ]; then
              echo "https://github.com/owner/repo.git"
              exit 0
            fi
            echo "unexpected git command: $@" >&2
            exit 1
        "#},
        )
        .expect("write git stub");
        let mut perms = fs::metadata(&git_stub).expect("metadata").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&git_stub, perms).expect("chmod");
        git_stub
    }

    fn create_gh_stub(stub_dir: &std::path::Path, is_private: Option<bool>) -> std::path::PathBuf {
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
        gh_stub
    }

    #[test]
    #[serial]
    fn new_should_not_require_network_to_generate_draft() {
        let temp_cwd = tempdir().expect("tempdir");
        let _cwd_guard = WorkingDirGuard::change(temp_cwd.path());

        let stub_dir = tempdir().expect("stub dir");
        create_git_stub(stub_dir.path());
        create_gh_stub(stub_dir.path(), None); // offline
        let _path_guard = PathGuard::prepend(stub_dir.path());

        let args = NewArgs {
            title: Some("Test Title".to_string()),
        };

        let result = run(&args);

        assert!(
            result.is_ok(),
            "new should not fail just because gh (network) is unavailable"
        );
    }

    #[test]
    #[serial]
    fn new_should_exclude_ready_for_translation_for_private_repo() {
        let temp_cwd = tempdir().expect("tempdir");
        let _cwd_guard = WorkingDirGuard::change(temp_cwd.path());

        let stub_dir = tempdir().expect("stub dir");
        create_git_stub(stub_dir.path());
        create_gh_stub(stub_dir.path(), Some(true)); // private
        let _path_guard = PathGuard::prepend(stub_dir.path());

        let args = NewArgs {
            title: Some("Test Title".to_string()),
        };

        run(&args).expect("run should succeed");

        let draft_path = DraftFile::path_for(&RepoInfo::from_git_only().unwrap());
        let content = fs::read_to_string(&draft_path).expect("read draft");

        assert!(
            !content.contains("ready-for-translation"),
            "private repo should not include ready-for-translation step, but got:\n{content}"
        );
    }

    #[test]
    #[serial]
    fn new_should_include_ready_for_translation_for_public_repo() {
        let temp_cwd = tempdir().expect("tempdir");
        let _cwd_guard = WorkingDirGuard::change(temp_cwd.path());

        let stub_dir = tempdir().expect("stub dir");
        create_git_stub(stub_dir.path());
        create_gh_stub(stub_dir.path(), Some(false)); // public
        let _path_guard = PathGuard::prepend(stub_dir.path());

        let args = NewArgs {
            title: Some("Test Title".to_string()),
        };

        run(&args).expect("run should succeed");

        let draft_path = DraftFile::path_for(&RepoInfo::from_git_only().unwrap());
        let content = fs::read_to_string(&draft_path).expect("read draft");

        assert!(
            content.contains("ready-for-translation"),
            "public repo should include ready-for-translation step, but got:\n{content}"
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
