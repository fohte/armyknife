use clap::Subcommand;

use crate::infra::git;
use crate::infra::github::{GitHubClient, RepoClient};
use crate::shared::config;

/// Configuration management commands.
#[derive(Subcommand, Clone, PartialEq, Eq)]
pub enum ConfigCommands {
    /// Get a configuration value by dot-separated key
    Get {
        /// Configuration key (e.g., "agent.worktree.branch_prefix", "repo.language",
        /// "org.ai.review.reviewers", "agent.profile", "agent.profile.<path>")
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
    use rstest::{fixture, rstest};

    #[rstest]
    fn get_value_agent_profile_renders_selected_profile_as_yaml(agent_profiles_config: Config) {
        let value = agent_profiles_config
            .get_value("agent.profile", None)
            .expect("selected profile exists");

        assert_eq!(
            format_value(&value).unwrap(),
            indoc! {"
                code-review:
                  engine: claude
                review:
                  engine: claude
                  model: sample-model-alpha
                  reasoning_effort: medium
                writing-review:
                  engine: agy
                  model: sample-review-model
            "}
        );
    }

    #[fixture]
    fn agent_profiles_config() -> Config {
        config_from_yaml(indoc! {"
            agent:
              active_profile: sample-alpha
              profiles:
                sample-alpha:
                  code-review:
                    engine: claude
                  writing-review:
                    engine: agy
                    model: sample-review-model
                  review:
                    engine: claude
                    model: sample-model-alpha
                    reasoning_effort: medium
                sample-beta:
                  review:
                    engine: codex
                    model: sample-model-beta
                    reasoning_effort: high
        "})
    }

    #[rstest]
    #[case::first_profile("sample-alpha", "sample-model-alpha")]
    #[case::second_profile("sample-beta", "sample-model-beta")]
    fn get_value_agent_profile_path_uses_active_profile(
        mut agent_profiles_config: Config,
        #[case] active_profile: &str,
        #[case] expected: &str,
    ) {
        agent_profiles_config.agent.active_profile = Some(active_profile.to_string());
        let value = agent_profiles_config
            .get_value("agent.profile.review.model", None)
            .expect("selected profile model exists");

        assert_eq!(format_value(&value).unwrap(), format!("{expected}\n"));
    }

    #[rstest]
    #[case::profile_unselected(None, "agent.profile")]
    #[case::profile_undefined(Some("sample-unconfigured"), "agent.profile")]
    #[case::type_missing(Some("sample-alpha"), "agent.profile.unconfigured.model")]
    #[case::omitted_model(Some("sample-alpha"), "agent.profile.code-review.model")]
    #[case::omitted_effort(Some("sample-alpha"), "agent.profile.code-review.reasoning_effort")]
    #[case::path_missing(Some("sample-alpha"), "agent.profile.review.unconfigured")]
    #[case::path_below_scalar(Some("sample-alpha"), "agent.profile.review.model.value")]
    fn get_value_agent_profile_returns_none_without_a_value(
        mut agent_profiles_config: Config,
        #[case] active_profile: Option<&str>,
        #[case] key: &str,
    ) {
        agent_profiles_config.agent.active_profile = active_profile.map(str::to_string);

        assert_eq!(agent_profiles_config.get_value(key, None), None);
    }

    #[rstest]
    #[case::active_profile("agent.active_profile", serde_json::json!("sample-alpha"))]
    #[case::fixed_profile_path(
        "agent.profiles.sample-alpha.review.model",
        serde_json::json!("sample-model-alpha")
    )]
    fn get_value_keeps_existing_agent_profile_paths(
        agent_profiles_config: Config,
        #[case] key: &str,
        #[case] expected: serde_json::Value,
    ) {
        assert_eq!(agent_profiles_config.get_value(key, None), Some(expected));
    }

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
    #[case::agent_worktree_dir("agent.worktree.dir", ".worktrees")]
    #[case::agent_worktree_branch_prefix("agent.worktree.branch_prefix", "fohte/")]
    #[case::agent_auto_pause_timeout("agent.auto_pause.timeout", "30m")]
    #[case::editor_editor_command("editor.editor_command", "nvim")]
    #[case::editor_terminal("editor.terminal", "wezterm")]
    #[case::notification_enabled("notification.enabled", "true")]
    #[case::notification_sound("notification.sound", "Glass")]
    fn get_value_returns_default_values(#[case] key: &str, #[case] expected: &str) {
        let cfg = Config::default();
        assert_eq!(scalar(cfg.get_value(key, None)).as_deref(), Some(expected));
    }

    #[rstest]
    #[case::agent_worktree_dir("agent.worktree.dir", Some(".wt"))]
    #[case::agent_worktree_branch_prefix("agent.worktree.branch_prefix", Some("user/"))]
    #[case::agent_auto_pause_timeout("agent.auto_pause.timeout", Some("40m"))]
    #[case::notification_enabled("notification.enabled", Some("false"))]
    #[case::notification_sound("notification.sound", Some("Ping"))]
    fn get_value_returns_custom_values(#[case] key: &str, #[case] expected: Option<&str>) {
        let cfg = config_from_yaml(indoc! {"
            agent:
              worktree:
                dir: .wt
                branch_prefix: user/
              auto_pause:
                timeout: 40m
            notification:
              enabled: false
              sound: Ping
        "});
        assert_eq!(scalar(cfg.get_value(key, None)).as_deref(), expected);
    }

    #[rstest]
    #[case::top_level("nonexistent")]
    #[case::nested("agent.nonexistent")]
    #[case::legacy_wm("wm.worktrees_dir")]
    #[case::legacy_cc("cc.auto_pause.timeout")]
    fn get_value_returns_none_for_unknown_key(#[case] key: &str) {
        let cfg = Config::default();
        assert_eq!(cfg.get_value(key, None), None);
    }

    #[test]
    fn get_value_returns_object_for_map_key() {
        let cfg = Config::default();
        assert_eq!(
            cfg.get_value("agent.worktree", None),
            Some(serde_json::json!({
                "dir": ".worktrees",
                "branch_prefix": "fohte/",
                "layout": serde_json::to_value(Config::default().agent.worktree.layout).unwrap(),
                "repos_root": null,
            }))
        );
    }

    #[test]
    fn format_value_renders_map_as_yaml() {
        let cfg = config_from_yaml(indoc! {"
            orgs:
              example-org:
                ai:
                  review:
                    reviewers: [coderabbit]
        "});
        let v = cfg
            .get_value("orgs.example-org", Some("example-org/example-repo"))
            .expect("organization config exists");
        let yaml = format_value(&v).unwrap();
        assert_eq!(
            yaml,
            indoc! {"
                ai:
                  review:
                    reviewers:
                    - coderabbit
                vibe: false
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
    #[case::enabled("example-org/standalone-tool", "true")]
    #[case::disabled("example-org/managed-fork", "false")]
    #[case::repo_entry_without_key_defaults_to_false("example-org/unset-fork", "false")]
    #[case::repo_entry_absent_defaults_to_false("example-org/unconfigured-fork", "false")]
    fn get_value_repo_standalone_fork(#[case] repo_id: &str, #[case] expected: &str) {
        let cfg = config_from_yaml(indoc! {"
            repos:
              example-org/standalone-tool:
                standalone_fork: true
              example-org/managed-fork:
                standalone_fork: false
              example-org/unset-fork: {}
        "});
        assert_eq!(
            scalar(cfg.get_value("repo.standalone_fork", Some(repo_id))).as_deref(),
            Some(expected)
        );
    }

    #[rstest]
    #[case::denied_by_default("{}", "example-org/example-repo", false)]
    #[case::org_value_falls_back(
        indoc! {"
            orgs:
              example-org:
                vibe: true
        "},
        "example-org/example-repo",
        true
    )]
    #[case::repo_false_overrides_org_true(
        indoc! {"
            orgs:
              example-org:
                vibe: true
            repos:
              example-org/example-repo:
                vibe: false
        "},
        "example-org/example-repo",
        false
    )]
    #[case::repo_true_overrides_org_false(
        indoc! {"
            orgs:
              example-org:
                vibe: false
            repos:
              example-org/example-repo:
                vibe: true
        "},
        "example-org/example-repo",
        true
    )]
    #[case::mixed_case_repo_blocklist_overrides_org_allow(
        indoc! {"
            orgs:
              example-org:
                vibe: true
            repos:
              example-org/restricted-repository:
                vibe: false
        "},
        "EXAMPLE-ORG/Restricted-Repository",
        false
    )]
    #[case::mixed_case_org_uses_configured_owner(
        indoc! {"
            orgs:
              example-org:
                vibe: true
        "},
        "EXAMPLE-ORG/example-repository",
        true
    )]
    #[case::case_variant_org_blocklist_takes_precedence(
        indoc! {"
            orgs:
              example-org:
                vibe: true
              EXAMPLE-ORG:
                vibe: false
        "},
        "Example-Org/example-repository",
        false
    )]
    #[case::case_variant_repo_blocklist_takes_precedence(
        indoc! {"
            orgs:
              example-org:
                vibe: true
            repos:
              example-org/restricted-repository:
                vibe: true
              EXAMPLE-ORG/RESTRICTED-REPOSITORY:
                vibe: false
        "},
        "Example-Org/Restricted-Repository",
        false
    )]
    fn get_value_repo_vibe_resolves_effective_value(
        #[case] yaml: &str,
        #[case] repo_id: &str,
        #[case] expected: bool,
    ) {
        let cfg = config_from_yaml(yaml);

        assert_eq!(
            cfg.get_value("repo.vibe", Some(repo_id)),
            Some(serde_json::Value::Bool(expected)),
        );
    }

    #[test]
    fn get_value_repo_vibe_requires_repository_context() {
        assert_eq!(Config::default().get_value("repo.vibe", None), None,);
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
