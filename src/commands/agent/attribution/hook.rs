use std::path::Path;

use anyhow::{Context, Result};
use serde_json::{Value, json};

use super::command::{classify_command, inject_context};
use super::context::{self, OperationContext};
use super::transcript;
use super::{Provider, ProxyEnvelope, model_identity};

pub(super) fn transform(
    provider: Provider,
    raw_envelope: &str,
    context_dir: &Path,
) -> Result<Option<Value>> {
    let envelope: ProxyEnvelope = serde_json::from_str(raw_envelope)
        .context("provider proxy input must be a JSON object with hook_input and hook_output")?;
    let input = &envelope.hook_input;
    if input.get("hook_event_name").and_then(Value::as_str) != Some("PreToolUse") {
        return Ok(envelope.hook_output);
    }
    if decision(&envelope.hook_output) == Some("deny") {
        return Ok(envelope.hook_output);
    }
    let Some(tool_name) = input.get("tool_name").and_then(Value::as_str) else {
        return Ok(skip_attribution(
            &envelope.hook_output,
            "provider hook input has no tool name",
        ));
    };
    if !is_shell_tool(tool_name) {
        return Ok(envelope.hook_output);
    }
    let Some(original_command) = input
        .get("tool_input")
        .and_then(Value::as_object)
        .and_then(|tool_input| tool_input.get("command"))
        .and_then(Value::as_str)
    else {
        return Ok(skip_attribution(
            &envelope.hook_output,
            "provider hook input has no command",
        ));
    };
    let final_command = envelope
        .hook_output
        .as_ref()
        .and_then(|output| output.pointer("/hookSpecificOutput/updatedInput/command"))
        .and_then(Value::as_str)
        .unwrap_or(original_command)
        .to_owned();
    let operation = match classify_command(original_command) {
        Ok(Some(operation)) => operation,
        Ok(None) => return Ok(envelope.hook_output),
        Err(error) => return Ok(skip_attribution(&envelope.hook_output, error)),
    };
    let final_operation = match classify_command(&final_command) {
        Ok(operation) => operation,
        Err(error) => return Ok(skip_attribution(&envelope.hook_output, error)),
    };
    if final_operation != Some(operation) {
        return Ok(skip_attribution(
            &envelope.hook_output,
            "permission proxy changed the commit operation",
        ));
    }
    if let Err(error) = inject_context(&final_command, Path::new("/context-placeholder")) {
        return Ok(skip_attribution(&envelope.hook_output, error));
    }
    if let Err(error) = patch_hook_output(
        provider,
        input,
        envelope.hook_output.clone(),
        "context-placeholder",
    ) {
        return Ok(skip_attribution(&envelope.hook_output, error));
    }

    let Some(tool_use_id) = input
        .get("tool_use_id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
    else {
        return Ok(skip_attribution(
            &envelope.hook_output,
            "provider hook input has no tool-use identifier",
        ));
    };
    let model = match provider {
        Provider::Codex => input
            .get("model")
            .and_then(Value::as_str)
            .filter(|slug| !slug.is_empty())
            .map(str::to_owned),
        Provider::ClaudeCode => {
            let transcript_path = input
                .get("transcript_path")
                .and_then(Value::as_str)
                .map(Path::new);
            transcript_path.and_then(|path| {
                transcript::model_for_tool_use(path, tool_use_id, tool_name, original_command).ok()
            })
        }
    };
    let model = match model_identity(model)
        .map_err(|error| anyhow::anyhow!("runtime model is unavailable: {error}"))
    {
        Ok(model) => model,
        Err(error) => return Ok(skip_attribution(&envelope.hook_output, error)),
    };
    let context = match OperationContext::new(provider, model, tool_use_id.to_owned()) {
        Ok(context) => context,
        Err(error) => return Ok(skip_attribution(&envelope.hook_output, error)),
    };
    let context_path = match context::create(&context, context_dir) {
        Ok(path) => path,
        Err(error) => return Ok(skip_attribution(&envelope.hook_output, error)),
    };
    let transformed = (|| {
        let command = inject_context(&final_command, &context_path)?;
        patch_hook_output(provider, input, envelope.hook_output.clone(), &command)
    })();
    match transformed {
        Ok(output) => Ok(Some(output)),
        Err(error) => {
            if let Err(discard_error) = context::discard(&context_path) {
                eprintln!("failed to discard skipped attribution context: {discard_error:#}");
            }
            Ok(skip_attribution(&envelope.hook_output, error))
        }
    }
}

fn skip_attribution(output: &Option<Value>, reason: impl std::fmt::Display) -> Option<Value> {
    eprintln!("agent attribution skipped: {reason}");
    output.clone()
}

fn is_shell_tool(tool_name: &str) -> bool {
    tool_name == "Bash"
}

fn decision(output: &Option<Value>) -> Option<&str> {
    output
        .as_ref()?
        .pointer("/hookSpecificOutput/permissionDecision")?
        .as_str()
}

fn patch_hook_output(
    provider: Provider,
    hook_input: &Value,
    output: Option<Value>,
    command: &str,
) -> Result<Value> {
    let mut output = output.unwrap_or_else(|| json!({}));
    let root = output
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("permission proxy output must be a JSON object"))?;
    let specific = root
        .entry("hookSpecificOutput")
        .or_insert_with(|| json!({"hookEventName": "PreToolUse"}))
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("permission proxy hookSpecificOutput must be an object"))?;
    if specific.get("hookEventName").and_then(Value::as_str) != Some("PreToolUse") {
        anyhow::bail!("permission proxy output is not for PreToolUse");
    }
    match (
        provider,
        specific.get("permissionDecision").and_then(Value::as_str),
    ) {
        (_, Some("deny")) => return Ok(output),
        (Provider::Codex, Some("ask")) => {
            anyhow::bail!(
                "Codex ask decisions must stay inside the existing runok exec --ask command wrapper"
            );
        }
        (Provider::Codex, Some("allow")) | (Provider::ClaudeCode, Some("allow" | "ask")) => {}
        (Provider::Codex, None) => {
            // Codex requires allow to accept updatedInput.
            specific.insert("permissionDecision".to_owned(), json!("allow"));
        }
        (Provider::ClaudeCode, None) => {}
        (_, Some(other)) => anyhow::bail!("unsupported provider permission decision: {other}"),
    }

    let mut updated_input = specific
        .get("updatedInput")
        .filter(|input| input.is_object())
        .cloned()
        .unwrap_or_else(|| {
            hook_input
                .get("tool_input")
                .cloned()
                .unwrap_or_else(|| json!({}))
        });
    let updated = updated_input
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("provider tool input must be a JSON object"))?;
    updated.insert("command".to_owned(), json!(command));
    specific.insert("updatedInput".to_owned(), updated_input);
    Ok(output)
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::path::{Path, PathBuf};

    use super::super::CONTEXT_ENV;
    use super::super::Provider;
    use super::transform;
    use crate::commands::agent::attribution::context;
    use rstest::{fixture, rstest};
    use serde_json::{Value, json};
    use tempfile::TempDir;

    const COMMIT_COMMAND: &str = "git commit -m 'Fixture subject'";

    #[fixture]
    fn context_dir() -> TempDir {
        tempfile::tempdir().expect("create test context directory")
    }

    fn envelope(input: Value, output: Option<Value>) -> String {
        json!({"hook_input": input, "hook_output": output}).to_string()
    }

    fn codex_input(command: &str, model: Option<&str>, tool_use_id: &str) -> Value {
        let mut input = json!({
            "hook_event_name": "PreToolUse",
            "tool_name": "Bash",
            "tool_use_id": tool_use_id,
            "tool_input": {"command": command},
        });
        if let Some(model) = model {
            input["model"] = json!(model);
        }
        input
    }

    fn context_path(output: &Value) -> PathBuf {
        let command = output
            .pointer("/hookSpecificOutput/updatedInput/command")
            .and_then(Value::as_str)
            .expect("transformed command");
        let words = shlex::split(command).expect("outer shell command parses");
        let words = if words.first().is_some_and(|word| word == "runok") {
            let delimiter = words
                .iter()
                .position(|word| word == "--")
                .expect("runok command delimiter");
            shlex::split(&words[delimiter + 1]).expect("wrapped shell command parses")
        } else {
            words
        };
        let assignment = words
            .first()
            .and_then(|word| word.strip_prefix(&format!("{CONTEXT_ENV}=")))
            .expect("context assignment");
        PathBuf::from(assignment)
    }

    fn normalize_output(output: Value, path: &Path) -> Value {
        let text = output.to_string().replace(
            path.to_str().expect("context path is UTF-8"),
            "<context-file>",
        );
        serde_json::from_str(&text).expect("normalized output remains valid JSON")
    }

    #[rstest]
    fn codex_commit_receives_runtime_model_context(context_dir: TempDir) {
        let output = transform(
            Provider::Codex,
            &envelope(
                codex_input(
                    COMMIT_COMMAND,
                    Some("fixture-model-blue-2026"),
                    "fixture-tool-use-blue",
                ),
                None,
            ),
            context_dir.path(),
        )
        .expect("transform provider input")
        .expect("Codex receives updated input");
        let path = context_path(&output);
        let record = context::read(&path).expect("read operation context");
        let actual = (
            record.model().raw_slug(),
            record.coauthor_trailer(),
            normalize_output(output, &path),
        );

        assert_eq!(
            actual,
            (
                Some("fixture-model-blue-2026"),
                Some(
                    "Co-authored-by: Codex (fixture-model-blue-2026) <noreply@openai.com>"
                        .to_owned(),
                ),
                json!({
                    "hookSpecificOutput": {
                        "hookEventName": "PreToolUse",
                        "permissionDecision": "allow",
                        "updatedInput": {
                            "command": format!("{CONTEXT_ENV}=<context-file> {COMMIT_COMMAND}"),
                        },
                    },
                }),
            ),
        );
    }

    #[rstest]
    fn codex_keeps_the_existing_runok_ask_wrapper(context_dir: TempDir) {
        let command =
            r#"runok exec --ask --hook-origin fixture-token -- 'git commit -m "Fixture subject"'"#;
        let proxy_output = json!({
            "hookSpecificOutput": {
                "hookEventName": "PreToolUse",
                "permissionDecision": "allow",
                "updatedInput": {"command": command},
            },
        });
        let output = transform(
            Provider::Codex,
            &envelope(
                codex_input(
                    COMMIT_COMMAND,
                    Some("fixture-model-blue"),
                    "fixture-tool-use-ask",
                ),
                Some(proxy_output),
            ),
            context_dir.path(),
        )
        .expect("transform runok input")
        .expect("Codex receives updated input");
        let path = context_path(&output);

        assert_eq!(
            normalize_output(output, &path),
            json!({
                "hookSpecificOutput": {
                    "hookEventName": "PreToolUse",
                    "permissionDecision": "allow",
                    "updatedInput": {
                        "command": r#"runok exec --ask --hook-origin fixture-token -- 'ARMYKNIFE_AGENT_ATTRIBUTION_CONTEXT=<context-file> git commit -m "Fixture subject"'"#,
                    },
                },
            }),
        );
    }

    #[rstest]
    fn claude_uses_the_model_for_the_matching_tool_use(context_dir: TempDir) {
        let transcript_path = context_dir.path().join("fixture-transcript.jsonl");
        let transcript_line = json!({
            "type": "assistant",
            "message": {
                "model": "fixture-claude-model-2026",
                "content": [{
                    "type": "tool_use",
                    "id": "fixture-tool-use-claude",
                    "name": "Bash",
                    "input": {"command": COMMIT_COMMAND},
                }],
            },
        })
        .to_string();
        let mut transcript_file =
            std::fs::File::create(&transcript_path).expect("create fixture transcript");
        writeln!(transcript_file, "{transcript_line}").expect("write fixture transcript");
        let input = json!({
            "hook_event_name": "PreToolUse",
            "tool_name": "Bash",
            "tool_use_id": "fixture-tool-use-claude",
            "transcript_path": transcript_path,
            "tool_input": {"command": COMMIT_COMMAND},
        });
        let proxy_output = json!({
            "hookSpecificOutput": {
                "hookEventName": "PreToolUse",
                "permissionDecision": "ask",
                "permissionDecisionReason": "fixture rule",
            },
        });
        let output = transform(
            Provider::ClaudeCode,
            &envelope(input, Some(proxy_output)),
            context_dir.path(),
        )
        .expect("transform Claude input")
        .expect("Claude receives updated input");
        let path = context_path(&output);
        let record = context::read(&path).expect("read operation context");
        let actual = (
            record.model().raw_slug(),
            record.coauthor_trailer(),
            normalize_output(output, &path),
        );

        assert_eq!(
            actual,
            (
                Some("fixture-claude-model-2026"),
                Some(
                    "Co-authored-by: Claude Code (fixture-claude-model-2026) <noreply@anthropic.com>"
                        .to_owned(),
                ),
                json!({
                    "hookSpecificOutput": {
                        "hookEventName": "PreToolUse",
                        "permissionDecision": "ask",
                        "permissionDecisionReason": "fixture rule",
                        "updatedInput": {
                            "command": format!("{CONTEXT_ENV}=<context-file> {COMMIT_COMMAND}"),
                        },
                    },
                }),
            ),
        );
    }

    #[rstest]
    fn deny_decision_is_preserved_without_creating_context(context_dir: TempDir) {
        let command =
            r#"runok exec --ask --hook-origin fixture-token -- 'git commit -m "Fixture subject"'"#;
        let proxy_output = json!({
            "hookSpecificOutput": {
                "hookEventName": "PreToolUse",
                "permissionDecision": "deny",
                "permissionDecisionReason": "fixture denied rule",
                "updatedInput": {"command": command},
            },
        });
        let result = transform(
            Provider::Codex,
            &envelope(
                codex_input(
                    command,
                    Some("fixture-model-blue"),
                    "fixture-tool-use-denied",
                ),
                Some(proxy_output.clone()),
            ),
            context_dir.path(),
        )
        .expect("preserve denied request");
        let context_count = std::fs::read_dir(context_dir.path())
            .expect("read context directory")
            .count();

        assert_eq!((result, context_count), (Some(proxy_output), 0));
    }

    #[rstest]
    #[case::missing_model(None)]
    #[case::trailer_delimiter(Some("fixture-model <fixture@example.invalid>"))]
    fn unusable_model_preserves_permission_proxy_output(
        context_dir: TempDir,
        #[case] model: Option<&str>,
    ) {
        let proxy_output = json!({
            "hookSpecificOutput": {
                "hookEventName": "PreToolUse",
                "permissionDecision": "allow",
            },
        });
        let result = transform(
            Provider::Codex,
            &envelope(
                codex_input(COMMIT_COMMAND, model, "fixture-tool-use-missing-model"),
                Some(proxy_output.clone()),
            ),
            context_dir.path(),
        )
        .expect("unavailable model skips attribution");
        let context_count = std::fs::read_dir(context_dir.path())
            .expect("read context directory")
            .count();

        assert_eq!((result, context_count), (Some(proxy_output), 0));
    }

    #[rstest]
    #[case::status("git status --short")]
    #[case::pull("git pull origin fixture-branch")]
    #[case::merge("git merge fixture-branch")]
    #[case::rebase("git rebase fixture-base")]
    #[case::unknown_alias("git ci -m 'Fixture subject'")]
    #[case::compound("git commit -m 'Fixture subject' && git status --short")]
    #[case::compound_read("git rev-parse --show-toplevel && git status --short")]
    #[case::compound_read_with_shell_syntax_check(
        "git status --short && git log --oneline -5 && git diff --check && bash -n /fixture/bin/review-hook && git branch -vv"
    )]
    fn non_attribution_commands_preserve_permission_proxy_output(
        context_dir: TempDir,
        #[case] command: &str,
    ) {
        let proxy_output = json!({
            "hookSpecificOutput": {
                "hookEventName": "PreToolUse",
                "permissionDecision": "allow",
                "updatedInput": {"command": command},
            },
        });
        let result = transform(
            Provider::Codex,
            &envelope(
                codex_input(
                    command,
                    Some("fixture-model-blue"),
                    "fixture-tool-use-other",
                ),
                Some(proxy_output.clone()),
            ),
            context_dir.path(),
        )
        .expect("pass through command outside attribution handling");
        let context_count = std::fs::read_dir(context_dir.path())
            .expect("read context directory")
            .count();

        assert_eq!((result, context_count), (Some(proxy_output), 0));
    }

    #[rstest]
    fn codex_ask_decision_is_preserved(context_dir: TempDir) {
        let proxy_output = json!({
            "hookSpecificOutput": {
                "hookEventName": "PreToolUse",
                "permissionDecision": "ask",
                "permissionDecisionReason": "fixture rule",
            },
        });
        let result = transform(
            Provider::Codex,
            &envelope(
                codex_input(
                    COMMIT_COMMAND,
                    Some("fixture-model-blue"),
                    "fixture-tool-use-ask",
                ),
                Some(proxy_output.clone()),
            ),
            context_dir.path(),
        )
        .expect("preserve Codex ask response");

        assert_eq!(result, Some(proxy_output));
    }

    #[rstest]
    fn proxy_rewrite_to_unrecognized_command_is_preserved(context_dir: TempDir) {
        let rewritten_command =
            r#"runok exec --ask --hook-origin fixture-token -- 'git pull origin fixture-branch'"#;
        let proxy_output = json!({
            "hookSpecificOutput": {
                "hookEventName": "PreToolUse",
                "permissionDecision": "allow",
                "updatedInput": {"command": rewritten_command},
            },
        });
        let result = transform(
            Provider::Codex,
            &envelope(
                codex_input(
                    COMMIT_COMMAND,
                    Some("fixture-model-blue"),
                    "fixture-tool-use-rewrite",
                ),
                Some(proxy_output.clone()),
            ),
            context_dir.path(),
        )
        .expect("preserve proxy rewrite");

        assert_eq!(result, Some(proxy_output));
    }

    #[rstest]
    fn context_write_failure_does_not_deny_the_operation(context_dir: TempDir) {
        let invalid_context_dir = context_dir.path().join("fixture-not-a-directory");
        std::fs::write(&invalid_context_dir, "fixture").expect("create non-directory context path");
        let proxy_output = json!({
            "hookSpecificOutput": {
                "hookEventName": "PreToolUse",
                "permissionDecision": "allow",
            },
        });
        let result = transform(
            Provider::Codex,
            &envelope(
                codex_input(
                    COMMIT_COMMAND,
                    Some("fixture-model-blue"),
                    "fixture-tool-use-context-error",
                ),
                Some(proxy_output.clone()),
            ),
            &invalid_context_dir,
        )
        .expect("skip attribution on context write failure");

        assert_eq!(result, Some(proxy_output));
    }

    #[rstest]
    fn unrelated_claude_transcript_model_is_not_used(context_dir: TempDir) {
        let transcript_path = context_dir.path().join("fixture-transcript.jsonl");
        let line = json!({
            "type": "assistant",
            "message": {
                "model": "fixture-recent-model",
                "content": [{
                    "type": "tool_use",
                    "id": "fixture-other-tool-use",
                    "name": "Bash",
                    "input": {"command": COMMIT_COMMAND},
                }],
            },
        })
        .to_string();
        let mut transcript_file =
            std::fs::File::create(&transcript_path).expect("create fixture transcript");
        writeln!(transcript_file, "{line}").expect("write fixture transcript");
        let proxy_output = json!({
            "hookSpecificOutput": {
                "hookEventName": "PreToolUse",
                "permissionDecision": "allow",
            },
        });
        let input = json!({
            "hook_event_name": "PreToolUse",
            "tool_name": "Bash",
            "tool_use_id": "fixture-requested-tool-use",
            "transcript_path": transcript_path,
            "tool_input": {"command": COMMIT_COMMAND},
        });
        let result = transform(
            Provider::ClaudeCode,
            &envelope(input, Some(proxy_output.clone())),
            context_dir.path(),
        )
        .expect("skip attribution without matching transcript evidence");

        assert_eq!(result, Some(proxy_output));
    }
}
