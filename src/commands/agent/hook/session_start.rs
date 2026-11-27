use std::io::Write;

use anyhow::Result;

use super::stop;
use crate::commands::agent::types::{Engine, HookEvent};

pub(super) fn write_hook_response(
    event: HookEvent,
    engine: Engine,
    block_reason: Option<&str>,
    marker: Option<&str>,
    output: impl Write,
) -> Result<()> {
    if event == HookEvent::SessionStart {
        let marker = if engine == Engine::Claude {
            marker
        } else {
            None
        };
        return stop::write_session_start_response(&session_start_context(marker), output);
    }
    stop::write_response(engine, event, block_reason, output)
}

pub(super) fn write_without_instruction(output: impl Write) -> Result<()> {
    stop::write_session_start_response("", output)
}

fn session_start_context(marker: Option<&str>) -> String {
    let web_ui_instruction = super::super::web_ui::session_start_instruction();
    let mut context = web_ui_instruction.to_string();
    if let Some(marker) = marker {
        context.push('\n');
        context.push('\n');
        context.push_str(&super::super::watch_web_user_marker::session_start_context(
            marker,
        ));
    }
    context
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    #[rstest]
    #[case::claude_without_watch_web_marker(Engine::Claude, None)]
    #[case::claude_with_watch_web_marker(
        Engine::Claude,
        Some("⟦armyknife-user:fixture-marker-cobalt⟧")
    )]
    #[case::codex(Engine::Codex, None)]
    fn session_start_response_includes_the_web_ui_instruction_without_the_decision_instruction(
        #[case] engine: Engine,
        #[case] marker: Option<&str>,
    ) {
        let mut output = Vec::new();

        write_hook_response(HookEvent::SessionStart, engine, None, marker, &mut output)
            .expect("SessionStart response should serialize");

        let web_ui_instruction = crate::commands::agent::web_ui::session_start_instruction();
        let mut expected_context = web_ui_instruction.to_string();
        if let Some(marker) = marker {
            expected_context.push('\n');
            expected_context.push('\n');
            expected_context.push_str(
                &crate::commands::agent::watch_web_user_marker::session_start_context(marker),
            );
        }

        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&output)
                .expect("response should be valid JSON"),
            serde_json::json!({
                "hookSpecificOutput": {
                    "hookEventName": "SessionStart",
                    "additionalContext": expected_context,
                }
            }),
        );
    }

    #[test]
    fn codex_non_session_start_response_uses_the_existing_stop_response() {
        let mut output = Vec::new();

        write_hook_response(
            HookEvent::PostToolUse,
            Engine::Codex,
            None,
            None,
            &mut output,
        )
        .expect("Codex response should write");

        assert_eq!(output, b"");
    }
}
