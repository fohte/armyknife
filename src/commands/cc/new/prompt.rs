use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};

use super::NewArgs;
use crate::commands::cc::error::CcError;
use crate::commands::name_branch::{detect_backend, generate_branch_name};
use crate::shared::cache;
use crate::shared::command;

/// Get the cache path for prompt recovery.
fn get_prompt_cache_path(repo_root: &str) -> Option<PathBuf> {
    let repo_name = Path::new(repo_root).file_name()?.to_str()?;
    cache::wm_prompt(repo_name)
}

/// Save prompt to cache directory for recovery.
pub(super) fn save_prompt_cache(repo_root: &str, prompt: &str) -> Result<PathBuf> {
    let path = get_prompt_cache_path(repo_root).context("Failed to determine cache directory")?;
    save_prompt_cache_to(&path, prompt)?;
    Ok(path)
}

/// Internal implementation for saving prompt to a specific path.
/// Allows testing with temporary directories.
fn save_prompt_cache_to(path: &Path, prompt: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).context("Failed to create cache directory")?;
    }

    std::fs::write(path, prompt).context("Failed to save prompt")?;

    Ok(())
}

/// Delete the saved prompt cache after successful completion.
pub(super) fn delete_prompt_cache(repo_root: &str) {
    if let Some(path) = get_prompt_cache_path(repo_root) {
        delete_prompt_cache_at(&path);
    }
}

/// Internal implementation for deleting prompt cache at a specific path.
fn delete_prompt_cache_at(path: &Path) {
    let _ = std::fs::remove_file(path);
}

/// Open $EDITOR to let user input a prompt.
/// Returns the prompt text, or None if the user didn't provide any input.
fn open_editor_for_prompt() -> Result<Option<String>> {
    let editor = std::env::var("EDITOR").unwrap_or_else(|_| "vi".to_string());

    // Create an empty temp file for the prompt
    let temp_file = tempfile::Builder::new()
        .prefix("wm-prompt-")
        .suffix(".md")
        .tempfile()
        .context("Failed to create temp file")?;

    let temp_path = temp_file.path().to_path_buf();

    // Launch editor
    let status = command::new(&editor)
        .arg(&temp_path)
        .status()
        .with_context(|| format!("Failed to launch editor '{editor}'"))?;

    if !status.success() {
        bail!("Editor exited with status: {status}");
    }

    // Read the content
    let content = std::fs::read_to_string(&temp_path).context("Failed to read temp file")?;

    let prompt = content.trim().to_string();

    if prompt.is_empty() {
        Ok(None)
    } else {
        Ok(Some(prompt))
    }
}

/// Resolved branch name and prompt information
#[derive(Debug)]
pub(super) struct ResolvedArgs {
    pub(super) branch_name: String,
    pub(super) prompt: Option<String>,
}

/// Resolve branch name: use provided name or generate from prompt.
/// If no name and no prompt provided, opens editor to get prompt.
pub(super) fn resolve_args(args: &NewArgs) -> Result<ResolvedArgs> {
    resolve_args_with_deps(args, || detect_backend(), open_editor_for_prompt)
}

/// Internal implementation that accepts dependencies for testability.
fn resolve_args_with_deps<F, E>(
    args: &NewArgs,
    backend_factory: F,
    editor_fn: E,
) -> Result<ResolvedArgs>
where
    F: FnOnce() -> Box<dyn crate::commands::name_branch::Backend>,
    E: FnOnce() -> Result<Option<String>>,
{
    match (&args.worktree, &args.prompt) {
        (Some(name), prompt) => Ok(ResolvedArgs {
            branch_name: name.clone(),
            prompt: prompt.clone(),
        }),
        (None, Some(prompt)) => {
            let backend = backend_factory();
            let generated = generate_branch_name(prompt, backend.as_ref())?;
            Ok(ResolvedArgs {
                branch_name: generated,
                prompt: Some(prompt.clone()),
            })
        }
        (None, None) => {
            // Open editor to get prompt
            let prompt = editor_fn()?.ok_or(CcError::Cancelled)?;
            let backend = backend_factory();
            let generated = generate_branch_name(&prompt, backend.as_ref())?;
            Ok(ResolvedArgs {
                branch_name: generated,
                prompt: Some(prompt),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::name_branch::{Backend, Result as NameBranchResult};
    use rstest::rstest;
    use tempfile::TempDir;

    /// Mock backend for testing
    struct MockBackend {
        response: String,
    }

    impl Backend for MockBackend {
        fn generate(&self, _prompt: &str) -> NameBranchResult<String> {
            Ok(self.response.clone())
        }
    }

    fn mock_backend(response: &str) -> Box<dyn Backend> {
        Box::new(MockBackend {
            response: response.to_string(),
        })
    }

    #[rstest]
    #[case::explicit_name(Some("my-branch"), None, "my-branch", None)]
    #[case::name_takes_priority_over_prompt(
        Some("my-branch"),
        Some("some task"),
        "my-branch",
        Some("some task")
    )]
    #[case::generate_from_prompt(
        None,
        Some("fix login bug"),
        "fix-login-bug",
        Some("fix login bug")
    )]
    fn resolve_args_returns_expected(
        #[case] name: Option<&str>,
        #[case] prompt: Option<&str>,
        #[case] expected_branch: &str,
        #[case] expected_prompt: Option<&str>,
    ) {
        let args = NewArgs {
            worktree: name.map(String::from),
            from: None,
            force: false,
            prompt: prompt.map(String::from),
            agent: false,
            label: None,
            parent_session_id: None,
            repo: None,
            skip_hooks: false,
        };
        let result = resolve_args_with_deps(
            &args,
            || mock_backend("fix-login-bug"),
            || panic!("editor should not be called"),
        )
        .unwrap();

        assert_eq!(result.branch_name, expected_branch);
        assert_eq!(result.prompt.as_deref(), expected_prompt);
    }

    #[rstest]
    #[case::editor_returns_prompt(
        Some("prompt from editor"),
        Ok(("editor-branch", Some("prompt from editor")))
    )]
    #[case::editor_returns_empty(None, Err(true))]
    fn resolve_args_with_editor(
        #[case] editor_input: Option<&str>,
        #[case] expected: std::result::Result<(&str, Option<&str>), bool>,
    ) {
        let args = NewArgs {
            worktree: None,
            from: None,
            force: false,
            prompt: None,
            agent: false,
            label: None,
            parent_session_id: None,
            repo: None,
            skip_hooks: false,
        };
        let result = resolve_args_with_deps(
            &args,
            || mock_backend("editor-branch"),
            || Ok(editor_input.map(String::from)),
        );

        match expected {
            Ok((branch, prompt)) => {
                let resolved = result.unwrap();
                assert_eq!(resolved.branch_name, branch);
                assert_eq!(resolved.prompt.as_deref(), prompt);
            }
            Err(_) => {
                let err = result.unwrap_err();
                assert!(
                    err.downcast_ref::<CcError>()
                        .is_some_and(|e| matches!(e, CcError::Cancelled))
                );
            }
        }
    }

    #[rstest]
    #[case::extracts_repo_name("/home/user/projects/my-repo", Some("my-repo"))]
    #[case::root_returns_none("/", None)]
    fn get_prompt_cache_path_behavior(
        #[case] repo_root: &str,
        #[case] expected_repo: Option<&str>,
    ) {
        let path = get_prompt_cache_path(repo_root);
        match expected_repo {
            Some(repo) => {
                let p = path.unwrap();
                assert!(p.ends_with("prompt.md"));
                assert!(p.parent().unwrap().ends_with(repo));
            }
            None => assert!(path.is_none()),
        }
    }

    #[rstest]
    fn save_and_delete_prompt_cache() {
        let temp_dir = TempDir::new().unwrap();
        let cache_path = temp_dir
            .path()
            .join("wm")
            .join("test-repo")
            .join("prompt.md");

        let prompt = "test prompt content";
        save_prompt_cache_to(&cache_path, prompt).unwrap();

        // File should exist with correct content
        assert!(cache_path.exists());
        assert_eq!(std::fs::read_to_string(&cache_path).unwrap(), prompt);

        // Delete should remove the file
        delete_prompt_cache_at(&cache_path);
        assert!(!cache_path.exists());
    }

    #[rstest]
    fn save_prompt_cache_creates_parent_directories() {
        let temp_dir = TempDir::new().unwrap();
        let cache_path = temp_dir
            .path()
            .join("wm")
            .join("nested")
            .join("path")
            .join("repo")
            .join("prompt.md");

        save_prompt_cache_to(&cache_path, "test").unwrap();
        assert!(cache_path.exists());
    }

    #[rstest]
    fn delete_prompt_cache_does_not_fail_for_nonexistent() {
        let temp_dir = TempDir::new().unwrap();
        let repo_root = temp_dir.path().join("nonexistent-repo");
        std::fs::create_dir_all(&repo_root).unwrap();

        // Should not panic or error
        delete_prompt_cache(repo_root.to_str().unwrap());
    }
}
