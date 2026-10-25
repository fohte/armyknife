use std::path::Path;

use anyhow::{Context, Result};
use serde_json::{Map, Value, json};

use super::command::{classify_command, inject_context};
use super::context::{self, OperationContext};
use super::transcript;
use super::{Provider, ProxyEnvelope, model_identity};

pub(super) fn transform(
    provider: Provider,
    raw_envelope: &str,
    unknown_reason: Option<&str>,
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
    let tool_name = input
        .get("tool_name")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("provider hook input has no tool name"))?;
    if !is_shell_tool(provider, tool_name) {
        return Ok(envelope.hook_output);
    }
    let tool_input = input
        .get("tool_input")
        .and_then(Value::as_object)
        .ok_or_else(|| anyhow::anyhow!("provider hook input has no command object"))?;
    let original_command = tool_input
        .get("command")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("provider hook input has no command"))?;
    let Some(operation) = classify_command(original_command)? else {
        return Ok(envelope.hook_output);
    };

    let final_command = envelope
        .hook_output
        .as_ref()
        .and_then(|output| output.pointer("/hookSpecificOutput/updatedInput/command"))
        .and_then(Value::as_str)
        .unwrap_or(original_command)
        .to_owned();
    if classify_command(&final_command)? != Some(operation) {
        anyhow::bail!("permission proxy changed the commit operation; attribution was not applied");
    }
    let _ = inject_context(&final_command, Path::new("/context-placeholder"))?;
    let _ = patch_hook_output(
        provider,
        input,
        envelope.hook_output.clone(),
        "context-placeholder",
    )?;

    let tool_use_id = input
        .get("tool_use_id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .ok_or_else(|| anyhow::anyhow!("provider hook input has no tool-use identifier"))?;
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
    let model = model_identity(model, unknown_reason)
        .map_err(|error| anyhow::anyhow!("runtime model is unavailable: {error}"))?;
    let context = OperationContext::new(provider, model, tool_use_id.to_owned())?;
    let context_path = context::create(&context, context_dir)?;
    let transformed = (|| {
        let command = inject_context(&final_command, &context_path)?;
        patch_hook_output(provider, input, envelope.hook_output, &command)
    })();
    match transformed {
        Ok(output) => Ok(Some(output)),
        Err(error) => {
            context::discard(&context_path)?;
            Err(error)
        }
    }
}

pub(super) fn blocked_output(_provider: Provider, reason: &str) -> Value {
    let mut specific = Map::new();
    specific.insert("hookEventName".to_owned(), json!("PreToolUse"));
    specific.insert("permissionDecision".to_owned(), json!("deny"));
    specific.insert("permissionDecisionReason".to_owned(), json!(reason));
    json!({"hookSpecificOutput": specific})
}

fn is_shell_tool(provider: Provider, tool_name: &str) -> bool {
    match provider {
        Provider::ClaudeCode => tool_name == "Bash",
        Provider::Codex => tool_name == "exec_command",
    }
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

    use super::super::Provider;
    use super::super::{CONTEXT_ENV, REQUIRED_ENV};
    use super::{blocked_output, transform};
    use crate::commands::agent::attribution::context::{self, ModelIdentity};
    use rstest::{fixture, rstest};
    use serde_json::{Value, json};
    use std::path::{Path, PathBuf};
    use tempfile::TempDir;

    const COMMIT_COMMAND: &str = "git commit -m 'Fixture subject'";

    #[fixture]
    fn context_dir() -> TempDir {
        tempfile::tempdir().expect("create test context directory")
    }

    fn envelope(input: Value, output: Option<Value>) -> String {
        json!({
            "hook_input": input,
            "hook_output": output,
        })
        .to_string()
    }

    fn codex_input(command: &str, model: Option<&str>, tool_use_id: &str) -> Value {
        let mut input = json!({
            "hook_event_name": "PreToolUse",
            "tool_name": "exec_command",
            "tool_use_id": tool_use_id,
            "tool_input": {
                "command": command,
                "timeout_ms": 2_500,
            },
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
    fn codex_keeps_raw_model_and_separates_tool_operations(context_dir: TempDir) {
        let first = transform(
            Provider::Codex,
            &envelope(
                codex_input(
                    COMMIT_COMMAND,
                    Some("fixture-model-red-2026"),
                    "fixture-tool-use-red",
                ),
                None,
            ),
            None,
            context_dir.path(),
        )
        .expect("transform first request")
        .expect("Codex must emit updated input");
        let second = transform(
            Provider::Codex,
            &envelope(
                codex_input(
                    COMMIT_COMMAND,
                    Some("fixture-model-blue-2026"),
                    "fixture-tool-use-blue",
                ),
                None,
            ),
            None,
            context_dir.path(),
        )
        .expect("transform second request")
        .expect("Codex must emit updated input");
        let first_path = context_path(&first);
        let second_path = context_path(&second);
        let first_context = context::read(&first_path).expect("read first operation context");
        let second_context = context::read(&second_path).expect("read second operation context");

        let actual = (
            first_path != second_path,
            [
                (first_context.tool(), first_context.model().trailer_value()),
                (
                    second_context.tool(),
                    second_context.model().trailer_value(),
                ),
            ],
            normalize_output(first, &first_path),
        );
        assert_eq!(
            actual,
            (
                true,
                [
                    ("Codex", "fixture-model-red-2026"),
                    ("Codex", "fixture-model-blue-2026"),
                ],
                json!({
                    "hookSpecificOutput": {
                        "hookEventName": "PreToolUse",
                        "permissionDecision": "allow",
                        "updatedInput": {
                            "command": format!("{CONTEXT_ENV}=<context-file> {REQUIRED_ENV}=1 {COMMIT_COMMAND}"),
                            "timeout_ms": 2_500,
                        },
                    },
                }),
            ),
        );
    }

    #[rstest]
    fn codex_preserves_the_runok_ask_wrapper(context_dir: TempDir) {
        let runok_command =
            r#"runok exec --ask --hook-origin fixture-token -- 'git commit -m "Fixture subject"'"#;
        let output = json!({
            "hookSpecificOutput": {
                "hookEventName": "PreToolUse",
                "permissionDecision": "allow",
                "updatedInput": {
                    "command": runok_command,
                },
            },
        });
        let result = transform(
            Provider::Codex,
            &envelope(
                codex_input(
                    COMMIT_COMMAND,
                    Some("fixture-model-red-2026"),
                    "fixture-tool-use-ask",
                ),
                Some(output),
            ),
            None,
            context_dir.path(),
        )
        .expect("transform runok ask")
        .expect("Codex must emit updated input");
        let path = context_path(&result);

        assert_eq!(
            normalize_output(result, &path),
            json!({
                "hookSpecificOutput": {
                    "hookEventName": "PreToolUse",
                    "permissionDecision": "allow",
                    "updatedInput": {
                        "command": r#"runok exec --ask --hook-origin fixture-token -- 'ARMYKNIFE_AGENT_ATTRIBUTION_CONTEXT=<context-file> ARMYKNIFE_AGENT_ATTRIBUTION_REQUIRED=1 git commit -m "Fixture subject"'"#,
                    },
                },
            }),
        );
    }

    #[rstest]
    fn claude_preserves_ask_while_using_the_exact_transcript_message(context_dir: TempDir) {
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
            std::fs::File::create(&transcript_path).expect("create fixture Claude transcript");
        writeln!(transcript_file, "{transcript_line}").expect("write fixture Claude transcript");
        let input = json!({
            "hook_event_name": "PreToolUse",
            "tool_name": "Bash",
            "tool_use_id": "fixture-tool-use-claude",
            "transcript_path": transcript_path,
            "tool_input": {"command": COMMIT_COMMAND},
        });
        let output = json!({
            "hookSpecificOutput": {
                "hookEventName": "PreToolUse",
                "permissionDecision": "ask",
                "permissionDecisionReason": "fixture rule",
            },
        });
        let result = transform(
            Provider::ClaudeCode,
            &envelope(input, Some(output)),
            None,
            context_dir.path(),
        )
        .expect("transform Claude request")
        .expect("Claude must receive updated input");
        let path = context_path(&result);
        let record = context::read(&path).expect("read Claude operation context");

        let actual = (
            record.model().trailer_value(),
            normalize_output(result, &path),
        );
        assert_eq!(
            actual,
            (
                "fixture-claude-model-2026",
                json!({
                    "hookSpecificOutput": {
                        "hookEventName": "PreToolUse",
                        "permissionDecision": "ask",
                        "permissionDecisionReason": "fixture rule",
                        "updatedInput": {
                            "command": format!("{CONTEXT_ENV}=<context-file> {REQUIRED_ENV}=1 {COMMIT_COMMAND}"),
                        },
                    },
                }),
            ),
        );
    }

    #[rstest]
    fn deny_is_preserved_without_creating_a_context(context_dir: TempDir) {
        let output = json!({
            "hookSpecificOutput": {
                "hookEventName": "PreToolUse",
                "permissionDecision": "deny",
                "permissionDecisionReason": "fixture denied rule",
            },
        });
        let result = transform(
            Provider::Codex,
            &envelope(
                codex_input(COMMIT_COMMAND, None, "fixture-tool-use-denied"),
                Some(output.clone()),
            ),
            None,
            context_dir.path(),
        )
        .expect("preserve denied request");
        let file_count = std::fs::read_dir(context_dir.path())
            .expect("read context directory")
            .count();

        assert_eq!((result, file_count), (Some(output), 0));
    }

    #[rstest]
    fn missing_codex_model_requires_an_explicit_unknown_reason(context_dir: TempDir) {
        let raw = envelope(
            codex_input(COMMIT_COMMAND, None, "fixture-tool-use-unknown"),
            None,
        );
        let without_exception = transform(Provider::Codex, &raw, None, context_dir.path())
            .expect_err("missing model must fail closed");
        let with_exception = transform(
            Provider::Codex,
            &raw,
            Some("fixture runtime model unavailable"),
            context_dir.path(),
        )
        .expect("explicit unknown exception is accepted")
        .expect("Codex must receive updated input");
        let path = context_path(&with_exception);
        let context = context::read(&path).expect("read explicit unknown context");

        let actual = (
            without_exception.to_string(),
            context.model().clone(),
            normalize_output(with_exception, &path),
        );
        assert_eq!(
            actual,
            (
                "runtime model is unavailable: runtime model is unavailable and no explicit unknown reason was supplied".to_owned(),
                ModelIdentity::Unknown {
                    reason: "fixture runtime model unavailable".to_owned(),
                },
                json!({
                    "hookSpecificOutput": {
                        "hookEventName": "PreToolUse",
                        "permissionDecision": "allow",
                        "updatedInput": {
                            "command": format!("{CONTEXT_ENV}=<context-file> {REQUIRED_ENV}=1 {COMMIT_COMMAND}"),
                            "timeout_ms": 2_500,
                        },
                    },
                }),
            ),
        );
    }

    #[rstest]
    fn non_commit_commands_pass_through_unchanged(context_dir: TempDir) {
        let input = codex_input(
            "git status --short",
            Some("fixture-model-red-2026"),
            "fixture-tool-use-status",
        );
        let output = json!({
            "hookSpecificOutput": {
                "hookEventName": "PreToolUse",
                "permissionDecision": "allow",
            },
        });
        let result = transform(
            Provider::Codex,
            &envelope(input, Some(output.clone())),
            None,
            context_dir.path(),
        )
        .expect("pass through non-commit request");
        let file_count = std::fs::read_dir(context_dir.path())
            .expect("read context directory")
            .count();

        assert_eq!((result, file_count), (Some(output), 0));
    }

    #[rstest]
    fn unsupported_history_rewrites_are_rejected(context_dir: TempDir) {
        let input = codex_input(
            "git rebase fixture-base",
            Some("fixture-model-red-2026"),
            "fixture-tool-use-rebase",
        );
        let error = transform(
            Provider::Codex,
            &envelope(input, None),
            None,
            context_dir.path(),
        )
        .expect_err("rebase must not bypass attribution hooks");

        assert_eq!(
            error.to_string(),
            "git rebase may create commits outside the supported single-commit hook path",
        );
    }

    #[rstest]
    fn claude_does_not_use_an_unrelated_transcript_model_as_fallback(context_dir: TempDir) {
        let transcript_path = context_dir.path().join("fixture-transcript.jsonl");
        let unrelated_message = json!({
            "type": "assistant",
            "message": {
                "model": "fixture-latest-model",
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
            std::fs::File::create(&transcript_path).expect("create fixture Claude transcript");
        writeln!(transcript_file, "{unrelated_message}").expect("write fixture Claude transcript");
        let input = json!({
            "hook_event_name": "PreToolUse",
            "tool_name": "Bash",
            "tool_use_id": "fixture-requested-tool-use",
            "transcript_path": transcript_path,
            "tool_input": {"command": COMMIT_COMMAND},
        });
        let error = transform(
            Provider::ClaudeCode,
            &envelope(input, None),
            None,
            context_dir.path(),
        )
        .expect_err("an unrelated transcript model cannot identify the operation");

        assert_eq!(
            error.to_string(),
            "runtime model is unavailable: runtime model is unavailable and no explicit unknown reason was supplied",
        );
    }

    #[rstest]
    #[case::long_skip_hooks("git commit --no-verify -m 'Fixture subject'")]
    #[case::short_skip_hooks("git commit -n -m 'Fixture subject'")]
    #[case::git_config_override("git -c CORE.HOOKSPATH=/fixture/hooks commit -m 'Fixture subject'")]
    fn hook_bypass_options_are_rejected(context_dir: TempDir, #[case] command: &str) {
        let input = codex_input(
            command,
            Some("fixture-model-red-2026"),
            "fixture-tool-use-no-verify",
        );
        let error = transform(
            Provider::Codex,
            &envelope(input, None),
            None,
            context_dir.path(),
        )
        .expect_err("commit-msg bypass must fail closed");

        assert_eq!(
            error.to_string(),
            "commit command bypasses the attribution integrity hook",
        );
    }

    #[rstest]
    fn invalid_hook_output_is_a_deny_response() {
        assert_eq!(
            blocked_output(Provider::Codex, "fixture context missing"),
            json!({
                "hookSpecificOutput": {
                    "hookEventName": "PreToolUse",
                    "permissionDecision": "deny",
                    "permissionDecisionReason": "fixture context missing",
                },
            }),
        );
    }
}
