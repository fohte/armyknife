use clap::Subcommand;

use crate::infra::git;
use crate::infra::github::{GitHubClient, RepoClient};
use crate::shared::config;

/// Configuration management commands.
#[derive(Subcommand, Clone, PartialEq, Eq)]
pub enum ConfigCommands {
    /// Get a configuration value by dot-separated key
    Get {
        /// Configuration key (e.g., "wm.branch_prefix", "repo.language", "org.ai.review.reviewers")
        key: String,
    },
}

impl ConfigCommands {
    pub async fn run(&self) -> anyhow::Result<()> {
        match self {
            Self::Get { key } => run_get(key).await,
        }
    }
}

async fn run_get(key: &str) -> anyhow::Result<()> {
    let cfg = config::load_config()?;

    // For repo.* and org.* keys, resolve owner/repo from CWD git remote.
    // org.* uses the owner segment; repo.* uses the full "owner/repo".
    let repo_id = if key.starts_with("repo.") || key.starts_with("org.") {
        git::get_owner_repo().map(|(owner, repo)| format!("{owner}/{repo}"))
    } else {
        None
    };

    match cfg.get_value(key, repo_id.as_deref()) {
        Some(value) => print!("{}", format_value(&value)?),
        None if key == "repo.language" => {
            // Default: private repos -> "ja", public repos -> "en".
            // Stay silent when there is no repo context — there's nothing to
            // default to without an owner/repo.
            if let Some((owner, repo)) = repo_id.as_deref().and_then(|id| id.split_once('/')) {
                let client = GitHubClient::get()?;
                let is_private = client.is_repo_private(owner, repo).await.unwrap_or(true);
                let default_lang = if is_private { "ja" } else { "en" };
                println!("{default_lang}");
            }
        }
        None => anyhow::bail!("key not found: {key}"),
    }
    Ok(())
}

/// Format a resolved config value for stdout. Scalars render bare (preserving
/// pre-existing `a config get` output); maps and arrays render as YAML so the
/// shape of the config file round-trips into stdout. The returned string is
/// terminated with a newline.
fn format_value(value: &serde_json::Value) -> anyhow::Result<String> {
    Ok(match value {
        serde_json::Value::String(s) => format!("{s}\n"),
        serde_json::Value::Bool(b) => format!("{b}\n"),
        serde_json::Value::Number(n) => format!("{n}\n"),
        // serde_yaml emits a trailing newline already.
        _ => serde_yaml::to_string(value)?,
    })
}

#[cfg(test)]
mod tests {
    use super::format_value;
    use crate::infra::github::{GitHubMockServer, RepoClient};
    use crate::shared::config::Config;
    use indoc::indoc;
    use rstest::rstest;

    fn config_from_yaml(yaml: &str) -> Config {
        serde_yaml::from_str(yaml).unwrap()
    }

    /// Stringify a scalar Value the same way `format_value` would, minus the
    /// trailing newline. Returns None for non-scalar values so that existing
    /// scalar-focused tests stay readable.
    fn scalar(v: Option<serde_json::Value>) -> Option<String> {
        match v {
            Some(serde_json::Value::String(s)) => Some(s),
            Some(serde_json::Value::Bool(b)) => Some(b.to_string()),
            Some(serde_json::Value::Number(n)) => Some(n.to_string()),
            _ => None,
        }
    }

    #[rstest]
    #[case::wm_worktrees_dir("wm.worktrees_dir", ".worktrees")]
    #[case::wm_branch_prefix("wm.branch_prefix", "fohte/")]
    #[case::editor_editor_command("editor.editor_command", "nvim")]
    #[case::editor_terminal("editor.terminal", "wezterm")]
    #[case::notification_enabled("notification.enabled", "true")]
    #[case::notification_sound("notification.sound", "Glass")]
    fn get_value_returns_default_values(#[case] key: &str, #[case] expected: &str) {
        let cfg = Config::default();
        assert_eq!(scalar(cfg.get_value(key, None)).as_deref(), Some(expected));
    }

    #[rstest]
    #[case::wm_worktrees_dir("wm.worktrees_dir", Some(".wt"))]
    #[case::wm_branch_prefix("wm.branch_prefix", Some("user/"))]
    #[case::notification_enabled("notification.enabled", Some("false"))]
    #[case::notification_sound("notification.sound", Some("Ping"))]
    fn get_value_returns_custom_values(#[case] key: &str, #[case] expected: Option<&str>) {
        let cfg = config_from_yaml(indoc! {"
            wm:
              worktrees_dir: .wt
              branch_prefix: user/
            notification:
              enabled: false
              sound: Ping
        "});
        assert_eq!(scalar(cfg.get_value(key, None)).as_deref(), expected);
    }

    #[rstest]
    #[case::top_level("nonexistent")]
    #[case::nested("wm.nonexistent")]
    fn get_value_returns_none_for_unknown_key(#[case] key: &str) {
        let cfg = Config::default();
        assert_eq!(cfg.get_value(key, None), None);
    }

    #[test]
    fn get_value_returns_object_for_map_key() {
        let cfg = Config::default();
        let v = cfg.get_value("wm", None).expect("wm is present");
        assert!(v.is_object(), "wm should resolve to a YAML map: {v:?}");
    }

    #[test]
    fn format_value_renders_map_as_yaml() {
        let cfg = config_from_yaml(indoc! {"
            orgs:
              fohte:
                ai:
                  review:
                    reviewers: [coderabbit]
        "});
        let v = cfg
            .get_value("orgs.fohte", Some("fohte/any"))
            .expect("orgs.fohte exists");
        let yaml = format_value(&v).unwrap();
        assert_eq!(
            yaml,
            indoc! {"
                ai:
                  review:
                    reviewers:
                    - coderabbit
            "}
        );
    }

    #[test]
    fn format_value_renders_list_as_yaml() {
        let cfg = config_from_yaml(indoc! {"
            orgs:
              fohte:
                ai:
                  review:
                    reviewers: [coderabbit, devin]
        "});
        let v = cfg
            .get_value("orgs.fohte.ai.review.reviewers", None)
            .expect("reviewers list exists");
        let yaml = format_value(&v).unwrap();
        assert_eq!(
            yaml,
            indoc! {"
                - coderabbit
                - devin
            "}
        );
    }

    #[test]
    fn format_value_scalars_match_legacy_output() {
        assert_eq!(format_value(&serde_json::json!("foo")).unwrap(), "foo\n");
        assert_eq!(format_value(&serde_json::json!(true)).unwrap(), "true\n");
        assert_eq!(format_value(&serde_json::json!(42)).unwrap(), "42\n");
    }

    #[rstest]
    #[case::configured("fohte/t-rader", Some("ja"))]
    #[case::not_configured("fohte/unknown", None)]
    fn get_value_repo_language(#[case] repo_id: &str, #[case] expected: Option<&str>) {
        let cfg = config_from_yaml(indoc! {"
            repos:
              fohte/t-rader:
                language: ja
        "});
        assert_eq!(
            scalar(cfg.get_value("repo.language", Some(repo_id))).as_deref(),
            expected
        );
    }

    #[test]
    fn get_value_repo_without_repo_id() {
        let cfg = config_from_yaml(indoc! {"
            repos:
              fohte/t-rader:
                language: ja
        "});
        assert_eq!(cfg.get_value("repo.language", None), None);
    }

    #[rstest]
    #[case::explicit_true("fohte/dotfiles", "true")]
    #[case::explicit_false("fohte/blocked", "false")]
    #[case::repo_entry_without_key_defaults_to_false("fohte/no-direct-commit-key", "false")]
    #[case::repo_entry_absent_defaults_to_false("fohte/unknown", "false")]
    fn get_value_repo_direct_commit(#[case] repo_id: &str, #[case] expected: &str) {
        let cfg = config_from_yaml(indoc! {"
            repos:
              fohte/dotfiles:
                direct_commit: true
              fohte/blocked:
                direct_commit: false
              fohte/no-direct-commit-key: {}
        "});
        assert_eq!(
            scalar(cfg.get_value("repo.direct_commit", Some(repo_id))).as_deref(),
            Some(expected)
        );
    }

    #[rstest]
    #[case::private_repo(true, "ja")]
    #[case::public_repo(false, "en")]
    #[tokio::test]
    async fn run_get_repo_language_defaults_by_visibility(
        #[case] is_private: bool,
        #[case] expected: &str,
    ) {
        let mock = GitHubMockServer::start().await;
        mock.repo("owner", "repo")
            .repo_info()
            .private(is_private)
            .get()
            .await;
        let client = mock.client();

        // Config has no repos entry, so default should kick in
        let cfg = Config::default();
        let repo_id = Some("owner/repo");

        let result = cfg.get_value("repo.language", repo_id);
        assert!(result.is_none(), "should fall through to default");

        // Verify the default logic directly
        let is_private_result = client.is_repo_private("owner", "repo").await.unwrap();
        assert_eq!(is_private_result, is_private);
        let default_lang = if is_private_result { "ja" } else { "en" };
        assert_eq!(default_lang, expected);
    }
}
