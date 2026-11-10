use chrono::{DateTime, Utc};
use serde_json::Value;

use super::super::super::BackgroundTask;
use super::{ToolUse, tool_result_text};

#[derive(Default)]
pub(super) struct BackgroundTaskTracker {
    tasks: Vec<RunningBackgroundTask>,
}

impl BackgroundTaskTracker {
    pub(super) fn complete_tool_use(
        &mut self,
        tool_use: &ToolUse,
        record: &Value,
        result: &Value,
        now: DateTime<Utc>,
    ) {
        if !is_background_tool(&tool_use.name, tool_use.input.as_ref(), record) {
            return;
        }
        let Some(task_id) = background_task_id(&tool_use.name, record, result) else {
            return;
        };
        self.tasks.retain(|task| task.id != task_id);
        self.tasks.push(RunningBackgroundTask {
            id: task_id,
            description: tool_use_description(tool_use),
            started_at: tool_use.started_at.unwrap_or(now),
        });
    }

    pub(super) fn finish_notification(&mut self, record: &Value) {
        let Some(message) = record.get("message") else {
            return;
        };
        let text = super::user_text(message);
        let Some(task_id) = tagged_value(&text, "task-id") else {
            return;
        };
        let Some(status) = tagged_value(&text, "status") else {
            return;
        };
        if matches!(status, "completed" | "failed" | "killed") {
            self.tasks.retain(|task| task.id != task_id);
        }
    }

    pub(super) fn stop(&mut self, input: Option<&Value>, name: &str) {
        if !is_task_stop_tool(name) {
            return;
        }
        let Some(task_id) = input.and_then(|input| {
            [
                "task_id", "taskId", "agent_id", "agentId", "shell_id", "shellId",
            ]
            .into_iter()
            .find_map(|key| input.get(key).and_then(Value::as_str))
        }) else {
            return;
        };
        self.tasks.retain(|task| task.id != task_id);
    }

    pub(super) fn into_api_tasks(self, now: DateTime<Utc>) -> Vec<BackgroundTask> {
        self.tasks
            .into_iter()
            .map(|task| BackgroundTask {
                id: task.id,
                description: task.description,
                elapsed_seconds: u64::try_from(
                    now.signed_duration_since(task.started_at)
                        .num_seconds()
                        .max(0),
                )
                .unwrap_or_default(),
            })
            .collect()
    }
}

pub(super) fn is_task_notification(record: &Value) -> bool {
    let has_task_notification_origin = record
        .get("origin")
        .and_then(|origin| origin.get("kind"))
        .and_then(Value::as_str)
        == Some("task-notification");
    has_task_notification_origin
        || (record.get("type").and_then(Value::as_str) == Some("user")
            && record.get("message").is_some_and(|message| {
                super::user_text(message)
                    .trim_start()
                    .starts_with("<task-notification>")
            }))
}

pub(super) fn record_timestamp(record: &Value) -> Option<DateTime<Utc>> {
    record
        .get("timestamp")
        .and_then(Value::as_str)
        .and_then(|timestamp| DateTime::parse_from_rfc3339(timestamp).ok())
        .map(|timestamp| timestamp.with_timezone(&Utc))
}

struct RunningBackgroundTask {
    id: String,
    description: String,
    started_at: DateTime<Utc>,
}

fn is_background_tool(name: &str, input: Option<&Value>, record: &Value) -> bool {
    (matches!(name, "Bash" | "Task")
        && input
            .and_then(|input| input.get("run_in_background"))
            .and_then(Value::as_bool)
            == Some(true))
        || (name == "Agent" && is_async_agent_result(record.get("toolUseResult")))
}

fn is_async_agent_result(result: Option<&Value>) -> bool {
    result.is_some_and(|result| {
        result.get("isAsync").and_then(Value::as_bool) == Some(true)
            && result.get("status").and_then(Value::as_str) == Some("async_launched")
    })
}

fn is_task_stop_tool(name: &str) -> bool {
    matches!(name, "TaskStop" | "KillShell")
}

fn tool_use_description(tool_use: &ToolUse) -> String {
    let description = tool_use
        .input
        .as_ref()
        .and_then(|input| {
            ["description", "command", "prompt"]
                .into_iter()
                .find_map(|key| input.get(key).and_then(Value::as_str))
        })
        .filter(|description| !description.trim().is_empty())
        .unwrap_or(&tool_use.name);
    description.chars().take(240).collect()
}

fn background_task_id(name: &str, record: &Value, result: &Value) -> Option<String> {
    let structured_result = record.get("toolUseResult");
    if name == "Agent"
        && is_async_agent_result(structured_result)
        && let Some(agent_id) = structured_result
            .and_then(|result| result.get("agentId"))
            .and_then(Value::as_str)
    {
        return Some(agent_id.to_owned());
    }
    let id = structured_result.and_then(|result| {
        [
            "task_id",
            "taskId",
            "background_task_id",
            "backgroundTaskId",
            "shell_id",
            "shellId",
        ]
        .into_iter()
        .find_map(|key| result.get(key).and_then(Value::as_str))
    });
    id.map(str::to_owned).or_else(|| {
        tool_result_text(result.get("content")?)
            .as_deref()
            .and_then(background_task_id_from_text)
    })
}

fn background_task_id_from_text(text: &str) -> Option<String> {
    let lowercase_text = text.to_ascii_lowercase();
    let background_index = lowercase_text.find("background")?;
    let id_index = lowercase_text[background_index..].find("id:")? + background_index + "id:".len();
    let candidate = text[id_index..].split_whitespace().next()?;
    let id = candidate.trim_matches(|character: char| {
        !character.is_ascii_alphanumeric() && !matches!(character, '-' | '_')
    });
    (!id.is_empty()).then(|| id.to_owned())
}

fn tagged_value<'a>(text: &'a str, tag: &str) -> Option<&'a str> {
    let start = format!("<{tag}>");
    let end = format!("</{tag}>");
    text.split_once(&start)?
        .1
        .split_once(&end)
        .map(|(value, _)| value)
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use chrono::{DateTime, Utc};
    use indoc::indoc;
    use rstest::{fixture, rstest};
    use serde_json::{Value, json};

    use super::super::parse_with_background_tasks;
    use crate::commands::agent::watch_web::messages::{BackgroundTask, ChatItem, ToolOperation};

    #[rstest]
    fn reports_a_running_background_bash_task_with_elapsed_time(
        background_bash_transcript: String,
    ) {
        assert_eq!(
            parse_with_background_tasks(
                Cursor::new(background_bash_transcript),
                None,
                None,
                None,
                timestamp("2026-01-01T00:01:30Z"),
            )
            .unwrap(),
            (
                background_bash_messages(),
                vec![BackgroundTask {
                    id: "fixture-task-id".to_string(),
                    description: "Watch the synthetic build".to_string(),
                    elapsed_seconds: 90,
                }],
            ),
        );
    }

    #[rstest]
    #[case::completed("completed", Vec::new(), true)]
    #[case::failed("failed", Vec::new(), true)]
    #[case::killed("killed", Vec::new(), true)]
    #[case::still_running("running", vec![BackgroundTask {
        id: "fixture-task-id".to_string(),
        description: "Watch the synthetic build".to_string(),
        elapsed_seconds: 90,
    }], true)]
    #[case::legacy_notification("completed", Vec::new(), false)]
    fn task_notifications_only_remove_finished_tasks(
        background_bash_transcript: String,
        #[case] status: &str,
        #[case] expected_tasks: Vec<BackgroundTask>,
        #[case] include_origin: bool,
    ) {
        let mut transcript = background_bash_transcript;
        let mut notification = json!({
            "uuid": "fixture-notification-row",
            "parentUuid": "fixture-disconnected-parent-row",
            "type": "user",
            "message": {"content": format!(
                "<task-notification><task-id>fixture-task-id</task-id><status>{status}</status></task-notification>"
            )},
        });
        if include_origin {
            notification["origin"] = json!({"kind": "task-notification"});
        }
        transcript.push_str(&serde_json::to_string(&notification).unwrap());
        transcript.push('\n');
        transcript.push_str(
            &serde_json::to_string(&json!({
                "uuid": "fixture-current-row",
                "parentUuid": "fixture-bash-result-row",
                "type": "system",
            }))
            .unwrap(),
        );
        transcript.push('\n');

        assert_eq!(
            parse_with_background_tasks(
                Cursor::new(transcript),
                None,
                None,
                None,
                timestamp("2026-01-01T00:01:30Z"),
            )
            .unwrap(),
            (background_bash_messages(), expected_tasks),
        );
    }

    #[rstest]
    #[case::task_stop(
        "TaskStop",
        json!({"task_id": "fixture-task-id"}),
        "TaskStop",
    )]
    #[case::kill_shell(
        "KillShell",
        json!({"shell_id": "fixture-task-id"}),
        "KillShell",
    )]
    fn stop_tool_calls_remove_running_tasks(
        background_bash_transcript: String,
        #[case] tool: &str,
        #[case] input: Value,
        #[case] result_tool: &str,
    ) {
        let mut transcript = background_bash_transcript;
        let stop_call = json!({
            "uuid": "fixture-stop-call-row",
            "parentUuid": "fixture-bash-result-row",
            "type": "assistant",
            "message": {"content": [{
                "type": "tool_use",
                "id": "fixture-stop-tool-use-id",
                "name": tool,
                "input": input,
            }]},
        });
        let stop_result = json!({
            "uuid": "fixture-stop-result-row",
            "parentUuid": "fixture-stop-call-row",
            "type": "user",
            "toolUseResult": {},
            "sourceToolAssistantUUID": "fixture-stop-call-row",
            "message": {"content": [{
                "type": "tool_result",
                "tool_use_id": "fixture-stop-tool-use-id",
                "content": "Synthetic stop result.",
            }]},
        });
        transcript.push_str(&serde_json::to_string(&stop_call).unwrap());
        transcript.push('\n');
        transcript.push_str(&serde_json::to_string(&stop_result).unwrap());
        transcript.push('\n');

        assert_eq!(
            parse_with_background_tasks(
                Cursor::new(transcript),
                None,
                None,
                None,
                timestamp("2026-01-01T00:01:30Z"),
            )
            .unwrap(),
            (
                vec![
                    ChatItem::User {
                        text: "Watch the fixture build.".to_string(),
                        from: None,
                    },
                    ChatItem::ToolSummary {
                        summary: "2 actions".to_string(),
                        operations: vec![
                            ToolOperation {
                                tool: "Bash".to_string(),
                                target: Some("watch fixture build".to_string()),
                            },
                            ToolOperation {
                                tool: result_tool.to_string(),
                                target: None,
                            },
                        ],
                    },
                ],
                Vec::new(),
            ),
        );
    }

    #[test]
    fn reports_a_running_task_tool_subagent() {
        let transcript = indoc! {r#"
            {"uuid":"fixture-task-prompt-row","parentUuid":null,"type":"user","message":{"content":"Delegate a synthetic task."}}
            {"uuid":"fixture-task-call-row","parentUuid":"fixture-task-prompt-row","type":"assistant","timestamp":"2026-01-01T00:00:00Z","message":{"content":[{"type":"tool_use","id":"fixture-task-tool-use-id","name":"Task","input":{"description":"Review the synthetic task","prompt":"Review a fictional task.","run_in_background":true}}]}}
            {"uuid":"fixture-task-result-row","parentUuid":"fixture-task-call-row","type":"user","sourceToolAssistantUUID":"fixture-task-call-row","toolUseResult":{"task_id":"fixture-task-id"},"message":{"content":[{"type":"tool_result","tool_use_id":"fixture-task-tool-use-id","content":"The task was launched."}]}}
        "#};

        assert_eq!(
            parse_with_background_tasks(
                Cursor::new(transcript),
                None,
                None,
                None,
                timestamp("2026-01-01T00:00:30Z"),
            )
            .unwrap(),
            (
                vec![
                    ChatItem::User {
                        text: "Delegate a synthetic task.".to_string(),
                        from: None,
                    },
                    ChatItem::ToolSummary {
                        summary: "1 action".to_string(),
                        operations: vec![ToolOperation {
                            tool: "Task".to_string(),
                            target: Some("Review the synthetic task".to_string()),
                        }],
                    },
                ],
                vec![BackgroundTask {
                    id: "fixture-task-id".to_string(),
                    description: "Review the synthetic task".to_string(),
                    elapsed_seconds: 30,
                }],
            ),
        );
    }

    #[test]
    fn reports_an_async_agent_from_its_structured_tool_result() {
        let transcript = indoc! {r#"
            {"uuid":"fixture-agent-prompt-row","parentUuid":null,"type":"user","message":{"content":"Review the synthetic change."}}
            {"uuid":"fixture-agent-call-row","parentUuid":"fixture-agent-prompt-row","type":"assistant","timestamp":"2026-01-01T00:00:00Z","message":{"content":[{"type":"tool_use","id":"fixture-agent-tool-use-id","name":"Agent","input":{"description":"Review the synthetic change","prompt":"Review a fictional change."}}]}}
            {"uuid":"fixture-agent-result-row","parentUuid":"fixture-agent-call-row","type":"user","sourceToolAssistantUUID":"fixture-agent-call-row","toolUseResult":{"agentId":"fixture-agent-id","isAsync":true,"status":"async_launched"},"message":{"content":[{"type":"tool_result","tool_use_id":"fixture-agent-tool-use-id","content":"agentId: fixture-agent-id (internal ID)"}]}}
        "#};

        assert_eq!(
            parse_with_background_tasks(
                Cursor::new(transcript),
                None,
                None,
                None,
                timestamp("2026-01-01T00:00:45Z"),
            )
            .unwrap(),
            (
                vec![
                    ChatItem::User {
                        text: "Review the synthetic change.".to_string(),
                        from: None,
                    },
                    ChatItem::ToolSummary {
                        summary: "1 action".to_string(),
                        operations: vec![ToolOperation {
                            tool: "Agent".to_string(),
                            target: Some("Review the synthetic change".to_string()),
                        }],
                    },
                ],
                vec![BackgroundTask {
                    id: "fixture-agent-id".to_string(),
                    description: "Review the synthetic change".to_string(),
                    elapsed_seconds: 45,
                }],
            ),
        );
    }

    #[fixture]
    fn background_bash_transcript() -> String {
        indoc! {r#"
            {"uuid":"fixture-prompt-row","parentUuid":null,"type":"user","message":{"content":"Watch the fixture build."}}
            {"uuid":"fixture-bash-call-row","parentUuid":"fixture-prompt-row","type":"assistant","timestamp":"2026-01-01T00:00:00Z","message":{"content":[{"type":"tool_use","id":"fixture-bash-tool-use-id","name":"Bash","input":{"command":"watch fixture build","description":"Watch the synthetic build","run_in_background":true}}]}}
            {"uuid":"fixture-bash-result-row","parentUuid":"fixture-bash-call-row","type":"user","sourceToolAssistantUUID":"fixture-bash-call-row","toolUseResult":{},"message":{"content":[{"type":"tool_result","tool_use_id":"fixture-bash-tool-use-id","content":"Command running in background with ID: fixture-task-id. Output is being written to: /fixture/task-output.log"}]}}
        "#}
        .to_string()
    }

    fn background_bash_messages() -> Vec<ChatItem> {
        vec![
            ChatItem::User {
                text: "Watch the fixture build.".to_string(),
                from: None,
            },
            ChatItem::ToolSummary {
                summary: "1 action".to_string(),
                operations: vec![ToolOperation {
                    tool: "Bash".to_string(),
                    target: Some("watch fixture build".to_string()),
                }],
            },
        ]
    }

    fn timestamp(value: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(value)
            .unwrap()
            .with_timezone(&Utc)
    }
}
