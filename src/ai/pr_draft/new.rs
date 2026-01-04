use clap::Args;
use std::fs;

use super::common::{DraftFile, RepoInfo, generate_frontmatter, read_stdin_if_available};

#[derive(Args, Clone, PartialEq, Eq)]
pub struct NewArgs {
    /// PR title
    #[arg(long)]
    pub title: Option<String>,
}

pub fn run(args: &NewArgs) -> std::result::Result<(), Box<dyn std::error::Error>> {
    // Use from_git_only() to avoid network access - is_private check happens at submit time
    let repo_info = RepoInfo::from_git_only()?;
    let draft_path = DraftFile::path_for(&repo_info);

    // Create parent directories
    if let Some(parent) = draft_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let title = args.title.as_deref().unwrap_or("");
    // Always include both steps (ready-for-translation + submit) - the actual
    // is_private validation happens at submit time, not during draft creation
    let frontmatter = generate_frontmatter(title, false);

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

    #[test]
    #[serial]
    fn new_should_not_require_network_to_generate_draft() {
        let temp_cwd = tempdir().expect("tempdir");
        let _cwd_guard = WorkingDirGuard::change(temp_cwd.path());

        let stub_dir = tempdir().expect("stub dir");
        let git_stub = stub_dir.path().join("git");
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
        let gh_stub = stub_dir.path().join("gh");
        fs::write(
            &gh_stub,
            indoc! {r#"
            #!/bin/sh
            echo "offline" >&2
            exit 2
        "#},
        )
        .expect("write gh stub");
        for stub in [&git_stub, &gh_stub] {
            let mut perms = fs::metadata(stub).expect("metadata").permissions();
            perms.set_mode(0o755);
            fs::set_permissions(stub, perms).expect("chmod");
        }
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
