use anyhow::{Context, Result};

use crate::shared::config::{ResolvedAgentProfileType, ResolvedLaunchOptions, load_config};

use super::{CommonNewArgs, NewArgs};

pub(super) fn dispatch(
    args: &NewArgs,
    dry_run: impl FnOnce(&CommonNewArgs) -> Result<()>,
    regular: impl FnOnce(&NewArgs) -> Result<()>,
) -> Result<()> {
    if args.dry_run {
        dry_run(&args.common)
    } else {
        regular(args)
    }
}

pub(super) fn run_dry_run(common: &CommonNewArgs) -> Result<()> {
    let type_name = common
        .type_name
        .as_deref()
        .context("--dry-run requires --type")?;
    let config = load_config()?;
    let resolved = config.agent.resolve_profile_type(type_name)?;
    let launch_options =
        super::launch_options::resolve_profile_type_launch_options(common, &resolved)?;
    print!("{}", format_dry_run(common, &resolved, &launch_options));
    Ok(())
}

pub(super) fn format_dry_run(
    common: &CommonNewArgs,
    resolved: &ResolvedAgentProfileType,
    launch_options: &ResolvedLaunchOptions,
) -> String {
    let source = format!(
        "agent.profiles[{:?}][{:?}]",
        resolved.profile_name, resolved.type_name
    );
    let engine_source = if common.engine.is_some() {
        "--engine".to_string()
    } else {
        format!("{source}.engine")
    };
    let model_source = if common.model.is_some() {
        "--model".to_string()
    } else {
        format!("{source}.model")
    };
    let reasoning_effort_source = if common.reasoning_effort.is_some() {
        "--reasoning-effort".to_string()
    } else {
        format!("{source}.reasoning_effort")
    };
    format!(
        "Resolved agent launch settings:\n  profile: {}\n  type: {}\n  engine: {}\n    source: {engine_source}\n  model: {}\n    source: {model_source}\n  reasoning effort: {}\n    source: {reasoning_effort_source}\n",
        resolved.profile_name,
        resolved.type_name,
        launch_options.engine.process_name(),
        launch_options.model.as_deref().unwrap_or("<none>"),
        launch_options
            .reasoning_effort
            .map(|effort| effort.as_str())
            .unwrap_or("<none>"),
    )
}
