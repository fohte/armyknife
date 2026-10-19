use anyhow::{Context, Result};

use crate::shared::config::{ResolvedAgentProfileType, load_config};

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
    print!("{}", format_dry_run(&resolved));
    Ok(())
}

pub(super) fn format_dry_run(resolved: &ResolvedAgentProfileType) -> String {
    let source = format!(
        "agent.profiles[{:?}][{:?}]",
        resolved.profile_name, resolved.type_name
    );
    format!(
        "Resolved agent launch settings:\n  profile: {}\n  type: {}\n  engine: {}\n    source: {source}.engine\n  model: {}\n    source: {source}.model\n  reasoning effort: {}\n    source: {source}.reasoning_effort\n",
        resolved.profile_name,
        resolved.type_name,
        resolved.settings.engine.process_name(),
        resolved.settings.model,
        resolved.settings.reasoning_effort.as_str(),
    )
}
