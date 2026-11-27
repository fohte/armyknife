use std::fs;
use std::path::Path;

use anyhow::Result;

use super::types::{Engine, HookEvent, HookInput};
use crate::shared::env_var::EnvVars;

pub(super) const TRANSCRIPT_SLIM_MIN_BYTES: u64 = 20 * 1024 * 1024;

const COMPACT_RESUME_ENV: &[(&str, &str)] = &[
    ("ARMYKNIFE_SKIP_HOOKS", "1"),
    ("ARMYKNIFE_SLIM_TRANSCRIPT_ON_COMPACT", "1"),
];

pub(super) fn spawn_compact_resume_with<F>(
    session_id: &str,
    cwd: &Path,
    spawn: F,
) -> std::io::Result<()>
where
    F: FnOnce(&str, &[&str], Option<&Path>, &[(&str, &str)]) -> std::io::Result<()>,
{
    let args = ["-r", session_id, "-p", "/compact"];
    spawn("claude", &args, Some(cwd), COMPACT_RESUME_ENV)
}

pub(super) fn process_skipped_hook_compact_slim<F, S>(
    event: HookEvent,
    engine: Engine,
    env_vars: &EnvVars,
    read_input: F,
    schedule_slim: S,
) -> Result<()>
where
    F: FnOnce() -> Result<HookInput>,
    S: FnMut(&str, &Path),
{
    if !env_vars.skip_hooks
        || !env_vars.slim_transcript_on_compact
        || event != HookEvent::SessionStart
        || engine != Engine::Claude
    {
        return Ok(());
    }

    let mut input = read_input()?;
    input.engine = engine;
    maybe_schedule_transcript_slim(event, &input, schedule_slim);
    Ok(())
}

pub(super) fn maybe_schedule_transcript_slim<S>(
    event: HookEvent,
    input: &HookInput,
    mut schedule_slim: S,
) where
    S: FnMut(&str, &Path),
{
    if event != HookEvent::SessionStart
        || input.engine != Engine::Claude
        || input.source.as_deref() != Some("compact")
    {
        return;
    }
    let Some(transcript_path) = input.transcript_path.as_deref() else {
        return;
    };

    match fs::metadata(transcript_path) {
        Ok(metadata) if metadata.len() > TRANSCRIPT_SLIM_MIN_BYTES => {
            schedule_slim(&input.session_id, transcript_path);
        }
        Ok(_) => {}
        Err(error) => tracing::warn!(
            event = "agent.transcript.slim.inspect_failed",
            session = %input.session_id,
            transcript = %transcript_path.display(),
            error = %error,
        ),
    }
}

#[cfg(test)]
mod tests {
    use std::fs::File;
    use std::path::Path;

    use serde_json::json;
    use tempfile::tempdir;

    use super::{
        TRANSCRIPT_SLIM_MIN_BYTES, process_skipped_hook_compact_slim, spawn_compact_resume_with,
    };
    use crate::commands::agent::types::{Engine, HookEvent};
    use crate::shared::env_var::EnvVars;

    #[test]
    fn idle_compact_spawn_environment_enables_skipped_hook_slimming() {
        let directory = tempdir().expect("fixture directory should be created");
        let transcript_path = directory.path().join("fixture-transcript.jsonl");
        File::create(&transcript_path)
            .expect("transcript fixture should be created")
            .set_len(TRANSCRIPT_SLIM_MIN_BYTES + 1)
            .expect("transcript fixture size should be set");

        let mut spawn_request = None;
        spawn_compact_resume_with(
            "fixture-session",
            directory.path(),
            |program, args, cwd, env| {
                spawn_request = Some((
                    program.to_string(),
                    args.iter().map(|arg| arg.to_string()).collect::<Vec<_>>(),
                    cwd.map(Path::to_path_buf),
                    env.iter()
                        .map(|(name, value)| (name.to_string(), value.to_string()))
                        .collect::<Vec<_>>(),
                ));
                Ok(())
            },
        )
        .expect("injected compact spawn should succeed");

        let spawn_request = spawn_request.expect("spawn request should be captured");
        let env_vars = spawn_request
            .3
            .iter()
            .map(|(name, value)| (name.as_str(), Some(value.as_str())))
            .collect::<Vec<_>>();
        let (skip_hooks, slim_transcript_on_compact, scheduled_calls) =
            temp_env::with_vars(&env_vars, || {
                let env_vars = EnvVars::load();
                let input = serde_json::from_value(json!({
                    "session_id": "fixture-session",
                    "cwd": directory.path().to_string_lossy(),
                    "transcript_path": transcript_path.to_string_lossy(),
                    "source": "compact",
                }))
                .expect("compact hook input should deserialize");
                let mut scheduled_calls = Vec::new();

                process_skipped_hook_compact_slim(
                    HookEvent::SessionStart,
                    Engine::Claude,
                    &env_vars,
                    || Ok(input),
                    |session_id, path| {
                        scheduled_calls.push((session_id.to_string(), path.to_path_buf()));
                    },
                )
                .expect("skipped compact hook should succeed");

                (
                    env_vars.skip_hooks,
                    env_vars.slim_transcript_on_compact,
                    scheduled_calls,
                )
            });

        assert_eq!(
            (
                spawn_request,
                (skip_hooks, slim_transcript_on_compact, scheduled_calls),
            ),
            (
                (
                    "claude".to_string(),
                    vec![
                        "-r".to_string(),
                        "fixture-session".to_string(),
                        "-p".to_string(),
                        "/compact".to_string(),
                    ],
                    Some(directory.path().to_path_buf()),
                    vec![
                        ("ARMYKNIFE_SKIP_HOOKS".to_string(), "1".to_string()),
                        (
                            "ARMYKNIFE_SLIM_TRANSCRIPT_ON_COMPACT".to_string(),
                            "1".to_string(),
                        ),
                    ],
                ),
                (
                    true,
                    true,
                    vec![("fixture-session".to_string(), transcript_path)],
                ),
            ),
        );
    }
}
