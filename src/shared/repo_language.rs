use std::future::Future;
use std::path::{Path, PathBuf};

use crate::infra::git;
use serde_json::Value;

pub(crate) fn copier_answers_path_for_current_repo(
    expected_repo_id: Option<&str>,
) -> Option<PathBuf> {
    let repo = git::open_repo().ok()?;
    if let Some(expected_repo_id) = expected_repo_id {
        let (owner, name) = git::github_owner_and_repo(&repo).ok()?;
        if !repo_ids_match(&format!("{owner}/{name}"), expected_repo_id) {
            return None;
        }
    }
    Some(repo.workdir().join(".copier-answers.yml"))
}

pub(crate) fn copier_answers_path_for_repo_at(
    cwd: &Path,
    expected_repo_id: &str,
) -> Option<PathBuf> {
    copier_answers_path_for_repo_at_with(cwd, expected_repo_id, |cwd| {
        let repo = git::open_repo_at(cwd).ok()?;
        let (owner, name) = git::github_owner_and_repo(&repo).ok()?;
        Some((repo.workdir().to_path_buf(), format!("{owner}/{name}")))
    })
}

fn copier_answers_path_for_repo_at_with(
    cwd: &Path,
    expected_repo_id: &str,
    resolve_repo: impl FnOnce(&Path) -> Option<(PathBuf, String)>,
) -> Option<PathBuf> {
    let (workdir, actual_repo_id) = resolve_repo(cwd)?;
    if !repo_ids_match(&actual_repo_id, expected_repo_id) {
        return None;
    }
    Some(workdir.join(".copier-answers.yml"))
}

fn repo_ids_match(actual: &str, expected: &str) -> bool {
    actual.eq_ignore_ascii_case(expected)
}

pub(crate) async fn resolve_repo_language<F, Fut>(
    configured: Option<Value>,
    copier_answers_path: Option<&Path>,
    fetch_visibility: F,
) -> anyhow::Result<Option<Value>>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = anyhow::Result<Option<bool>>>,
{
    if let Some(value) = configured {
        return Ok(Some(value));
    }
    if let Some(language) = copier_answers_path.and_then(read_copier_repo_language) {
        return Ok(Some(Value::String(language)));
    }
    Ok(fetch_visibility()
        .await?
        .map(|is_private| Value::String(if is_private { "ja" } else { "en" }.to_owned())))
}

fn read_copier_repo_language(answers_path: &Path) -> Option<String> {
    let content = std::fs::read_to_string(answers_path).ok()?;
    let answers: serde_yaml::Value = serde_yaml::from_str(&content).ok()?;
    answers
        .get("repo_language")?
        .as_str()
        .filter(|value| !value.trim().is_empty() && !value.chars().any(char::is_control))
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::{copier_answers_path_for_repo_at_with, repo_ids_match, resolve_repo_language};
    use indoc::indoc;
    use rstest::rstest;
    use serde_json::{Value, json};
    use std::fs;

    #[rstest]
    #[case::matching_repository("sample-owner/sample-repository", true)]
    #[case::different_repository("sample-owner/another-repository", false)]
    #[tokio::test]
    async fn session_copier_answers_path_uses_the_target_worktree_and_repository_identity(
        #[case] actual_repo_id: &str,
        #[case] expected_match: bool,
    ) {
        let directory = tempfile::tempdir().expect("fixture directory should exist");
        let session_cwd = directory.path().join("session-cwd");
        let worktree = directory.path().join("reviewed-worktree");
        fs::create_dir_all(&worktree).expect("reviewed worktree should be created");
        fs::write(worktree.join(".copier-answers.yml"), "repo_language: ja\n")
            .expect("reviewed worktree Copier answers should be written");
        let mut resolved_cwd = None;

        let copier_answers_path = copier_answers_path_for_repo_at_with(
            &session_cwd,
            "sample-owner/sample-repository",
            |cwd| {
                resolved_cwd = Some(cwd.to_path_buf());
                Some((worktree.clone(), actual_repo_id.to_owned()))
            },
        );
        let language = resolve_repo_language(None, copier_answers_path.as_deref(), || async {
            Ok(Some(false))
        })
        .await
        .expect("repo language should resolve");

        assert_eq!(
            (resolved_cwd, copier_answers_path, language),
            (
                Some(session_cwd),
                expected_match.then(|| worktree.join(".copier-answers.yml")),
                Some(json!(if expected_match { "ja" } else { "en" })),
            ),
        );
    }

    #[rstest]
    #[case::same_id(
        "example-owner/sample-repository",
        "example-owner/sample-repository",
        true
    )]
    #[case::case_variant(
        "example-owner/sample-repository",
        "Example-Owner/Sample-Repository",
        true
    )]
    #[case::different_repository(
        "example-owner/sample-repository",
        "example-owner/other-repository",
        false
    )]
    fn repo_id_matching_ignores_case_only(
        #[case] actual: &str,
        #[case] expected: &str,
        #[case] matches: bool,
    ) {
        assert_eq!(repo_ids_match(actual, expected), matches);
    }

    #[rstest]
    #[case::config_overrides_copier_and_visibility(
        Some(json!("fr")),
        Some("repo_language: ja\n"),
        Some(false),
        Some(json!("fr")),
    )]
    #[case::copier_overrides_visibility(
        None,
        Some("repo_language: ja\n"),
        Some(false),
        Some(json!("ja")),
    )]
    #[case::private_visibility_fallback(None, None, Some(true), Some(json!("ja")))]
    #[case::public_visibility_fallback(None, None, Some(false), Some(json!("en")))]
    #[case::no_repo_context(None, None, None, None)]
    #[tokio::test]
    async fn resolve_repo_language_uses_config_then_copier_then_visibility(
        #[case] configured: Option<Value>,
        #[case] copier_answers: Option<&str>,
        #[case] is_private: Option<bool>,
        #[case] expected: Option<Value>,
    ) {
        let temp_dir = tempfile::tempdir().unwrap();
        let copier_answers_path = copier_answers.map(|contents| {
            let path = temp_dir.path().join(".copier-answers.yml");
            fs::write(&path, contents).unwrap();
            path
        });

        let actual = resolve_repo_language(configured, copier_answers_path.as_deref(), || async {
            Ok(is_private)
        })
        .await
        .unwrap();

        assert_eq!(actual, expected);
    }

    #[rstest]
    #[case::missing_key("project_name: sample-project\n", None)]
    #[case::empty_value("repo_language: \"\"\n", None)]
    #[case::non_string_value("repo_language: [ja]\n", None)]
    #[case::terminal_control_character("repo_language: \"ja\\u001b[31m\"\n", None)]
    #[case::repo_language(indoc! {"
        repo_language: ja
    "}, Some("ja"))]
    #[tokio::test]
    async fn copier_answers_file_returns_only_nonempty_plain_language_values(
        #[case] contents: &str,
        #[case] expected: Option<&str>,
    ) {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join(".copier-answers.yml");
        fs::write(&path, contents).unwrap();

        let result = resolve_repo_language(None, Some(&path), || async { Ok(None) })
            .await
            .unwrap();

        assert_eq!(
            result.and_then(|value| value.as_str().map(str::to_owned)),
            expected.map(str::to_owned)
        );
    }
}
