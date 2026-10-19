use anyhow::Result;

use crate::shared::config::{Config, ResolvedLaunchOptions};

use super::CommonNewArgs;

pub(super) fn resolve_launch_options(
    common: &CommonNewArgs,
    config: &Config,
) -> Result<ResolvedLaunchOptions> {
    if let Some(type_name) = common.type_name.as_deref() {
        let resolved = config.agent.resolve_profile_type(type_name)?;
        return Ok(ResolvedLaunchOptions {
            engine: resolved.settings.engine,
            model: Some(resolved.settings.model),
            reasoning_effort: Some(resolved.settings.reasoning_effort),
        });
    }

    Ok(config.agent.resolve_launch_options(
        common.kind.as_deref(),
        common.engine,
        common.model.as_deref(),
        common.reasoning_effort,
    ))
}

pub(super) fn validate_type_selection(common: &CommonNewArgs, config: &Config) -> Result<()> {
    if common.type_name.is_some() {
        resolve_launch_options(common, config)?;
    }
    Ok(())
}
