use serde::{Deserialize, Serialize};

use crate::commands::agent::types::{Engine, ReasoningEffort};

use super::AgentConfig;

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

/// Effective launch settings shared by `a agent new` and profile inspection commands.
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
        ResolvedLaunchOptions {
            engine: Engine::Codex,
            model: Some("global-codex-model".to_string()),
            reasoning_effort: Some(ReasoningEffort::High),
        }
    )]
    fn resolves_launch_options(
        agent_config: AgentConfig,
        #[case] kind: Option<&str>,
        #[case] explicit_engine: Option<Engine>,
        #[case] explicit_model: Option<&str>,
        #[case] explicit_effort: Option<ReasoningEffort>,
        #[case] expected: ResolvedLaunchOptions,
    ) {
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
    fn explicit_engine_does_not_use_another_engines_profile_defaults() {
        let agent_config = AgentConfig {
            default_engine: Engine::Claude,
            codex: CodexConfig {
                model: Some("global-codex-model".to_string()),
                reasoning_effort: Some(ReasoningEffort::High),
            },
            launch_profiles: HashMap::from([(
                "sample-flow".to_string(),
                AgentLaunchProfileConfig {
                    engine: Some(Engine::Claude),
                    claude: Some(AgentLaunchEngineConfig {
                        model: Some("claude-profile-model".to_string()),
                        reasoning_effort: Some(ReasoningEffort::Low),
                    }),
                    codex: None,
                },
            )]),
            ..Default::default()
        };

        assert_eq!(
            agent_config.resolve_launch_options(
                Some("sample-flow"),
                Some(Engine::Codex),
                None,
                None,
            ),
            ResolvedLaunchOptions {
                engine: Engine::Codex,
                model: Some("global-codex-model".to_string()),
                reasoning_effort: Some(ReasoningEffort::High),
            },
        );
    }

    #[test]
    fn profile_defaults_fall_back_to_global_values_independently() {
        let agent_config = AgentConfig {
            default_engine: Engine::Codex,
            codex: CodexConfig {
                model: Some("global-codex-model".to_string()),
                reasoning_effort: Some(ReasoningEffort::High),
            },
            launch_profiles: HashMap::from([(
                "sample-flow".to_string(),
                AgentLaunchProfileConfig {
                    engine: None,
                    claude: None,
                    codex: Some(AgentLaunchEngineConfig {
                        model: Some("profile-codex-model".to_string()),
                        reasoning_effort: None,
                    }),
                },
            )]),
            ..Default::default()
        };

        assert_eq!(
            agent_config.resolve_launch_options(Some("sample-flow"), None, None, None),
            ResolvedLaunchOptions {
                engine: Engine::Codex,
                model: Some("profile-codex-model".to_string()),
                reasoning_effort: Some(ReasoningEffort::High),
            },
        );
    }
}
