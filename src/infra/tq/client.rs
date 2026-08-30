//! tq CLI client.
//!
//! Shells out to the `tq` binary rather than speaking to the tq API
//! directly: `tq` already resolves its own base URL and Cloudflare Access
//! credentials (see the dotfiles `tq` wrapper), so this module never touches
//! either.

use std::time::Duration;

use serde::Deserialize;

use super::error::{Result, TqError};
use crate::infra::external_tool::ExternalTool;
use crate::infra::process;

const SESSION_LIST_ARGS: &[&str] = &["session", "list"];

/// Claude Code's provider identifier in tq's agent-session schema. Always
/// this literal value for sessions armyknife manages.
const CLAUDE_CODE_PROVIDER: &str = "claude_code";

/// Upper bound on how long a single `tq` invocation may run. `tq` talks to a
/// Cloudflare-Access-gated backend over the network, so an unresponsive
/// backend must not leak a blocking-pool thread and child process on every
/// session-creation-triggered refetch.
const TQ_COMMAND_TIMEOUT: Duration = Duration::from_secs(10);

/// One tq task linked to an agent session.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TqTask {
    pub id: String,
    pub number: u32,
    pub title: String,
    /// The id of this task's direct parent, if any. Absent on older `tq`
    /// binaries that predate this field, so it must default to `None`
    /// rather than fail deserialization.
    #[serde(default)]
    pub parent_id: Option<String>,
}

/// The tq tasks linked to a single agent session, as returned by
/// `tq session list`. A session with no linked tasks still appears, with an
/// empty `tasks` list.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionTasks {
    pub session_id: String,
    pub tasks: Vec<TqTask>,
}

pub struct TqClient;

impl TqClient {
    /// Returns `None` when the `tq` binary isn't on `PATH` -- the tq
    /// integration is then simply skipped, not treated as an error.
    pub fn detect() -> Option<Self> {
        ExternalTool::Tq.is_available().then_some(Self)
    }

    /// Lists every Claude Code agent session tq knows about, each with the
    /// tasks it's linked to.
    pub async fn list_session_tasks(&self) -> Result<Vec<SessionTasks>> {
        let join_result = tokio::task::spawn_blocking(run_session_list).await;
        match join_result {
            Ok(result) => result,
            Err(e) => Err(TqError::command_failed(
                SESSION_LIST_ARGS,
                format!("task panicked: {e}"),
                None,
            )),
        }
    }

    /// Resolves `task_id`'s web page URL via `tq task url` -- tq's own base
    /// URL stays inside `tq` (see module docs); this module only ever sees
    /// the finished URL string.
    pub async fn task_url(&self, task_id: &str) -> Result<String> {
        let id = task_id.to_string();
        let join_result = tokio::task::spawn_blocking(move || run_task_url(&id)).await;
        match join_result {
            Ok(result) => result,
            Err(e) => Err(TqError::command_failed(
                &["task", "url", task_id],
                format!("task panicked: {e}"),
                None,
            )),
        }
    }

    /// Deletes tq's record of a Claude Code agent session by session_id.
    /// Best-effort: the caller (see `cc::delete_tq_session_detached`) treats
    /// any error here as non-fatal, since tq's own periodic cleanup of stale
    /// sessions is the fallback if this never runs.
    pub fn delete_session(&self, session_id: &str) -> Result<()> {
        let args = ["session", "delete", CLAUDE_CODE_PROVIDER, session_id];
        let mut command = ExternalTool::Tq.command();
        command.args(args);

        let output = process::run_with_timeout(command, TQ_COMMAND_TIMEOUT)
            .map_err(|e| TqError::command_failed(&args, e.to_string(), None))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            return Err(TqError::command_failed(
                &args,
                "command exited with non-zero status",
                Some(stderr),
            ));
        }

        Ok(())
    }
}

/// Runs `tq session list` to completion and parses its stdout. Blocking, so
/// callers must run it via [`tokio::task::spawn_blocking`].
fn run_session_list() -> Result<Vec<SessionTasks>> {
    let mut command = ExternalTool::Tq.command();
    command.args(SESSION_LIST_ARGS);

    let output = process::run_with_timeout(command, TQ_COMMAND_TIMEOUT)
        .map_err(|e| TqError::command_failed(SESSION_LIST_ARGS, e.to_string(), None))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(TqError::command_failed(
            SESSION_LIST_ARGS,
            "command exited with non-zero status",
            Some(stderr),
        ));
    }

    parse_session_list(&String::from_utf8_lossy(&output.stdout))
}

/// Runs `tq task url <task_id>` to completion and returns its trimmed
/// stdout (the task's web page URL). Blocking, so callers must run it via
/// [`tokio::task::spawn_blocking`].
fn run_task_url(task_id: &str) -> Result<String> {
    let args = ["task", "url", task_id];
    let mut command = ExternalTool::Tq.command();
    command.args(args);

    let output = process::run_with_timeout(command, TQ_COMMAND_TIMEOUT)
        .map_err(|e| TqError::command_failed(&args, e.to_string(), None))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(TqError::command_failed(
            &args,
            "command exited with non-zero status",
            Some(stderr),
        ));
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Pure parse step, isolated from the process spawn so it can be unit tested
/// with literal JSON fixtures instead of invoking the real `tq` binary.
fn parse_session_list(stdout: &str) -> Result<Vec<SessionTasks>> {
    Ok(serde_json::from_str(stdout)?)
}

#[cfg(test)]
mod tests {
    use indoc::indoc;
    use rstest::rstest;

    use super::*;

    #[test]
    fn parses_sessions_with_and_without_linked_tasks() {
        let json = indoc! {r#"
            [
              {
                "sessionId": "session-1",
                "tasks": [
                  {
                    "id": "task-uuid-1",
                    "number": 42,
                    "title": "Fix the bug",
                    "parentId": "task-uuid-parent"
                  },
                  { "id": "task-uuid-2", "number": 43, "title": "Parent task" }
                ]
              },
              {
                "sessionId": "session-2",
                "tasks": []
              }
            ]
        "#};

        let result = parse_session_list(json).unwrap();

        assert_eq!(
            result,
            vec![
                SessionTasks {
                    session_id: "session-1".to_string(),
                    tasks: vec![
                        TqTask {
                            id: "task-uuid-1".to_string(),
                            number: 42,
                            title: "Fix the bug".to_string(),
                            parent_id: Some("task-uuid-parent".to_string()),
                        },
                        TqTask {
                            id: "task-uuid-2".to_string(),
                            number: 43,
                            title: "Parent task".to_string(),
                            parent_id: None,
                        },
                    ],
                },
                SessionTasks {
                    session_id: "session-2".to_string(),
                    tasks: vec![],
                },
            ]
        );
    }

    #[rstest]
    #[case::explicit_null(indoc! {r#"
        [
          {
            "sessionId": "session-1",
            "tasks": [
              { "id": "task-uuid-1", "number": 1, "title": "Task", "parentId": null }
            ]
          }
        ]
    "#})]
    #[case::key_absent(indoc! {r#"
        [
          {
            "sessionId": "session-1",
            "tasks": [
              { "id": "task-uuid-1", "number": 1, "title": "Task" }
            ]
          }
        ]
    "#})]
    fn parent_id_defaults_to_none(#[case] json: &str) {
        let result = parse_session_list(json).unwrap();

        assert_eq!(
            result,
            vec![SessionTasks {
                session_id: "session-1".to_string(),
                tasks: vec![TqTask {
                    id: "task-uuid-1".to_string(),
                    number: 1,
                    title: "Task".to_string(),
                    parent_id: None,
                }],
            }]
        );
    }

    #[test]
    fn ignores_unknown_fields() {
        // Mirrors the full shape of a real `tq session list` element: every
        // field besides `sessionId`/`tasks` (and `tasks[].id/number/title`)
        // is ignored.
        let json = indoc! {r#"
            [
              {
                "id": "agent-session-uuid",
                "provider": "claude_code",
                "sessionId": "session-1",
                "parentSessionId": null,
                "context": "work",
                "cwd": "/path/to/project",
                "label": "Example label",
                "lastMessage": "Example last message",
                "customLabel": null,
                "startedAt": "2026-01-01T00:00:00.000Z",
                "lastActiveAt": "2026-01-01T01:00:00.000Z",
                "endedAt": null,
                "tasks": [
                  { "id": "task-uuid-1", "number": 1, "title": "Task", "status": "in_progress" }
                ]
              }
            ]
        "#};

        let result = parse_session_list(json).unwrap();

        assert_eq!(
            result,
            vec![SessionTasks {
                session_id: "session-1".to_string(),
                tasks: vec![TqTask {
                    id: "task-uuid-1".to_string(),
                    number: 1,
                    title: "Task".to_string(),
                    parent_id: None,
                }],
            }]
        );
    }

    #[test]
    fn invalid_json_is_an_error() {
        let result = parse_session_list("not json");

        assert!(result.is_err());
    }
}
