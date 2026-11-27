use anyhow::{Context, Result, bail};
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::commands::agent::error::CcError;
use crate::commands::name_branch::{detect_backend, generate_branch_name};
use crate::shared::cache;
use crate::shared::command;

/// Get the cache path template for prompt recovery.
fn get_prompt_cache_path(repo_root: &str) -> Option<PathBuf> {
    let repo_name = Path::new(repo_root).file_name()?.to_str()?;
    cache::wm_prompt(repo_name)
}

/// Save prompt to cache directory for recovery.
pub(super) fn save_prompt_cache(repo_root: &str, prompt: &str) -> Result<PathBuf> {
    let path = get_prompt_cache_path(repo_root).context("Failed to determine cache directory")?;
    save_prompt_cache_to(&path, prompt)
}

/// Save prompt into a unique file under the cache directory.
fn save_prompt_cache_to(path: &Path, prompt: &str) -> Result<PathBuf> {
    let parent = path
        .parent()
        .context("Failed to determine cache directory")?;
    std::fs::create_dir_all(parent).context("Failed to create cache directory")?;

    let mut file = tempfile::Builder::new()
        .prefix("prompt-")
        .suffix(".md")
        .tempfile_in(parent)
        .context("Failed to create prompt cache file")?;
    file.write_all(prompt.as_bytes())
        .context("Failed to save prompt")?;
    let (_, saved_path) = file.keep().context("Failed to persist prompt cache")?;

    Ok(saved_path)
}

/// Delete the saved prompt cache after successful completion.
pub(super) fn delete_prompt_cache(path: &Path) {
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
/// `worktree` is the value of `--worktree` (already known to be present).
pub(super) fn resolve_args(worktree: Option<&str>, prompt: Option<&str>) -> Result<ResolvedArgs> {
    resolve_args_with_deps(
        worktree,
        prompt,
        || detect_backend(),
        open_editor_for_prompt,
    )
}

/// Internal implementation that accepts dependencies for testability.
fn resolve_args_with_deps<F, E>(
    worktree: Option<&str>,
    prompt: Option<&str>,
    backend_factory: F,
    editor_fn: E,
) -> Result<ResolvedArgs>
where
    F: FnOnce() -> Box<dyn crate::commands::name_branch::Backend>,
    E: FnOnce() -> Result<Option<String>>,
{
    match (worktree, prompt) {
        (Some(name), prompt) => Ok(ResolvedArgs {
            branch_name: name.to_string(),
            prompt: prompt.map(String::from),
        }),
        (None, Some(prompt)) => {
            let backend = backend_factory();
            let generated = generate_branch_name(prompt, backend.as_ref())?;
            Ok(ResolvedArgs {
                branch_name: generated,
                prompt: Some(prompt.to_string()),
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
    use clap::Parser;
    use rstest::rstest;
    use std::cell::{Cell, RefCell};
    use tempfile::TempDir;

    #[derive(Parser)]
    struct TestNewCli {
        #[command(flatten)]
        args: super::super::NewArgs,
    }

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
        let result = resolve_args_with_deps(
            name,
            prompt,
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
        let result = resolve_args_with_deps(
            None,
            None,
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

    #[test]
    fn worktree_failure_caches_the_resolved_editor_prompt() {
        let args = TestNewCli::try_parse_from(["a", "--worktree"])
            .unwrap()
            .args;
        let saved_prompt = RefCell::new(None);
        let launched = RefCell::new(None);
        let deleted = Cell::new(false);

        let result = super::super::run_worktree_mode_with_deps(
            &args,
            args.worktree.as_ref().and_then(Option::as_deref),
            "/repo",
            |worktree, prompt| {
                resolve_args_with_deps(
                    worktree,
                    prompt,
                    || mock_backend("editor-branch"),
                    || Ok(Some("editor draft".to_string())),
                )
            },
            |_repo, prompt| {
                saved_prompt.replace(Some(prompt.to_string()));
                Ok(PathBuf::from("prompt-cache.md"))
            },
            |_path| deleted.set(true),
            |_, branch_name, prompt, _| {
                launched.replace(Some((branch_name.to_string(), prompt.map(str::to_string))));
                anyhow::bail!("worktree creation failed")
            },
        )
        .map_err(|error| error.to_string());

        assert_eq!(
            (
                result,
                saved_prompt.into_inner(),
                launched.into_inner(),
                deleted.get(),
            ),
            (
                Err("worktree creation failed".to_string()),
                Some("editor draft".to_string()),
                Some((
                    "editor-branch".to_string(),
                    Some("editor draft".to_string()),
                )),
                false,
            ),
        );
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
        let cache_path_template = temp_dir
            .path()
            .join("wm")
            .join("test-repo")
            .join("prompt.md");

        let prompt = "test prompt content";
        let cache_path = save_prompt_cache_to(&cache_path_template, prompt).unwrap();

        // File should exist with correct content
        assert!(cache_path.exists());
        assert_eq!(std::fs::read_to_string(&cache_path).unwrap(), prompt);

        // Delete should remove the file
        delete_prompt_cache(&cache_path);
        assert!(!cache_path.exists());
    }

    #[rstest]
    fn save_prompt_cache_creates_parent_directories() {
        let temp_dir = TempDir::new().unwrap();
        let cache_path_template = temp_dir
            .path()
            .join("wm")
            .join("nested")
            .join("path")
            .join("repo")
            .join("prompt.md");

        let cache_path = save_prompt_cache_to(&cache_path_template, "test").unwrap();
        assert!(cache_path.exists());
    }

    #[rstest]
    fn save_prompt_cache_uses_a_unique_file_per_invocation() {
        let temp_dir = TempDir::new().unwrap();
        let cache_path_template = temp_dir.path().join("repo").join("prompt.md");
        let cache_dir = cache_path_template.parent().unwrap().to_path_buf();

        let first = save_prompt_cache_to(&cache_path_template, "first prompt").unwrap();
        let second = save_prompt_cache_to(&cache_path_template, "second prompt").unwrap();

        assert_eq!(
            (
                first.parent().unwrap().to_path_buf(),
                second.parent().unwrap().to_path_buf(),
                first == second,
                std::fs::read_to_string(&first).unwrap(),
                std::fs::read_to_string(&second).unwrap(),
            ),
            (
                cache_dir.clone(),
                cache_dir,
                false,
                "first prompt".to_string(),
                "second prompt".to_string(),
            ),
        );
    }

    #[rstest]
    fn delete_prompt_cache_does_not_fail_for_nonexistent() {
        let temp_dir = TempDir::new().unwrap();
        let cache_path = temp_dir.path().join("nonexistent-repo").join("prompt.md");

        // Should not panic or error
        delete_prompt_cache(&cache_path);
    }
}
