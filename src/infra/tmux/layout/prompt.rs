//! Rewrites an agent pane command so it starts with the session prompt (and
//! model) supplied to `a agent new`.

use std::path::Path;

use crate::commands::agent::types::{Engine, ReasoningEffort};

/// Whether a pane command starts `engine`'s CLI. Both `claude` and `codex`
/// take the session prompt as a trailing positional argument and
/// `--model <model>`, so `apply_prompt_if_agent` builds them the same way.
pub(super) fn is_engine_command(command: &str, engine: Engine) -> bool {
    command.starts_with(engine.process_name())
}

/// If the command starts `engine`'s CLI, insert `--model <model>` (and, for
/// codex, `-c model_reasoning_effort=<effort>`) right after the program name
/// and append the prompt file path. Any other command is returned untouched,
/// including another engine's CLI: `model` and `reasoning_effort` are only
/// meaningful to the engine the session was started for.
///
/// Uses `$(cat <path>)` to read the prompt at shell execution time.
/// If `cleanup` is true, also deletes the temp file after reading.
/// Only the last `engine` pane should set `cleanup = true` to avoid
/// deleting the file before other panes have read it.
pub(super) fn apply_prompt_if_agent(
    command: &str,
    engine: Engine,
    model: Option<&str>,
    reasoning_effort: Option<ReasoningEffort>,
    prompt_file: Option<&Path>,
    cleanup: bool,
) -> String {
    if !is_engine_command(command, engine) {
        return command.to_string();
    }
    let program = engine.process_name();

    let mut flags = String::new();
    if let Some(model) = model {
        let escaped_model = shlex::try_quote(model)
            .map(|c| c.into_owned())
            .unwrap_or_else(|_| model.to_string());
        flags.push_str(&format!(" --model {escaped_model}"));
    }
    // `-c` overrides config.toml for this launch only, unlike editing
    // `~/.codex/config.toml`, which a hand-run `codex` would also pick up.
    if let (Engine::Codex, Some(effort)) = (engine, reasoning_effort) {
        flags.push_str(&format!(" -c model_reasoning_effort={}", effort.as_str()));
    }

    // Restrict to the exact program name (optionally followed by a
    // space-separated rest) so a differently-named pane command that merely
    // starts with it (e.g. a "claude-code" wrapper script) isn't mangled by
    // splicing flags into the middle of its name.
    let command = match command.strip_prefix(program) {
        Some(rest) if rest.is_empty() || rest.starts_with(' ') => {
            format!("{program}{flags}{rest}")
        }
        _ => command.to_string(),
    };

    match prompt_file {
        Some(path) => {
            let path_str = path.display().to_string();
            let escaped_path = shlex::try_quote(&path_str)
                .map(|c| c.into_owned())
                .unwrap_or(path_str);
            if cleanup {
                format!("{command} \"$(cat {escaped_path})\" ; rm {escaped_path}")
            } else {
                format!("{command} \"$(cat {escaped_path})\"")
            }
        }
        None => command,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;
    use std::path::PathBuf;

    #[rstest]
    #[case::claude_without_prompt(Engine::Claude, "claude", None)]
    #[case::codex_without_prompt(Engine::Codex, "codex", None)]
    #[case::non_agent_with_prompt(Engine::Claude, "nvim", Some("/tmp/prompt.txt"))]
    #[case::non_agent_without_prompt(Engine::Claude, "bash", None)]
    #[case::other_engine_with_prompt(Engine::Claude, "codex", Some("/tmp/prompt.txt"))]
    #[case::other_engine_with_prompt_reversed(Engine::Codex, "claude", Some("/tmp/prompt.txt"))]
    fn test_apply_prompt_if_agent_no_expansion(
        #[case] engine: Engine,
        #[case] command: &str,
        #[case] path: Option<&str>,
    ) {
        let path_buf = path.map(PathBuf::from);
        let result = apply_prompt_if_agent(command, engine, None, None, path_buf.as_deref(), true);
        assert_eq!(result, command);
    }

    #[rstest]
    #[case::claude_with_cleanup(
        Engine::Claude,
        "claude",
        "/tmp/prompt.txt",
        true,
        "claude \"$(cat /tmp/prompt.txt)\" ; rm /tmp/prompt.txt"
    )]
    #[case::claude_without_cleanup(
        Engine::Claude,
        "claude",
        "/tmp/prompt.txt",
        false,
        "claude \"$(cat /tmp/prompt.txt)\""
    )]
    #[case::codex_with_cleanup(
        Engine::Codex,
        "codex",
        "/tmp/prompt.txt",
        true,
        "codex \"$(cat /tmp/prompt.txt)\" ; rm /tmp/prompt.txt"
    )]
    #[case::codex_without_cleanup(
        Engine::Codex,
        "codex",
        "/tmp/prompt.txt",
        false,
        "codex \"$(cat /tmp/prompt.txt)\""
    )]
    #[case::claude_wrapper_still_gets_prompt(
        Engine::Claude,
        "claude-code",
        "/tmp/prompt.txt",
        true,
        "claude-code \"$(cat /tmp/prompt.txt)\" ; rm /tmp/prompt.txt"
    )]
    #[case::claude_code_with_cleanup(
        Engine::Claude,
        "claude code",
        "/tmp/prompt.txt",
        true,
        "claude code \"$(cat /tmp/prompt.txt)\" ; rm /tmp/prompt.txt"
    )]
    fn test_apply_prompt_if_agent_with_file(
        #[case] engine: Engine,
        #[case] command: &str,
        #[case] path: &str,
        #[case] cleanup: bool,
        #[case] expected: &str,
    ) {
        let path_buf = PathBuf::from(path);
        let result = apply_prompt_if_agent(command, engine, None, None, Some(&path_buf), cleanup);
        assert_eq!(result, expected);
    }

    #[rstest]
    #[case::claude_with_model_no_prompt(
        Engine::Claude,
        "claude",
        Some("opus"),
        None,
        None,
        "claude --model opus"
    )]
    #[case::claude_with_extra_args_and_model(
        Engine::Claude,
        "claude -p agent1",
        Some("opus"),
        None,
        None,
        "claude --model opus -p agent1"
    )]
    #[case::claude_without_model_unchanged(Engine::Claude, "claude", None, None, None, "claude")]
    #[case::non_agent_with_model_unchanged(
        Engine::Claude,
        "nvim",
        Some("opus"),
        None,
        None,
        "nvim"
    )]
    #[case::claude_prefixed_other_command_unchanged(
        Engine::Claude,
        "claude-code",
        Some("opus"),
        None,
        None,
        "claude-code"
    )]
    #[case::claude_with_model_and_prompt(
        Engine::Claude,
        "claude",
        Some("opus"),
        None,
        Some("/tmp/prompt.txt"),
        "claude --model opus \"$(cat /tmp/prompt.txt)\" ; rm /tmp/prompt.txt"
    )]
    #[case::claude_ignores_effort(
        Engine::Claude,
        "claude",
        None,
        Some(ReasoningEffort::Max),
        None,
        "claude"
    )]
    #[case::codex_with_model_no_prompt(
        Engine::Codex,
        "codex",
        Some("gpt-5"),
        None,
        None,
        "codex --model gpt-5"
    )]
    #[case::codex_with_extra_args_and_model(
        Engine::Codex,
        "codex --search",
        Some("gpt-5"),
        None,
        None,
        "codex --model gpt-5 --search"
    )]
    #[case::codex_prefixed_other_command_unchanged(
        Engine::Codex,
        "codex-wrapper",
        Some("gpt-5"),
        Some(ReasoningEffort::Max),
        None,
        "codex-wrapper"
    )]
    #[case::codex_with_model_and_prompt(
        Engine::Codex,
        "codex",
        Some("gpt-5"),
        None,
        Some("/tmp/prompt.txt"),
        "codex --model gpt-5 \"$(cat /tmp/prompt.txt)\" ; rm /tmp/prompt.txt"
    )]
    #[case::codex_effort_only(
        Engine::Codex,
        "codex",
        None,
        Some(ReasoningEffort::Max),
        None,
        "codex -c model_reasoning_effort=max"
    )]
    #[case::codex_effort_keeps_extra_args(
        Engine::Codex,
        "codex --search",
        None,
        Some(ReasoningEffort::XHigh),
        None,
        "codex -c model_reasoning_effort=xhigh --search"
    )]
    #[case::codex_model_and_effort_with_prompt(
        Engine::Codex,
        "codex",
        Some("gpt-5.6-luna"),
        Some(ReasoningEffort::Max),
        Some("/tmp/prompt.txt"),
        "codex --model gpt-5.6-luna -c model_reasoning_effort=max \"$(cat /tmp/prompt.txt)\" ; rm /tmp/prompt.txt"
    )]
    #[case::other_engine_pane_keeps_model_effort_and_prompt_off(
        Engine::Claude,
        "codex",
        Some("opus"),
        Some(ReasoningEffort::Max),
        Some("/tmp/prompt.txt"),
        "codex"
    )]
    fn test_apply_prompt_if_agent_with_launch_flags(
        #[case] engine: Engine,
        #[case] command: &str,
        #[case] model: Option<&str>,
        #[case] effort: Option<ReasoningEffort>,
        #[case] path: Option<&str>,
        #[case] expected: &str,
    ) {
        let path_buf = path.map(PathBuf::from);
        let result =
            apply_prompt_if_agent(command, engine, model, effort, path_buf.as_deref(), true);
        assert_eq!(result, expected);
    }
}
