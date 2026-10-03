use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use crate::infra::process;
use crate::shared::{command, dirs, env_var::EnvVars};

/// Returns the path to a hook script: `{config_dir}/armyknife/hooks/{hook_name}`
fn hook_path(hook_name: &str) -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join("armyknife").join("hooks").join(hook_name))
}

/// Resolves the hook script path only if it exists on disk.
fn existing_hook_path(hook_name: &str) -> Option<PathBuf> {
    let path = hook_path(hook_name)?;
    path.exists().then_some(path)
}

/// Returns whether a hook script is configured for the given hook name,
/// regardless of whether it is executable. Lets callers distinguish "no hook
/// configured" from "hook ran", since `run_hook` returns `Ok(())` in both
/// cases.
pub fn hook_exists(hook_name: &str) -> bool {
    existing_hook_path(hook_name).is_some()
}

/// Executes a hook script if one is configured.
///
/// Follows git-style hook conventions, but treats hook failure as a hard error:
/// - If the hook file doesn't exist, silently returns Ok(()) (hook is unconfigured)
/// - If the file exists but isn't executable, returns Err (likely a misconfiguration)
/// - If the hook exits with non-zero status, returns Err so callers can abort the
///   operation (e.g., a `pre-pr-submit` hook can block submission)
pub fn run_hook(hook_name: &str, env_vars: &[(&str, &str)]) -> anyhow::Result<()> {
    let Some(path) = existing_hook_path(hook_name) else {
        return Ok(());
    };

    let metadata = std::fs::metadata(&path)?;
    let permissions = metadata.permissions();
    if permissions.mode() & 0o111 == 0 {
        anyhow::bail!("hook '{}' exists but is not executable", path.display());
    }

    let mut cmd = command::new(&path);
    for (key, value) in env_vars {
        cmd.env(key, value);
    }

    let status = cmd.status()?;
    if !status.success() {
        let code = status
            .code()
            .map(|c| c.to_string())
            .unwrap_or_else(|| "signal".to_string());
        anyhow::bail!("hook '{}' exited with status {}", path.display(), code);
    }

    Ok(())
}

/// Starts a hook in a detached session and returns without waiting for it.
///
/// A missing hook is skipped. Spawn failures are returned so post hooks can
/// report them without making cleanup fail.
pub fn spawn_hook_detached(
    hook_name: &str,
    cwd: &Path,
    env_vars: &[(&str, &str)],
) -> anyhow::Result<bool> {
    spawn_hook_detached_with(
        hook_name,
        cwd,
        env_vars,
        existing_hook_path,
        |path, cwd, env_vars| {
            process::spawn_detached(path, std::iter::empty::<&str>(), Some(cwd), env_vars)
                .map_err(anyhow::Error::from)
        },
    )
}

fn spawn_hook_detached_with(
    hook_name: &str,
    cwd: &Path,
    env_vars: &[(&str, &str)],
    resolve_hook: impl FnOnce(&str) -> Option<PathBuf>,
    spawn: impl FnOnce(&Path, &Path, &[(&str, &str)]) -> anyhow::Result<()>,
) -> anyhow::Result<bool> {
    let Some(path) = resolve_hook(hook_name) else {
        return Ok(false);
    };

    let metadata = std::fs::metadata(&path)?;
    if metadata.permissions().mode() & 0o111 == 0 {
        anyhow::bail!("hook '{}' exists but is not executable", path.display());
    }

    spawn(&path, cwd, env_vars)?;
    Ok(true)
}

/// Starts `post-worktree-delete` after a worktree is removed.
///
/// Hook startup is best-effort because the deletion has already succeeded.
pub fn spawn_post_worktree_delete_hook(
    repo_root: &Path,
    worktree_path: &Path,
    branch_name: Option<&str>,
    merged: bool,
) {
    spawn_post_worktree_delete_hook_with(
        repo_root,
        worktree_path,
        branch_name,
        merged,
        spawn_hook_detached,
    );
}

fn spawn_post_worktree_delete_hook_with(
    repo_root: &Path,
    worktree_path: &Path,
    branch_name: Option<&str>,
    merged: bool,
    spawn_hook: impl FnOnce(&str, &Path, &[(&str, &str)]) -> anyhow::Result<bool>,
) {
    let repo_root_value = repo_root.to_string_lossy();
    let worktree_path = worktree_path.to_string_lossy();
    let branch_name = branch_name.unwrap_or_default();
    let merged = if merged { "true" } else { "false" };
    let env_vars = [
        (EnvVars::worktree_path_name(), worktree_path.as_ref()),
        (EnvVars::branch_name_name(), branch_name),
        (EnvVars::repo_root_name(), repo_root_value.as_ref()),
        (EnvVars::merged_name(), merged),
    ];

    if let Err(error) = spawn_hook("post-worktree-delete", repo_root, &env_vars) {
        tracing::warn!(
            target: "armyknife::shared::hooks",
            event = "hook.detached_spawn_failed",
            hook = "post-worktree-delete",
            worktree = %worktree_path,
            error = %error,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shared::env_var::EnvVars;
    use rstest::rstest;
    use std::cell::RefCell;
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
    fn run_hook_errors_on_non_executable_hook() {
        let dir = TempDir::new().unwrap();
        let hook_file = setup_hook(&dir, "post-worktree-create", "#!/bin/sh\necho hello", false);

        temp_env::with_vars(
            [("XDG_CONFIG_HOME", Some(dir.path().to_str().unwrap()))],
            || {
                let err = run_hook("post-worktree-create", &[])
                    .expect_err("non-executable hook must error");
                assert_eq!(
                    err.to_string(),
                    format!(
                        "hook '{}' exists but is not executable",
                        hook_file.display()
                    )
                );
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
    fn run_hook_errors_on_nonzero_exit() {
        let dir = TempDir::new().unwrap();
        let hook_file = setup_hook(&dir, "post-worktree-create", "#!/bin/sh\nexit 1", true);

        temp_env::with_vars(
            [("XDG_CONFIG_HOME", Some(dir.path().to_str().unwrap()))],
            || {
                let err = run_hook("post-worktree-create", &[])
                    .expect_err("non-zero exit must propagate");
                assert_eq!(
                    err.to_string(),
                    format!("hook '{}' exited with status 1", hook_file.display())
                );
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

    #[rstest]
    fn hook_exists_returns_false_when_config_dir_unavailable() {
        temp_env::with_vars([("XDG_CONFIG_HOME", Some("")), ("HOME", Some(""))], || {
            assert!(!hook_exists("post-worktree-create"));
        });
    }

    #[test]
    fn spawn_hook_detached_passes_the_hook_path_cwd_and_environment() {
        let dir = TempDir::new().unwrap();
        let hook_file = setup_hook(&dir, "post-worktree-delete", "#!/bin/sh\n", true);
        let cwd = dir.path().join("repository");
        let env_vars = [("EXAMPLE_KEY", "example-value")];
        let spawned = RefCell::new(None);

        let result = spawn_hook_detached_with(
            "post-worktree-delete",
            &cwd,
            &env_vars,
            |_| Some(hook_file.clone()),
            |path, cwd, env_vars| {
                *spawned.borrow_mut() = Some((
                    path.to_path_buf(),
                    cwd.to_path_buf(),
                    env_vars
                        .iter()
                        .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
                        .collect::<Vec<_>>(),
                ));
                Ok(())
            },
        )
        .unwrap();

        assert_eq!(
            (result, spawned.into_inner()),
            (
                true,
                Some((
                    hook_file,
                    cwd,
                    vec![("EXAMPLE_KEY".to_string(), "example-value".to_string())],
                )),
            )
        );
    }

    #[rstest]
    #[case::merged(true, Some("example/branch"), "true")]
    #[case::unmerged(false, Some("example/branch"), "false")]
    #[case::branch_unresolved(false, None, "false")]
    fn post_worktree_delete_hook_passes_its_environment(
        #[case] is_merged: bool,
        #[case] branch: Option<&str>,
        #[case] expected_merged: &str,
    ) {
        let repo_root = PathBuf::from("/tmp/example-repository");
        let worktree_path = PathBuf::from("/tmp/example-repository/.worktrees/example");
        let mut spawned = None;

        spawn_post_worktree_delete_hook_with(
            &repo_root,
            &worktree_path,
            branch,
            is_merged,
            |hook_name, cwd, env_vars| {
                spawned = Some((
                    hook_name.to_string(),
                    cwd.to_path_buf(),
                    env_vars
                        .iter()
                        .map(|(name, value)| ((*name).to_string(), (*value).to_string()))
                        .collect::<Vec<_>>(),
                ));
                Ok(true)
            },
        );

        assert_eq!(
            spawned,
            Some((
                "post-worktree-delete".to_string(),
                repo_root.clone(),
                vec![
                    (
                        EnvVars::worktree_path_name().to_string(),
                        worktree_path.to_string_lossy().into_owned(),
                    ),
                    (
                        EnvVars::branch_name_name().to_string(),
                        branch.unwrap_or_default().to_string(),
                    ),
                    (
                        EnvVars::repo_root_name().to_string(),
                        repo_root.to_string_lossy().into_owned(),
                    ),
                    (
                        EnvVars::merged_name().to_string(),
                        expected_merged.to_string(),
                    ),
                ],
            )),
        );
    }
}
