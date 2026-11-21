use super::{SessionView, TqTaskView};

fn compare_tasks(left: &Option<TqTaskView>, right: &Option<TqTaskView>) -> std::cmp::Ordering {
    match (left, right) {
        (Some(left), Some(right)) => left
            .priority_order
            .unwrap_or(usize::MAX)
            .cmp(&right.priority_order.unwrap_or(usize::MAX)),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    }
}

pub(super) fn sort_session_views(sessions: &mut [SessionView]) {
    sessions.sort_by(|left, right| {
        left.category
            .cmp(&right.category)
            .then_with(|| compare_tasks(&left.task, &right.task))
            .then_with(|| right.updated_at.cmp(&left.updated_at))
            .then_with(|| left.id.cmp(&right.id))
    });
}

#[cfg(test)]
mod tests {
    use super::super::SessionActivityStatus;
    use super::*;
    use crate::commands::agent::types::Engine;
    use crate::commands::agent::watch_web::sessions::category::SessionCategory;
    use chrono::TimeZone;
    use rstest::rstest;

    fn row(
        id: &str,
        category: SessionCategory,
        task: Option<(Option<i32>, Option<usize>, u32)>,
        updated_at: i64,
    ) -> SessionView {
        let task = task.map(|(priority, priority_order, number)| TqTaskView {
            id: format!("task-{number}"),
            number,
            title: id.to_string(),
            priority,
            is_closed: false,
            updated_at: None,
            priority_order,
        });
        SessionView {
            row: super::super::SessionRowView {
                id: id.to_string(),
                engine: Engine::Claude,
                work_type: None,
                category,
                kind: None,
                title: id.to_string(),
                preview: None,
                elapsed_seconds: 0,
                task: task.clone(),
                repo: String::new(),
                vibe_mode: crate::commands::agent::vibe::VibeState::RepoDisabled,
                decision_count: 0,
            },
            is_paused: false,
            activity_status: if category == SessionCategory::InProgress {
                SessionActivityStatus::Working
            } else {
                SessionActivityStatus::Stopped
            },
            waiting_reason: None,
            waiting_for_background_tasks: false,
            category_when_not_later: category,
            updated_at: chrono::Utc.timestamp_opt(updated_at, 0).unwrap(),
            last_prompted_at: None,
            unread: false,
            tasks: task.into_iter().collect(),
            branch: None,
            can_open_in_terminal: false,
            project: "Unsorted".to_string(),
            parent: None,
            has_session_tree: false,
            ancestor_session_ids: Vec::new(),
            launch_id: None,
            pull_request: None,
            pull_requests: Vec::new(),
            crit_url: None,
            is_waiting_for_crit_review: false,
            web_uis: Vec::new(),
            later_until: None,
            is_today: false,
            is_compacting: false,
            unreviewed_decision_count: 0,
            review_document_id: None,
        }
    }

    #[rstest]
    #[case::lower_priority_first(Some((Some(1), Some(0), 1)), Some((Some(0), Some(1), 2)), std::cmp::Ordering::Less)]
    #[case::unset_priority_before_deferred(Some((None, Some(2), 1)), Some((Some(-1), Some(3), 2)), std::cmp::Ordering::Less)]
    #[case::deferred_after_regular(Some((Some(-1), Some(4), 1)), Some((Some(0), Some(2), 2)), std::cmp::Ordering::Greater)]
    #[case::missing_order_after_known(Some((Some(1), None, 1)), Some((Some(-1), Some(4), 2)), std::cmp::Ordering::Greater)]
    #[case::same_priority_ties(Some((Some(1), Some(2), 1)), Some((Some(1), Some(2), 2)), std::cmp::Ordering::Equal)]
    #[case::taskless_last(Some((Some(1), Some(2), 1)), None, std::cmp::Ordering::Less)]
    fn compares_task_priority(
        #[case] left: Option<(Option<i32>, Option<usize>, u32)>,
        #[case] right: Option<(Option<i32>, Option<usize>, u32)>,
        #[case] expected: std::cmp::Ordering,
    ) {
        let to_task = |task: Option<(Option<i32>, Option<usize>, u32)>| {
            task.map(|(priority, priority_order, number)| TqTaskView {
                id: "task".to_string(),
                number,
                title: "task".to_string(),
                priority,
                is_closed: false,
                updated_at: None,
                priority_order,
            })
        };
        assert_eq!(compare_tasks(&to_task(left), &to_task(right)), expected);
    }

    #[rstest]
    #[case::category_then_priority(
        vec![
            row("done", SessionCategory::Done, Some((Some(0), Some(0), 1)), 1),
            row("working", SessionCategory::InProgress, Some((Some(0), Some(0), 2)), 1),
            row("needs_low", SessionCategory::NeedsAction, Some((Some(2), Some(1), 3)), 1),
            row("needs_high", SessionCategory::NeedsAction, Some((Some(1), Some(0), 4)), 1),
            row("needs_without_task", SessionCategory::NeedsAction, None, 0),
        ],
        vec!["needs_high", "needs_low", "needs_without_task", "working", "done"]
    )]
    #[case::unset_priority_between_regular_and_deferred(
        vec![
            row("deferred", SessionCategory::NeedsAction, Some((Some(-1), Some(4), 2)), 1),
            row("inbox", SessionCategory::NeedsAction, Some((None, Some(3), 1)), 1),
            row("regular", SessionCategory::NeedsAction, Some((Some(0), Some(2), 3)), 1),
        ],
        vec!["regular", "inbox", "deferred"]
    )]
    #[case::newest_same_priority_first(
        vec![
            row("newer", SessionCategory::NeedsAction, Some((Some(1), Some(0), 1)), 20),
            row("older", SessionCategory::NeedsAction, Some((Some(1), Some(0), 2)), 10),
        ],
        vec!["newer", "older"]
    )]
    #[case::newest_without_task_first(
        vec![
            row("newer", SessionCategory::NeedsAction, None, 20),
            row("older", SessionCategory::NeedsAction, None, 10),
        ],
        vec!["newer", "older"]
    )]
    #[case::id_breaks_equal_timestamps(
        vec![
            row("session_b", SessionCategory::NeedsAction, Some((Some(1), Some(0), 1)), 20),
            row("session_a", SessionCategory::NeedsAction, Some((Some(1), Some(0), 2)), 20),
        ],
        vec!["session_a", "session_b"]
    )]
    fn orders_session_rows(#[case] mut rows: Vec<SessionView>, #[case] expected_ids: Vec<&str>) {
        sort_session_views(&mut rows);

        assert_eq!(
            rows.into_iter()
                .map(|session| session.row.id)
                .collect::<Vec<_>>(),
            expected_ids
        );
    }
}
