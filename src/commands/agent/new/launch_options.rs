use crate::shared::config::{Config, ResolvedLaunchOptions};

use super::CommonNewArgs;

pub(super) fn resolve_launch_options(
    common: &CommonNewArgs,
    config: &Config,
) -> ResolvedLaunchOptions {
    config.agent.resolve_launch_options(
        common.kind.as_deref(),
        common.engine,
        common.model.as_deref(),
        common.reasoning_effort,
    )
}
