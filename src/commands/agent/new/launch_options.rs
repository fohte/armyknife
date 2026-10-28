use anyhow::{Context, Result, bail};

use crate::commands::agent::types::{Engine, ReasoningEffort};
use crate::shared::config::{
    AgentProfileTypeEngine, Config, ResolvedAgentProfileType, ResolvedLaunchOptions,
};

use super::CommonNewArgs;

pub(super) fn resolve_launch_options(
    common: &CommonNewArgs,
    config: &Config,
) -> Result<ResolvedLaunchOptions> {
    if let Some(type_name) = common.type_name.as_deref() {
        let resolved = config.agent.resolve_profile_type(type_name)?;
        return Ok(resolve_profile_type_launch_options(common, &resolved)?.into());
    }

    Ok(config.agent.resolve_launch_options(
        common.kind.as_deref(),
        common.engine,
        common.model.as_deref(),
        common.reasoning_effort,
    ))
}

#[derive(Debug, PartialEq)]
pub(super) struct ResolvedProfileTypeLaunchOptions {
    pub engine: Engine,
    pub model: String,
    pub reasoning_effort: ReasoningEffort,
}

impl From<ResolvedProfileTypeLaunchOptions> for ResolvedLaunchOptions {
    fn from(options: ResolvedProfileTypeLaunchOptions) -> Self {
        Self {
            engine: options.engine,
            model: Some(options.model),
            reasoning_effort: Some(options.reasoning_effort),
        }
    }
}

pub(super) fn resolve_profile_type_launch_options(
    common: &CommonNewArgs,
    resolved: &ResolvedAgentProfileType,
) -> Result<ResolvedProfileTypeLaunchOptions> {
    if resolved.settings.model.as_deref() == Some("") {
        bail!(
            "agent type `{}` in profile `{}` has an empty model; configure a non-empty model",
            resolved.type_name,
            resolved.profile_name,
        );
    }

    let profile_engine = match resolved.settings.engine {
        AgentProfileTypeEngine::Claude => Engine::Claude,
        AgentProfileTypeEngine::Codex => Engine::Codex,
        AgentProfileTypeEngine::Agy => bail!(
            "agent type `{}` in profile `{}` uses engine `agy`, which cannot be used with `a agent new --type`",
            resolved.type_name,
            resolved.profile_name,
        ),
    };

    if let Some(engine) = common.engine
        && engine != profile_engine
        && common.model.is_none()
    {
        bail!(
            "`--engine {}` differs from type `{}` engine `{}`; specify `--model`",
            engine.process_name(),
            resolved.type_name,
            profile_engine.process_name(),
        );
    }

    Ok(ResolvedProfileTypeLaunchOptions {
        engine: common.engine.unwrap_or(profile_engine),
        model: common
            .model
            .clone()
            .or_else(|| resolved.settings.model.clone())
            .with_context(|| {
                format!(
                    "agent type `{}` in profile `{}` has no model; specify `--model`",
                    resolved.type_name, resolved.profile_name
                )
            })?,
        reasoning_effort: common
            .reasoning_effort
            .or(resolved.settings.reasoning_effort)
            .with_context(|| {
                format!(
                    "agent type `{}` in profile `{}` has no reasoning_effort; specify `--reasoning-effort`",
                    resolved.type_name, resolved.profile_name
                )
            })?,
    })
}

pub(super) fn validate_type_selection(common: &CommonNewArgs, config: &Config) -> Result<()> {
    if common.type_name.is_some() {
        resolve_launch_options(common, config)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::agent::types::{Engine, ReasoningEffort};
    use crate::shared::config::Config;
    use clap::Parser;
    use indoc::indoc;
    use rstest::{fixture, rstest};

    #[derive(Parser)]
    struct TestCli {
        #[command(flatten)]
        common: CommonNewArgs,
    }

    #[fixture]
    fn config() -> Config {
        serde_yaml::from_str(indoc! {
            "
            agent:
              codex:
                model: model-global
                reasoning_effort: max
              launch_profiles:
                review:
                  engine: codex
                  codex:
                    model: model-kind
                    reasoning_effort: low
              active_profile: sample-profile
              profiles:
                sample-profile:
                  review:
                    engine: codex
                    model: model-ember
                    reasoning_effort: high
            "
        })
        .unwrap()
    }

    fn review_type_settings_mut(
        config: &mut Config,
    ) -> &mut crate::shared::config::AgentProfileTypeConfig {
        config
            .agent
            .profiles
            .get_mut("sample-profile")
            .and_then(|profile| profile.get_mut("review"))
            .expect("review profile type should be configured")
    }

    #[rstest]
    #[case::profile_defaults(&["a", "--type", "review"], Engine::Codex, "model-ember", ReasoningEffort::High)]
    #[case::model_override(&["a", "--type", "review", "--model", "model-raven"], Engine::Codex, "model-raven", ReasoningEffort::High)]
    #[case::reasoning_effort_override(&["a", "--type", "review", "--reasoning-effort", "low"], Engine::Codex, "model-ember", ReasoningEffort::Low)]
    #[case::engine_and_model_override(&["a", "--type", "review", "--engine", "claude", "--model", "model-raven"], Engine::Claude, "model-raven", ReasoningEffort::High)]
    #[case::matching_engine_override(&["a", "--type", "review", "--engine", "codex"], Engine::Codex, "model-ember", ReasoningEffort::High)]
    fn resolves_type_flags_and_profile_fallbacks(
        #[case] argv: &[&str],
        #[case] expected_engine: Engine,
        #[case] expected_model: &str,
        #[case] expected_reasoning_effort: ReasoningEffort,
        config: Config,
    ) {
        let cli = TestCli::try_parse_from(argv).unwrap();

        assert_eq!(
            resolve_launch_options(&cli.common, &config).unwrap(),
            ResolvedLaunchOptions {
                engine: expected_engine,
                model: Some(expected_model.to_string()),
                reasoning_effort: Some(expected_reasoning_effort),
            },
        );
    }

    #[rstest]
    #[case::different_engine_without_model(&["a", "--type", "review", "--engine", "claude"], "`--engine claude` differs from type `review` engine `codex`; specify `--model`")]
    fn rejects_type_engine_override_without_model(
        #[case] argv: &[&str],
        #[case] expected_error: &str,
        config: Config,
    ) {
        let cli = TestCli::try_parse_from(argv).unwrap();
        let result =
            resolve_launch_options(&cli.common, &config).map_err(|error| error.to_string());

        assert_eq!(result, Err(expected_error.to_string()));
    }

    #[rstest]
    #[case::missing_model(
        true,
        false,
        "agent type `review` in profile `sample-profile` has no model; specify `--model`"
    )]
    #[case::missing_reasoning_effort(
        false,
        true,
        "agent type `review` in profile `sample-profile` has no reasoning_effort; specify `--reasoning-effort`"
    )]
    fn requires_profile_values_without_global_or_kind_fallbacks(
        #[case] omit_model: bool,
        #[case] omit_reasoning_effort: bool,
        #[case] expected_error: &str,
        mut config: Config,
    ) {
        let settings = review_type_settings_mut(&mut config);
        if omit_model {
            settings.model = None;
        }
        if omit_reasoning_effort {
            settings.reasoning_effort = None;
        }
        let cli = TestCli::try_parse_from(["a", "--kind", "review", "--type", "review"]).unwrap();

        let result =
            resolve_launch_options(&cli.common, &config).map_err(|error| error.to_string());

        assert_eq!(result, Err(expected_error.to_string()));
    }

    #[rstest]
    fn cli_flags_supply_both_omitted_profile_values(mut config: Config) {
        let settings = review_type_settings_mut(&mut config);
        settings.model = None;
        settings.reasoning_effort = None;
        let cli = TestCli::try_parse_from([
            "a",
            "--type",
            "review",
            "--model",
            "model-raven",
            "--reasoning-effort",
            "low",
        ])
        .unwrap();

        assert_eq!(
            resolve_launch_options(&cli.common, &config).unwrap(),
            ResolvedLaunchOptions {
                engine: Engine::Codex,
                model: Some("model-raven".to_string()),
                reasoning_effort: Some(ReasoningEffort::Low),
            },
        );
    }

    #[rstest]
    #[case::profile_defaults(&["a", "--type", "review"])]
    #[case::engine_and_model_overrides(&[
        "a",
        "--type",
        "review",
        "--engine",
        "claude",
        "--model",
        "model-raven",
    ])]
    fn rejects_agy_profile_types_for_session_launch(#[case] argv: &[&str], mut config: Config) {
        let settings = review_type_settings_mut(&mut config);
        settings.engine = AgentProfileTypeEngine::Agy;
        settings.reasoning_effort = None;
        let cli = TestCli::try_parse_from(argv).unwrap();

        let result =
            resolve_launch_options(&cli.common, &config).map_err(|error| error.to_string());

        assert_eq!(
            result,
            Err("agent type `review` in profile `sample-profile` uses engine `agy`, which cannot be used with `a agent new --type`".to_string()),
        );
    }

    #[rstest]
    #[case::empty_model(&["a", "--type", "review"], AgentProfileTypeEngine::Codex, Some(""), None, "agent type `review` in profile `sample-profile` has an empty model; configure a non-empty model")]
    #[case::empty_model_with_cli_override(&["a", "--type", "review", "--model", "model-raven"], AgentProfileTypeEngine::Codex, Some(""), None, "agent type `review` in profile `sample-profile` has an empty model; configure a non-empty model")]
    fn rejects_invalid_profile_type_settings_at_use_time(
        #[case] argv: &[&str],
        #[case] engine: AgentProfileTypeEngine,
        #[case] model: Option<&str>,
        #[case] reasoning_effort: Option<ReasoningEffort>,
        #[case] expected_error: &str,
        mut config: Config,
    ) {
        let settings = review_type_settings_mut(&mut config);
        settings.engine = engine;
        settings.model = model.map(str::to_string);
        settings.reasoning_effort = reasoning_effort;
        let cli = TestCli::try_parse_from(argv).unwrap();

        let result =
            resolve_launch_options(&cli.common, &config).map_err(|error| error.to_string());

        assert_eq!(result, Err(expected_error.to_string()));
    }
}
