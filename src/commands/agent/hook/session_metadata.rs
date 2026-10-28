use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use serde_json::Value;

use super::super::transcript::claude_assistant_model;
use super::super::types::{Engine, HookEvent, HookInput};

const TRANSCRIPT_CHUNK_SIZE: usize = 16 * 1024;
const MAX_CODEX_EFFORT_SCAN_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Default)]
pub(super) struct Update {
    hook_model: Option<String>,
    transcript_model: Option<String>,
    effort: Option<String>,
}

impl Update {
    pub(super) fn apply(&self, model: &mut Option<String>, effort: &mut Option<String>) -> bool {
        let previous = (model.clone(), effort.clone());

        if let Some(value) = &self.hook_model {
            *model = Some(value.clone());
        } else if model.is_none()
            && let Some(value) = &self.transcript_model
        {
            *model = Some(value.clone());
        }

        if let Some(value) = &self.effort {
            *effort = Some(value.clone());
        }

        previous != (model.clone(), effort.clone())
    }
}

pub(super) fn collect(
    event: HookEvent,
    input: &HookInput,
    current_model: Option<&str>,
    transcript_path: Option<&Path>,
) -> Update {
    if input.agent_id.is_some() {
        return Update::default();
    }

    match input.engine {
        Engine::Claude => {
            let hook_model = match event {
                HookEvent::SessionStart => input.model.as_deref(),
                HookEvent::PostModelSwitch => input.to_model.as_deref(),
                _ => None,
            }
            .filter(|model| !model.is_empty())
            .map(str::to_owned);

            let transcript_model = if hook_model.is_none() && current_model.is_none() {
                transcript_path
                    .and_then(latest_claude_model)
                    .filter(|model| !model.is_empty())
            } else {
                None
            };

            Update {
                hook_model,
                transcript_model,
                effort: input
                    .effort
                    .as_ref()
                    .and_then(|effort| effort.level.as_deref())
                    .filter(|effort| !effort.is_empty())
                    .map(str::to_owned),
            }
        }
        Engine::Codex => Update {
            hook_model: input
                .model
                .as_deref()
                .filter(|model| !model.is_empty())
                .map(str::to_owned),
            transcript_model: None,
            effort: transcript_path.and_then(latest_codex_effort),
        },
    }
}

fn latest_claude_model(path: &Path) -> Option<String> {
    claude_assistant_model(&last_record(path, "assistant")?).map(str::to_owned)
}

fn latest_codex_effort(path: &Path) -> Option<String> {
    last_record_with_limit(path, "turn_context", Some(MAX_CODEX_EFFORT_SCAN_BYTES))?
        .get("payload")?
        .get("effort")
        .and_then(Value::as_str)
        .filter(|effort| !effort.is_empty())
        .map(str::to_owned)
}

fn last_record(path: &Path, expected_type: &str) -> Option<Value> {
    last_record_with_limit(path, expected_type, None)
}

fn last_record_with_limit(
    path: &Path,
    expected_type: &str,
    max_scan_bytes: Option<u64>,
) -> Option<Value> {
    let mut file = File::open(path).ok()?;
    let mut remaining = file.seek(SeekFrom::End(0)).ok()?;
    let scan_start = max_scan_bytes.map_or(0, |limit| remaining.saturating_sub(limit));
    let mut carry = Vec::new();

    while remaining > scan_start {
        let start = remaining
            .saturating_sub(TRANSCRIPT_CHUNK_SIZE as u64)
            .max(scan_start);
        let mut chunk = vec![0; usize::try_from(remaining - start).ok()?];
        file.seek(SeekFrom::Start(start)).ok()?;
        file.read_exact(&mut chunk).ok()?;
        chunk.extend_from_slice(&carry);

        let mut lines = chunk.split(|byte| *byte == b'\n').collect::<Vec<_>>();
        if start > 0 {
            if start > scan_start {
                carry = lines.first()?.to_vec();
                lines.remove(0);
            } else {
                carry.clear();
                if scan_start > 0 {
                    lines.remove(0);
                }
            }
        } else {
            carry.clear();
        }

        for line in lines.into_iter().rev().filter(|line| !line.is_empty()) {
            let Ok(value) = serde_json::from_slice::<Value>(line) else {
                continue;
            };
            if value.get("type").and_then(Value::as_str) == Some(expected_type) {
                return Some(value);
            }
        }

        remaining = start;
    }

    None
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use rstest::rstest;
    use serde_json::{Value, json};

    use super::*;

    fn input(engine: Engine, fields: Value) -> HookInput {
        let mut value = json!({
            "session_id": "fixture-session",
            "cwd": "/fixture/worktree",
            "transcript_path": null,
            "engine": engine,
        });
        value
            .as_object_mut()
            .expect("hook input should be an object")
            .extend(
                fields
                    .as_object()
                    .expect("fields should be an object")
                    .clone(),
            );
        serde_json::from_value(value).expect("hook input should deserialize")
    }

    fn transcript(content: &str) -> tempfile::NamedTempFile {
        let mut file = tempfile::NamedTempFile::new().expect("transcript should be created");
        writeln!(file, "{content}").expect("transcript should be written");
        file
    }

    #[rstest]
    #[case::session_start(HookEvent::SessionStart, json!({"model": "fixture-model-amber"}), "fixture-model-amber")]
    #[case::model_switch(HookEvent::PostModelSwitch, json!({"to_model": "fixture-model-cobalt"}), "fixture-model-cobalt")]
    fn claude_uses_the_model_from_supported_events(
        #[case] event: HookEvent,
        #[case] fields: Value,
        #[case] expected_model: &str,
    ) {
        let hook_input = input(Engine::Claude, fields);
        let update = collect(event, &hook_input, None, None);
        let mut model = None;
        let mut effort = None;

        update.apply(&mut model, &mut effort);

        assert_eq!(
            (model.as_deref(), effort.as_deref()),
            (Some(expected_model), None)
        );
    }

    #[test]
    fn claude_uses_the_latest_assistant_model_when_no_model_is_recorded() {
        let transcript = transcript(indoc::indoc! {r#"
                {"type":"assistant","message":{"model":"fixture-model-amber"}}
                {"type":"user","message":{}}
                {"type":"assistant","message":{"model":"fixture-model-cobalt"}}
            "#});
        let hook_input = input(Engine::Claude, json!({"effort": {"level": "high"}}));
        let update = collect(
            HookEvent::PreToolUse,
            &hook_input,
            None,
            Some(transcript.path()),
        );
        let mut model = None;
        let mut effort = None;

        update.apply(&mut model, &mut effort);

        assert_eq!(
            (model.as_deref(), effort.as_deref()),
            (Some("fixture-model-cobalt"), Some("high")),
        );
    }

    #[test]
    fn claude_keeps_recorded_values_when_hook_values_are_missing() {
        let hook_input = input(Engine::Claude, json!({}));
        let update = collect(
            HookEvent::PostToolUse,
            &hook_input,
            Some("fixture-model-amber"),
            None,
        );
        let mut model = Some("fixture-model-amber".to_owned());
        let mut effort = Some("medium".to_owned());

        update.apply(&mut model, &mut effort);

        assert_eq!(
            (model.as_deref(), effort.as_deref()),
            (Some("fixture-model-amber"), Some("medium")),
        );
    }

    #[test]
    fn codex_uses_event_model_and_latest_turn_effort() {
        let transcript = transcript(indoc::indoc! {r#"
                {"type":"turn_context","payload":{"model":"fixture-model-amber","effort":"low"}}
                {"type":"turn_context","payload":{"model":"fixture-model-cobalt","effort":"xhigh"}}
            "#});
        let hook_input = input(Engine::Codex, json!({"model": "fixture-model-cobalt"}));
        let update = collect(
            HookEvent::PreToolUse,
            &hook_input,
            None,
            Some(transcript.path()),
        );
        let mut model = None;
        let mut effort = None;

        update.apply(&mut model, &mut effort);

        assert_eq!(
            (model.as_deref(), effort.as_deref()),
            (Some("fixture-model-cobalt"), Some("xhigh")),
        );
    }

    #[test]
    fn codex_keeps_recorded_effort_when_latest_turn_context_omits_it() {
        let transcript = transcript(indoc::indoc! {r#"
                {"type":"turn_context","payload":{"effort":"high"}}
                {"type":"turn_context","payload":{}}
            "#});
        let hook_input = input(Engine::Codex, json!({"model": "fixture-model-amber"}));
        let update = collect(
            HookEvent::PreToolUse,
            &hook_input,
            Some("fixture-model-amber"),
            Some(transcript.path()),
        );
        let mut model = Some("fixture-model-amber".to_owned());
        let mut effort = Some("high".to_owned());

        update.apply(&mut model, &mut effort);

        assert_eq!(
            (model.as_deref(), effort.as_deref()),
            (Some("fixture-model-amber"), Some("high")),
        );
    }

    #[test]
    fn subagent_events_do_not_update_model_or_effort() {
        let hook_input = input(
            Engine::Claude,
            json!({
                "agent_id": "fixture-agent",
                "model": "fixture-model-cobalt",
                "effort": {"level": "high"},
            }),
        );
        let update = collect(HookEvent::SessionStart, &hook_input, None, None);
        let mut model = Some("fixture-model-amber".to_owned());
        let mut effort = Some("medium".to_owned());

        update.apply(&mut model, &mut effort);

        assert_eq!(
            (model.as_deref(), effort.as_deref()),
            (Some("fixture-model-amber"), Some("medium")),
        );
    }

    #[test]
    fn transcript_lookup_finds_records_across_chunks() {
        let mut transcript = tempfile::NamedTempFile::new().expect("transcript should be created");
        writeln!(
            transcript,
            "{{\"type\":\"turn_context\",\"payload\":{{\"effort\":\"high\"}}}}"
        )
        .expect("context should be written");
        writeln!(
            transcript,
            "{{\"type\":\"user\",\"content\":\"{}\"}}",
            "x".repeat(20_000)
        )
        .expect("large event should be written");

        assert_eq!(
            latest_codex_effort(transcript.path()).as_deref(),
            Some("high")
        );
    }

    #[test]
    fn codex_effort_lookup_does_not_scan_before_the_size_limit() {
        let mut transcript = tempfile::NamedTempFile::new().expect("transcript should be created");
        writeln!(
            transcript,
            "{{\"type\":\"turn_context\",\"payload\":{{\"effort\":\"high\"}}}}"
        )
        .expect("context should be written");
        let payload = "x".repeat(
            usize::try_from(MAX_CODEX_EFFORT_SCAN_BYTES).expect("scan limit should fit usize"),
        );
        writeln!(
            transcript,
            "{{\"type\":\"user\",\"content\":\"{payload}\"}}"
        )
        .expect("large user event should be written");

        assert_eq!(latest_codex_effort(transcript.path()), None);
    }
}
