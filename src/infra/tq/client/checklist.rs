use serde::{Deserialize, Serialize};

use super::{Result, TQ_COMMAND_TIMEOUT, TqClient, TqError, process};
use crate::infra::external_tool::ExternalTool;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
/// A tq checklist with its top-level items.
pub struct TqChecklist {
    /// Stable checklist identifier.
    pub id: String,
    /// Checklist name shown in tq.
    pub name: String,
    #[serde(default)]
    /// Top-level checklist items.
    pub items: Vec<TqChecklistItem>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
/// One checklist item and its nested children.
pub struct TqChecklistItem {
    /// Stable item identifier.
    pub id: String,
    /// Item text.
    pub content: String,
    #[serde(default)]
    /// Timestamp set when the item was checked.
    pub checked_at: Option<String>,
    #[serde(default)]
    /// Linked GitHub record identifier, if one exists.
    pub github_link_id: Option<String>,
    #[serde(default)]
    /// Child checklist items.
    pub children: Vec<TqChecklistItem>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
/// GitHub metadata linked from a tq task.
pub struct TqGithubLink {
    /// Stable link identifier.
    pub id: String,
    #[serde(default)]
    /// Link kind, such as pull_request.
    pub kind: Option<String>,
    #[serde(default)]
    /// GitHub repository owner.
    pub owner: Option<String>,
    #[serde(default)]
    /// GitHub repository name.
    pub repo: Option<String>,
    #[serde(default)]
    /// Pull request or issue number.
    pub number: Option<u64>,
    #[serde(default)]
    /// Linked GitHub title.
    pub title: Option<String>,
    #[serde(default)]
    /// Linked GitHub URL.
    pub url: Option<String>,
    #[serde(default)]
    /// Last synchronized GitHub state.
    pub state: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TaskGithubLinks {
    #[serde(default)]
    github_links: Vec<TqGithubLink>,
}

impl TqClient {
    /// Lists all checklists attached to a task.
    pub async fn list_checklists(&self, task_id: &str) -> Result<Vec<TqChecklist>> {
        let args = build_checklist_list_args(task_id);
        let task_id = task_id.to_string();
        let join_result = tokio::task::spawn_blocking(move || run_checklist_list(&task_id)).await;
        match join_result {
            Ok(result) => result,
            Err(error) => Err(TqError::command_failed(
                &args,
                format!("task panicked: {error}"),
                None,
            )),
        }
    }

    /// Lists GitHub links attached to a task.
    pub async fn task_github_links(&self, task_id: &str) -> Result<Vec<TqGithubLink>> {
        let args = build_task_get_args(task_id);
        let task_id = task_id.to_string();
        let join_result =
            tokio::task::spawn_blocking(move || run_task_github_links(&task_id)).await;
        match join_result {
            Ok(result) => result,
            Err(error) => Err(TqError::command_failed(
                &args,
                format!("task panicked: {error}"),
                None,
            )),
        }
    }
}

fn build_checklist_list_args(task_id: &str) -> Vec<String> {
    vec![
        "checklist".to_string(),
        "list".to_string(),
        task_id.to_string(),
    ]
}

fn build_task_get_args(task_id: &str) -> Vec<String> {
    vec!["task".to_string(), "get".to_string(), task_id.to_string()]
}

fn run_checklist_list(task_id: &str) -> Result<Vec<TqChecklist>> {
    run_tq_json(&build_checklist_list_args(task_id))
}

fn run_task_github_links(task_id: &str) -> Result<Vec<TqGithubLink>> {
    let task: TaskGithubLinks = run_tq_json(&build_task_get_args(task_id))?;
    Ok(task.github_links)
}

fn run_tq_json<T: serde::de::DeserializeOwned>(args: &[String]) -> Result<T> {
    let mut command = ExternalTool::Tq.command();
    command.args(args);

    let output = process::run_with_timeout(command, TQ_COMMAND_TIMEOUT)
        .map_err(|error| TqError::command_failed(args, error.to_string(), None))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(TqError::command_failed(
            args,
            "command exited with non-zero status",
            Some(stderr),
        ));
    }

    Ok(serde_json::from_slice(&output.stdout)?)
}

#[cfg(test)]
mod tests {
    use indoc::indoc;

    use super::*;

    #[test]
    fn parses_nested_checklist_items_and_optional_progress_fields() {
        let json = indoc! {r#"
            [
              {
                "id": "checklist-1",
                "name": "Implementation",
                "items": [
                  {
                    "id": "batch-1",
                    "content": "Batch 1",
                    "children": [
                      {
                        "id": "item-1",
                        "content": "PR 1: Add a sample feature",
                        "checkedAt": null,
                        "githubLinkId": "link-1",
                        "children": []
                      }
                    ]
                  }
                ]
              }
            ]
        "#};

        assert_eq!(
            serde_json::from_str::<Vec<TqChecklist>>(json).unwrap(),
            vec![TqChecklist {
                id: "checklist-1".to_string(),
                name: "Implementation".to_string(),
                items: vec![TqChecklistItem {
                    id: "batch-1".to_string(),
                    content: "Batch 1".to_string(),
                    checked_at: None,
                    github_link_id: None,
                    children: vec![TqChecklistItem {
                        id: "item-1".to_string(),
                        content: "PR 1: Add a sample feature".to_string(),
                        checked_at: None,
                        github_link_id: Some("link-1".to_string()),
                        children: Vec::new(),
                    }],
                }],
            }],
        );
    }

    #[test]
    fn parses_github_links_needed_to_resolve_checklist_items() {
        let json = indoc! {r#"
            {
              "githubLinks": [
                {
                  "id": "link-1",
                  "kind": "pull_request",
                  "owner": "sample-org",
                  "repo": "sample-project",
                  "number": 17,
                  "title": "Add a sample feature",
                  "url": "https://github.com/sample-org/sample-project/pull/17",
                  "state": "open"
                }
              ]
            }
        "#};

        assert_eq!(
            serde_json::from_str::<TaskGithubLinks>(json)
                .unwrap()
                .github_links,
            vec![TqGithubLink {
                id: "link-1".to_string(),
                kind: Some("pull_request".to_string()),
                owner: Some("sample-org".to_string()),
                repo: Some("sample-project".to_string()),
                number: Some(17),
                title: Some("Add a sample feature".to_string()),
                url: Some("https://github.com/sample-org/sample-project/pull/17".to_string()),
                state: Some("open".to_string()),
            }],
        );
    }

    #[test]
    fn checklist_command_arguments_include_the_requested_task() {
        assert_eq!(
            build_checklist_list_args("task-1"),
            vec!["checklist", "list", "task-1"],
        );
    }

    #[test]
    fn task_command_arguments_include_the_requested_task() {
        assert_eq!(build_task_get_args("task-2"), vec!["task", "get", "task-2"],);
    }
}
