//! Rewrites an agent pane command so it starts with the session prompt (and
//! model) supplied to `a agent new`.

use std::path::Path;

use clap::ValueEnum;

use crate::commands::agent::types::Engine;

/// The agent CLI a pane command starts, or `None` for any other command
/// (which is then left untouched). Every engine's CLI takes the session
/// prompt as a trailing positional argument and `--model <model>`, so
/// `apply_prompt_if_agent` builds all of them the same way.
pub(super) fn detect_engine(command: &str) -> Option<Engine> {
    Engine::value_variants()
        .iter()
        .copied()
        .find(|engine| command.starts_with(engine.process_name()))
}

/// If the command starts an agent CLI (see `detect_engine`), insert
/// `--model <model>` right after the program name and append the prompt file
/// path.
///
/// Uses `$(cat <path>)` to read the prompt at shell execution time.
/// If `cleanup` is true, also deletes the temp file after reading.
/// Only the last agent pane should set `cleanup = true` to avoid
/// deleting the file before other panes have read it.
pub(super) fn apply_prompt_if_agent(
    command: &str,
    model: Option<&str>,
    prompt_file: Option<&Path>,
    cleanup: bool,
) -> String {
    let Some(engine) = detect_engine(command) else {
        return command.to_string();
    };
    let program = engine.process_name();

    // Restrict to the exact program name (optionally followed by a
    // space-separated rest) so a differently-named pane command that merely
    // starts with it (e.g. a "claude-code" wrapper script) isn't mangled by
    // splicing --model into the middle of its name.
    let command = match (model, command.strip_prefix(program)) {
        (Some(model), Some(rest)) if rest.is_empty() || rest.starts_with(' ') => {
            let escaped_model = shlex::try_quote(model)
                .map(|c| c.into_owned())
                .unwrap_or_else(|_| model.to_string());
            format!("{program} --model {escaped_model}{rest}")
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
    #[case::claude_without_prompt("claude", None)]
    #[case::codex_without_prompt("codex", None)]
    #[case::non_claude_with_prompt("nvim", Some("/tmp/prompt.txt"))]
    #[case::non_claude_without_prompt("bash", None)]
    fn test_apply_prompt_if_agent_no_expansion(#[case] command: &str, #[case] path: Option<&str>) {
        let path_buf = path.map(PathBuf::from);
        let result = apply_prompt_if_agent(command, None, path_buf.as_deref(), true);
        assert_eq!(result, command);
    }

    #[rstest]
    #[case::claude_with_cleanup(
        "claude",
        "/tmp/prompt.txt",
        true,
        "claude \"$(cat /tmp/prompt.txt)\" ; rm /tmp/prompt.txt"
    )]
    #[case::claude_without_cleanup(
        "claude",
        "/tmp/prompt.txt",
        false,
        "claude \"$(cat /tmp/prompt.txt)\""
    )]
    #[case::codex_with_cleanup(
        "codex",
        "/tmp/prompt.txt",
        true,
        "codex \"$(cat /tmp/prompt.txt)\" ; rm /tmp/prompt.txt"
    )]
    #[case::codex_without_cleanup(
        "codex",
        "/tmp/prompt.txt",
        false,
        "codex \"$(cat /tmp/prompt.txt)\""
    )]
    #[case::claude_wrapper_still_gets_prompt(
        "claude-code",
        "/tmp/prompt.txt",
        true,
        "claude-code \"$(cat /tmp/prompt.txt)\" ; rm /tmp/prompt.txt"
    )]
    #[case::claude_code_with_cleanup(
        "claude code",
        "/tmp/prompt.txt",
        true,
        "claude code \"$(cat /tmp/prompt.txt)\" ; rm /tmp/prompt.txt"
    )]
    fn test_apply_prompt_if_agent_with_file(
        #[case] command: &str,
        #[case] path: &str,
        #[case] cleanup: bool,
        #[case] expected: &str,
    ) {
        let path_buf = PathBuf::from(path);
        let result = apply_prompt_if_agent(command, None, Some(&path_buf), cleanup);
        assert_eq!(result, expected);
    }

    #[rstest]
    #[case::claude_with_model_no_prompt("claude", Some("opus"), None, "claude --model opus")]
    #[case::claude_with_extra_args_and_model(
        "claude -p agent1",
        Some("opus"),
        None,
        "claude --model opus -p agent1"
    )]
    #[case::claude_without_model_unchanged("claude", None, None, "claude")]
    #[case::non_claude_with_model_unchanged("nvim", Some("opus"), None, "nvim")]
    #[case::claude_prefixed_other_command_unchanged(
        "claude-code",
        Some("opus"),
        None,
        "claude-code"
    )]
    #[case::codex_with_model_no_prompt("codex", Some("gpt-5"), None, "codex --model gpt-5")]
    #[case::codex_with_extra_args_and_model(
        "codex --search",
        Some("gpt-5"),
        None,
        "codex --model gpt-5 --search"
    )]
    #[case::codex_prefixed_other_command_unchanged(
        "codex-wrapper",
        Some("gpt-5"),
        None,
        "codex-wrapper"
    )]
    #[case::codex_with_model_and_prompt(
        "codex",
        Some("gpt-5"),
        Some("/tmp/prompt.txt"),
        "codex --model gpt-5 \"$(cat /tmp/prompt.txt)\" ; rm /tmp/prompt.txt"
    )]
    #[case::claude_with_model_and_prompt(
        "claude",
        Some("opus"),
        Some("/tmp/prompt.txt"),
        "claude --model opus \"$(cat /tmp/prompt.txt)\" ; rm /tmp/prompt.txt"
    )]
    fn test_apply_prompt_if_agent_with_model(
        #[case] command: &str,
        #[case] model: Option<&str>,
        #[case] path: Option<&str>,
        #[case] expected: &str,
    ) {
        let path_buf = path.map(PathBuf::from);
        let result = apply_prompt_if_agent(command, model, path_buf.as_deref(), true);
        assert_eq!(result, expected);
    }
}
