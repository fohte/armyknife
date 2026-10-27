use anyhow::Context;
use serde::{Deserialize, Serialize};

use crate::commands::agent::types::{Engine, ReasoningEffort};

use super::{AgentConfig, format_sorted_names};

/// Engines accepted in profiles, including values that cannot launch agent sessions.
#[cfg_attr(feature = "schema-gen", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentProfileTypeEngine {
    Claude,
    Codex,
    Agy,
}

impl AgentProfileTypeEngine {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Agy => "agy",
        }
    }
}

/// Execution settings for one type in an agent profile.
#[cfg_attr(feature = "schema-gen", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AgentProfileTypeConfig {
    /// Coding agent CLI configured for this type.
    pub engine: AgentProfileTypeEngine,

    /// Model passed to the selected engine; omitted to use its CLI default.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "schema-gen", schemars(length(min = 1)))]
    pub model: Option<String>,

    /// Reasoning effort used for this type; unavailable for the agy engine.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<ReasoningEffort>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawAgentProfileTypeConfig {
    engine: AgentProfileTypeEngine,
    #[serde(default, deserialize_with = "deserialize_optional_non_empty_model")]
    model: Option<String>,
    #[serde(default)]
    reasoning_effort: Option<ReasoningEffort>,
}

impl<'de> Deserialize<'de> for AgentProfileTypeConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = RawAgentProfileTypeConfig::deserialize(deserializer)?;
        if raw.engine == AgentProfileTypeEngine::Agy && raw.reasoning_effort.is_some() {
            return Err(serde::de::Error::custom(
                "reasoning_effort is not supported for the agy engine",
            ));
        }

        Ok(Self {
            engine: raw.engine,
            model: raw.model,
            reasoning_effort: raw.reasoning_effort,
        })
    }
}

fn deserialize_optional_non_empty_model<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let model = Option::<String>::deserialize(deserializer)?;
    if model.as_deref() == Some("") {
        return Err(<D::Error as serde::de::Error>::custom(
            "model must be a non-empty string",
        ));
    }
    Ok(model)
}

/// A profile and type resolved from `agent.active_profile`.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedAgentProfileType {
    pub profile_name: String,
    pub type_name: String,
    pub settings: AgentProfileTypeConfig,
}

impl AgentConfig {
    /// Resolve a type from the active profile without applying any fallback.
    pub fn resolve_profile_type(
        &self,
        type_name: &str,
    ) -> anyhow::Result<ResolvedAgentProfileType> {
        let profile_name = self
            .active_profile
            .as_deref()
            .context("agent.active_profile is not set")?;
        let profile = self.profiles.get(profile_name).with_context(|| {
            format!("agent.active_profile references undefined profile `{profile_name}`")
        })?;
        let settings = profile.get(type_name).with_context(|| {
            let configured = format_sorted_names(profile.keys());
            format!(
                "unknown agent type `{type_name}` in profile `{profile_name}`; configured types: {configured}"
            )
        })?;

        Ok(ResolvedAgentProfileType {
            profile_name: profile_name.to_string(),
            type_name: type_name.to_string(),
            settings: settings.clone(),
        })
    }
}

/// Per-engine defaults selected by an `agent.launch_profiles` entry.
#[cfg_attr(feature = "schema-gen", derive(schemars::JsonSchema))]
#[derive(Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AgentLaunchEngineConfig {
    /// Model passed to the selected engine when `--model` is omitted.
    #[serde(default)]
    pub model: Option<String>,

    /// Reasoning effort used when `--reasoning-effort` is omitted.
    #[serde(default)]
    pub reasoning_effort: Option<ReasoningEffort>,
}

/// Launch defaults for one `--kind` value.
#[cfg_attr(feature = "schema-gen", derive(schemars::JsonSchema))]
#[derive(Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AgentLaunchProfileConfig {
    /// Engine selected when `a agent new --engine` is omitted.
    #[serde(default)]
    pub engine: Option<Engine>,

    /// Claude-specific model and reasoning defaults.
    #[serde(default)]
    pub claude: Option<AgentLaunchEngineConfig>,

    /// Codex-specific model and reasoning defaults.
    #[serde(default)]
    pub codex: Option<AgentLaunchEngineConfig>,
}

impl AgentLaunchProfileConfig {
    fn engine_config(&self, engine: Engine) -> Option<&AgentLaunchEngineConfig> {
        match engine {
            Engine::Claude => self.claude.as_ref(),
            Engine::Codex => self.codex.as_ref(),
        }
    }
}

/// Effective engine, model, and effort after applying profile and global defaults.
#[derive(Debug, PartialEq)]
pub struct ResolvedLaunchOptions {
    pub engine: Engine,
    pub model: Option<String>,
    pub reasoning_effort: Option<ReasoningEffort>,
}

impl AgentConfig {
    /// Resolve launch values from explicit flags, the selected kind profile, and global defaults.
    pub fn resolve_launch_options(
        &self,
        kind: Option<&str>,
        explicit_engine: Option<Engine>,
        explicit_model: Option<&str>,
        explicit_reasoning_effort: Option<ReasoningEffort>,
    ) -> ResolvedLaunchOptions {
        let profile = kind.and_then(|kind| self.launch_profiles.get(kind));
        let engine = explicit_engine
            .or_else(|| profile.and_then(|profile| profile.engine))
            .unwrap_or(self.default_engine);
        let engine_config = profile.and_then(|profile| profile.engine_config(engine));
        let codex_config = (engine == Engine::Codex).then_some(&self.codex);

        ResolvedLaunchOptions {
            engine,
            model: explicit_model
                .map(str::to_owned)
                .or_else(|| engine_config.and_then(|config| config.model.clone()))
                .or_else(|| codex_config.and_then(|config| config.model.clone())),
            reasoning_effort: explicit_reasoning_effort
                .or_else(|| engine_config.and_then(|config| config.reasoning_effort))
                .or_else(|| codex_config.and_then(|config| config.reasoning_effort)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shared::config::{CodexConfig, Config};
    use indoc::indoc;
    use rstest::{fixture, rstest};
    use std::collections::HashMap;

    #[fixture]
    fn agent_config() -> AgentConfig {
        AgentConfig {
            default_engine: Engine::Codex,
            codex: CodexConfig {
                model: Some("global-codex-model".to_string()),
                reasoning_effort: Some(ReasoningEffort::High),
            },
            launch_profiles: HashMap::from([(
                "sample-flow".to_string(),
                AgentLaunchProfileConfig {
                    engine: Some(Engine::Claude),
                    claude: Some(AgentLaunchEngineConfig {
                        model: Some("profile-claude-model".to_string()),
                        reasoning_effort: Some(ReasoningEffort::Low),
                    }),
                    codex: Some(AgentLaunchEngineConfig {
                        model: Some("profile-codex-model".to_string()),
                        reasoning_effort: Some(ReasoningEffort::Max),
                    }),
                },
            )]),
            ..Default::default()
        }
    }

    #[derive(Clone, Copy)]
    enum ProfileAdjustment {
        Unchanged,
        RemoveCodexDefaults,
        OmitEngineAndCodexEffort,
        UseClaudeDefault,
    }

    #[test]
    fn parse_launch_profiles_yaml() {
        let yaml = indoc! {
            "
            agent:
              launch_profiles:
                sample-flow:
                  engine: claude
                  claude:
                    model: claude-test-model
                  codex:
                    model: codex-test-model
                    reasoning_effort: high
            "
        };
        let config: Config = serde_yaml::from_str(yaml).unwrap();

        assert_eq!(
            config.agent.launch_profiles,
            HashMap::from([(
                "sample-flow".to_string(),
                AgentLaunchProfileConfig {
                    engine: Some(Engine::Claude),
                    claude: Some(AgentLaunchEngineConfig {
                        model: Some("claude-test-model".to_string()),
                        reasoning_effort: None,
                    }),
                    codex: Some(AgentLaunchEngineConfig {
                        model: Some("codex-test-model".to_string()),
                        reasoning_effort: Some(ReasoningEffort::High),
                    }),
                },
            )]),
        );
    }

    #[rstest]
    #[case::kind_profile_selects_engine(
        Some("sample-flow"),
        None,
        None,
        None,
        ProfileAdjustment::Unchanged,
        ResolvedLaunchOptions {
            engine: Engine::Claude,
            model: Some("profile-claude-model".to_string()),
            reasoning_effort: Some(ReasoningEffort::Low),
        }
    )]
    #[case::explicit_engine_selects_its_profile(
        Some("sample-flow"),
        Some(Engine::Codex),
        None,
        None,
        ProfileAdjustment::Unchanged,
        ResolvedLaunchOptions {
            engine: Engine::Codex,
            model: Some("profile-codex-model".to_string()),
            reasoning_effort: Some(ReasoningEffort::Max),
        }
    )]
    #[case::explicit_values_win(
        Some("sample-flow"),
        Some(Engine::Codex),
        Some("cli-model"),
        Some(ReasoningEffort::Medium),
        ProfileAdjustment::Unchanged,
        ResolvedLaunchOptions {
            engine: Engine::Codex,
            model: Some("cli-model".to_string()),
            reasoning_effort: Some(ReasoningEffort::Medium),
        }
    )]
    #[case::missing_profile_preserves_global_defaults(
        None,
        None,
        None,
        None,
        ProfileAdjustment::Unchanged,
        ResolvedLaunchOptions {
            engine: Engine::Codex,
            model: Some("global-codex-model".to_string()),
            reasoning_effort: Some(ReasoningEffort::High),
        }
    )]
    #[case::explicit_engine_does_not_use_other_profile_defaults(
        Some("sample-flow"),
        Some(Engine::Codex),
        None,
        None,
        ProfileAdjustment::RemoveCodexDefaults,
        ResolvedLaunchOptions {
            engine: Engine::Codex,
            model: Some("global-codex-model".to_string()),
            reasoning_effort: Some(ReasoningEffort::High),
        }
    )]
    #[case::profile_values_fall_back_independently(
        Some("sample-flow"),
        None,
        None,
        None,
        ProfileAdjustment::OmitEngineAndCodexEffort,
        ResolvedLaunchOptions {
            engine: Engine::Codex,
            model: Some("profile-codex-model".to_string()),
            reasoning_effort: Some(ReasoningEffort::High),
        }
    )]
    #[case::claude_ignores_codex_globals_without_profile(
        None,
        None,
        None,
        None,
        ProfileAdjustment::UseClaudeDefault,
        ResolvedLaunchOptions {
            engine: Engine::Claude,
            model: None,
            reasoning_effort: None,
        }
    )]
    fn resolves_launch_options(
        agent_config: AgentConfig,
        #[case] kind: Option<&str>,
        #[case] explicit_engine: Option<Engine>,
        #[case] explicit_model: Option<&str>,
        #[case] explicit_effort: Option<ReasoningEffort>,
        #[case] adjustment: ProfileAdjustment,
        #[case] expected: ResolvedLaunchOptions,
    ) {
        let mut agent_config = agent_config;
        match adjustment {
            ProfileAdjustment::Unchanged => {}
            ProfileAdjustment::RemoveCodexDefaults => {
                agent_config
                    .launch_profiles
                    .get_mut("sample-flow")
                    .unwrap()
                    .codex = None;
            }
            ProfileAdjustment::OmitEngineAndCodexEffort => {
                let profile = agent_config.launch_profiles.get_mut("sample-flow").unwrap();
                profile.engine = None;
                profile.codex = Some(AgentLaunchEngineConfig {
                    model: Some("profile-codex-model".to_string()),
                    reasoning_effort: None,
                });
            }
            ProfileAdjustment::UseClaudeDefault => agent_config.default_engine = Engine::Claude,
        }

        assert_eq!(
            agent_config.resolve_launch_options(
                kind,
                explicit_engine,
                explicit_model,
                explicit_effort,
            ),
            expected,
        );
    }

    #[test]
    fn resolves_arbitrary_profile_and_type_names_to_exact_settings() {
        let config: Config = serde_yaml::from_str(indoc! {"
            agent:
              active_profile: \"profile one\"
              profiles:
                \"profile one\":
                  \"review type\":
                    engine: codex
                    model: model-ember
                    reasoning_effort: high
        "})
        .unwrap();

        assert_eq!(
            config.agent.resolve_profile_type("review type").unwrap(),
            ResolvedAgentProfileType {
                profile_name: "profile one".to_string(),
                type_name: "review type".to_string(),
                settings: AgentProfileTypeConfig {
                    engine: AgentProfileTypeEngine::Codex,
                    model: Some("model-ember".to_string()),
                    reasoning_effort: Some(ReasoningEffort::High),
                },
            },
        );
    }

    #[rstest]
    #[case::cli_defaults(indoc! {"
        engine: claude
    "}, AgentProfileTypeConfig {
        engine: AgentProfileTypeEngine::Claude,
        model: None,
        reasoning_effort: None,
    })]
    #[case::agy_model_only(indoc! {"
        engine: agy
        model: sample-review-model
    "}, AgentProfileTypeConfig {
        engine: AgentProfileTypeEngine::Agy,
        model: Some("sample-review-model".to_string()),
        reasoning_effort: None,
    })]
    fn parses_optional_profile_type_settings(
        #[case] yaml: &str,
        #[case] expected: AgentProfileTypeConfig,
    ) {
        assert_eq!(
            serde_yaml::from_str::<AgentProfileTypeConfig>(yaml).unwrap(),
            expected
        );
    }

    #[rstest]
    #[case::missing_engine(indoc! {"
        model: model-ember
        reasoning_effort: high
    "})]
    #[case::empty_model(indoc! {"
        engine: codex
        model: \"\"
        reasoning_effort: high
    "})]
    #[case::agy_reasoning_effort(indoc! {"
        engine: agy
        reasoning_effort: low
    "})]
    #[case::unknown_engine(indoc! {"
        engine: unknown
    "})]
    #[case::unknown_field(indoc! {"
        engine: codex
        model: model-ember
        reasoning_effort: high
        extra: true
    "})]
    fn rejects_invalid_profile_type_settings(#[case] yaml: &str) {
        let result = serde_yaml::from_str::<AgentProfileTypeConfig>(yaml)
            .map(|_| ())
            .map_err(|_| ());

        assert_eq!(result, Err(()));
    }

    #[rstest]
    #[case::profile_unset(None, "review type", "agent.active_profile is not set")]
    #[case::profile_undefined(
        Some("missing profile"),
        "review type",
        "agent.active_profile references undefined profile `missing profile`"
    )]
    #[case::type_undefined(
        Some("profile one"),
        "missing type",
        "unknown agent type `missing type` in profile `profile one`; configured types: review type"
    )]
    fn rejects_missing_profile_or_type_references(
        #[case] active_profile: Option<&str>,
        #[case] type_name: &str,
        #[case] expected_error: &str,
    ) {
        let agent = AgentConfig {
            active_profile: active_profile.map(str::to_string),
            profiles: HashMap::from([(
                "profile one".to_string(),
                HashMap::from([(
                    "review type".to_string(),
                    AgentProfileTypeConfig {
                        engine: AgentProfileTypeEngine::Claude,
                        model: Some("model-raven".to_string()),
                        reasoning_effort: Some(ReasoningEffort::Low),
                    },
                )]),
            )]),
            ..Default::default()
        };

        let actual = agent
            .resolve_profile_type(type_name)
            .err()
            .map(|error| error.to_string());

        assert_eq!(actual, Some(expected_error.to_string()));
    }
}
