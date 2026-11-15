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
        if format!("{owner}/{name}") != expected_repo_id {
            return None;
        }
    }
    Some(repo.workdir().join(".copier-answers.yml"))
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
    use super::resolve_repo_language;
    use indoc::indoc;
    use rstest::rstest;
    use serde_json::{Value, json};
    use std::fs;

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
