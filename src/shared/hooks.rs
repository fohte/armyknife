use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::Command;

use crate::shared::dirs;

/// Returns the path to a hook script: `{config_dir}/armyknife/hooks/{hook_name}`
fn hook_path(hook_name: &str) -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join("armyknife").join("hooks").join(hook_name))
}

/// Executes a hook script if it exists and is executable.
///
/// Follows git-style hook conventions:
/// - If the hook file doesn't exist, silently returns Ok(())
/// - If the file exists but isn't executable, prints a warning and returns Ok(())
/// - If the hook exits with non-zero status, prints a warning and returns Ok(())
///   (hook failure should not block the main operation)
pub fn run_hook(hook_name: &str, env_vars: &[(&str, &str)]) -> anyhow::Result<()> {
    let path = match hook_path(hook_name) {
        Some(p) => p,
        None => return Ok(()),
    };

    if !path.exists() {
        return Ok(());
    }

    let metadata = std::fs::metadata(&path)?;
    let permissions = metadata.permissions();
    if permissions.mode() & 0o111 == 0 {
        eprintln!(
            "warning: hook '{}' exists but is not executable, skipping",
            path.display()
        );
        return Ok(());
    }

    let mut cmd = Command::new(&path);
    for (key, value) in env_vars {
        cmd.env(key, value);
    }

    let status = cmd.status()?;
    if !status.success() {
        let code = status
            .code()
            .map(|c| c.to_string())
            .unwrap_or_else(|| "signal".to_string());
        eprintln!(
            "warning: hook '{}' exited with status {}",
            path.display(),
            code
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shared::env_var::EnvVars;
    use rstest::rstest;
    use std::fs;
    use tempfile::TempDir;

    fn setup_hook(dir: &TempDir, hook_name: &str, script: &str, executable: bool) -> PathBuf {
        let hooks_dir = dir.path().join("armyknife").join("hooks");
        fs::create_dir_all(&hooks_dir).unwrap();

        let hook_file = hooks_dir.join(hook_name);
        fs::write(&hook_file, script).unwrap();

        if executable {
            let mut perms = fs::metadata(&hook_file).unwrap().permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&hook_file, perms).unwrap();
        }

        hook_file
    }

    #[rstest]
    fn run_hook_returns_ok_when_hook_does_not_exist() {
        temp_env::with_vars([("XDG_CONFIG_HOME", Some("/nonexistent/path"))], || {
            let result = run_hook("post-worktree-create", &[]);
            assert!(result.is_ok());
        });
    }

    #[rstest]
    fn run_hook_skips_non_executable_hook() {
        let dir = TempDir::new().unwrap();
        setup_hook(&dir, "post-worktree-create", "#!/bin/sh\necho hello", false);

        temp_env::with_vars(
            [("XDG_CONFIG_HOME", Some(dir.path().to_str().unwrap()))],
            || {
                let result = run_hook("post-worktree-create", &[]);
                assert!(result.is_ok());
            },
        );
    }

    #[rstest]
    fn run_hook_executes_hook_and_passes_env_vars() {
        let dir = TempDir::new().unwrap();
        let output_file = dir.path().join("output.txt");

        let wt_name = EnvVars::worktree_path_name();
        let br_name = EnvVars::branch_name_name();
        let script = format!(
            "#!/bin/sh\necho \"${wt_name}:${br_name}\" > {}",
            output_file.display()
        );
        setup_hook(&dir, "post-worktree-create", &script, true);

        temp_env::with_vars(
            [("XDG_CONFIG_HOME", Some(dir.path().to_str().unwrap()))],
            || {
                let result = run_hook(
                    "post-worktree-create",
                    &[
                        (EnvVars::worktree_path_name(), "/tmp/test-worktree"),
                        (EnvVars::branch_name_name(), "feature/test"),
                    ],
                );
                assert!(result.is_ok());
            },
        );

        let output = fs::read_to_string(&output_file).unwrap();
        assert_eq!(output.trim(), "/tmp/test-worktree:feature/test");
    }

    #[rstest]
    fn run_hook_warns_on_nonzero_exit() {
        let dir = TempDir::new().unwrap();
        setup_hook(&dir, "post-worktree-create", "#!/bin/sh\nexit 1", true);

        temp_env::with_vars(
            [("XDG_CONFIG_HOME", Some(dir.path().to_str().unwrap()))],
            || {
                let result = run_hook("post-worktree-create", &[]);
                // Should still return Ok even when hook exits non-zero
                assert!(result.is_ok());
            },
        );
    }

    #[rstest]
    fn run_hook_returns_ok_when_config_dir_unavailable() {
        temp_env::with_vars([("XDG_CONFIG_HOME", Some("")), ("HOME", Some(""))], || {
            let result = run_hook("post-worktree-create", &[]);
            assert!(result.is_ok());
        });
    }
}
