use std::collections::HashMap;

use super::session_rows::SessionTask;
use super::tq_snapshot::TqSnapshot;
use crate::commands::agent::types::{TMUX_SESSION_OPTION, TMUX_SESSION_OPTION_LEGACY};
use crate::infra::tmux::{self, PaneInfoWithOption};

const TMUX_TQ_TASK_OPTION: &str = "@armyknife-tq-task";
const TMUX_TQ_CLOSED_OPTION: &str = "@armyknife-tq-closed";

pub(super) fn sync(snapshot: &TqSnapshot) {
    let panes = tmux::list_all_panes_with_option(TMUX_SESSION_OPTION, TMUX_SESSION_OPTION_LEGACY);
    let commands = tmux_option_commands(&panes, &snapshot.session_tasks);
    let _ = tmux::run_batch(&commands);
}

fn tmux_option_commands(
    panes: &[PaneInfoWithOption],
    session_tasks: &HashMap<String, Vec<SessionTask>>,
) -> Vec<Vec<String>> {
    let mut commands = Vec::with_capacity(panes.len() * 2);
    for pane in panes {
        let Some(session_id) = pane.option_value.as_deref() else {
            continue;
        };
        let task = session_tasks
            .get(session_id)
            .and_then(|tasks| tasks.first());
        let task_value = task.map(|task| {
            format!(
                "##{} {}",
                task.task_number,
                tmux::escape_format_value(&task.task_title)
            )
        });
        commands.push(pane_option_command(
            &pane.pane_id,
            TMUX_TQ_TASK_OPTION,
            task_value.as_deref(),
        ));
        commands.push(pane_option_command(
            &pane.pane_id,
            TMUX_TQ_CLOSED_OPTION,
            task.filter(|task| task.is_closed).map(|_| "1"),
        ));
    }
    commands
}

fn pane_option_command(pane_id: &str, option: &str, value: Option<&str>) -> Vec<String> {
    match value {
        Some(value) => vec![
            "set-option".to_string(),
            "-p".to_string(),
            "-t".to_string(),
            pane_id.to_string(),
            option.to_string(),
            value.to_string(),
        ],
        None => vec![
            "set-option".to_string(),
            "-p".to_string(),
            "-u".to_string(),
            "-t".to_string(),
            pane_id.to_string(),
            option.to_string(),
        ],
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn pane(pane_id: &str, session_id: &str) -> PaneInfoWithOption {
        PaneInfoWithOption {
            session_name: "main".to_string(),
            window_index: 0,
            pane_index: 0,
            pane_id: pane_id.to_string(),
            option_value: Some(session_id.to_string()),
        }
    }

    fn task(task_id: &str, task_number: u32, task_title: &str, is_closed: bool) -> SessionTask {
        SessionTask {
            task_id: task_id.to_string(),
            task_number,
            task_title: task_title.to_string(),
            parent_task_id: None,
            is_closed,
        }
    }

    #[test]
    fn tmux_options_use_latest_task_escape_title_and_clear_missing_tasks() {
        let panes = vec![
            pane("%1", "session-a"),
            pane("%2", "session-a"),
            pane("%3", "session-b"),
            pane("%4", "session-c"),
            pane("%5", "session-d"),
        ];
        let session_tasks = HashMap::from([
            (
                "session-a".to_string(),
                vec![
                    task("new", 742, "Catalog #2\t#[fg=red]", true),
                    task("old", 301, "Previous", false),
                ],
            ),
            ("session-c".to_string(), Vec::new()),
            (
                "session-d".to_string(),
                vec![task("open", 600, "Open item", false)],
            ),
        ]);

        assert_eq!(
            tmux_option_commands(&panes, &session_tasks),
            vec![
                vec![
                    "set-option",
                    "-p",
                    "-t",
                    "%1",
                    "@armyknife-tq-task",
                    "##742 Catalog ##2##[fg=red]",
                ],
                vec!["set-option", "-p", "-t", "%1", "@armyknife-tq-closed", "1",],
                vec![
                    "set-option",
                    "-p",
                    "-t",
                    "%2",
                    "@armyknife-tq-task",
                    "##742 Catalog ##2##[fg=red]",
                ],
                vec!["set-option", "-p", "-t", "%2", "@armyknife-tq-closed", "1",],
                vec!["set-option", "-p", "-u", "-t", "%3", "@armyknife-tq-task",],
                vec!["set-option", "-p", "-u", "-t", "%3", "@armyknife-tq-closed",],
                vec!["set-option", "-p", "-u", "-t", "%4", "@armyknife-tq-task",],
                vec!["set-option", "-p", "-u", "-t", "%4", "@armyknife-tq-closed",],
                vec![
                    "set-option",
                    "-p",
                    "-t",
                    "%5",
                    "@armyknife-tq-task",
                    "##600 Open item",
                ],
                vec!["set-option", "-p", "-u", "-t", "%5", "@armyknife-tq-closed",],
            ],
        );
    }
}
