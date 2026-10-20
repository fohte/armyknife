use anyhow::{Result, bail};

use crate::commands::agent::types::{Engine, ReasoningEffort};
use crate::shared::config::{Config, ResolvedAgentProfileType, ResolvedLaunchOptions};

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
    if let Some(engine) = common.engine
        && engine != resolved.settings.engine
        && common.model.is_none()
    {
        bail!(
            "`--engine {}` differs from type `{}` engine `{}`; specify `--model`",
            engine.process_name(),
            resolved.type_name,
            resolved.settings.engine.process_name(),
        );
    }

    Ok(ResolvedProfileTypeLaunchOptions {
        engine: common.engine.unwrap_or(resolved.settings.engine),
        model: common
            .model
            .clone()
            .unwrap_or_else(|| resolved.settings.model.clone()),
        reasoning_effort: common
            .reasoning_effort
            .unwrap_or(resolved.settings.reasoning_effort),
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
}
