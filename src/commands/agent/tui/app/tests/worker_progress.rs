use std::collections::HashMap;

use super::{create_test_app, create_test_session};
use crate::commands::agent::tui::progress::{FetchedDelegate, FetchedProgress};
use crate::commands::agent::tui::progress_batches::{
    ImplementationPlan, build_implementation_plan,
};
use crate::commands::agent::tui::session_rows::SessionTask;
use crate::commands::agent::tui::tq_snapshot::TqSnapshot;
use crate::commands::agent::types::{Session, SessionPrContext};
use crate::infra::tq::{TqChecklist, TqChecklistItem, TqGithubLink};
use rstest::rstest;

fn task_link() -> SessionTask {
    SessionTask {
        task_id: "task-sample".to_string(),
        task_number: 1,
        task_title: "Sample task".to_string(),
        parent_task_id: None,
        is_closed: false,
    }
}

fn fetched_worker(session: Session) -> FetchedDelegate {
    FetchedDelegate {
        session,
        checklist_item_id: Some("item-sample".to_string()),
        pull_request: None,
        lookup_unavailable: true,
    }
}

fn checklist_plan_with_link_state(state: &str, worker: FetchedDelegate) -> ImplementationPlan {
    let checklist = TqChecklist {
        id: "checklist-sample".to_string(),
        name: "実装".to_string(),
        items: vec![TqChecklistItem {
            id: "batch-sample".to_string(),
            content: "Batch 1".to_string(),
            checked_at: None,
            github_link_id: None,
            children: vec![TqChecklistItem {
                id: "item-sample".to_string(),
                content: "PR 1: Sample change".to_string(),
                checked_at: None,
                github_link_id: Some("link-sample".to_string()),
                children: Vec::new(),
            }],
        }],
    };
    let github_link = TqGithubLink {
        id: "link-sample".to_string(),
        kind: Some("pull_request".to_string()),
        owner: Some("sample-owner".to_string()),
        repo: Some("sample-project".to_string()),
        number: Some(17),
        title: Some("Sample change".to_string()),
        url: Some("https://example.test/pull/17".to_string()),
        state: Some(state.to_string()),
    };

    build_implementation_plan(&checklist, &[github_link], &[worker], &HashMap::new(), true)
}

#[rstest]
#[case::closed_link("closed", "closed")]
#[case::merged_link("merged", "merged")]
fn keeps_linked_worker_stage_after_its_plan_rows_close(
    #[case] linked_state: &str,
    #[case] expected_stage: &str,
) {
    let selected = create_test_session("selected");
    let mut worker = create_test_session("worker");
    worker.ancestor_session_ids = vec![selected.session_id.clone()];
    worker.pr_context = Some(SessionPrContext {
        owner: "sample-owner".to_string(),
        repo: "sample-project".to_string(),
        branch: "sample-branch".to_string(),
        checklist_item_id: Some("item-sample".to_string()),
    });
    let mut app = create_test_app(vec![selected.clone(), worker.clone()]);
    app.set_tq_snapshot(TqSnapshot::new(HashMap::from([
        (selected.session_id.clone(), vec![task_link()]),
        (worker.session_id.clone(), vec![task_link()]),
    ])));

    app.apply_progress_result(
        &selected.session_id,
        Ok(FetchedProgress {
            selected_session_id: selected.session_id.clone(),
            request_scope: None,
            delegated: vec![fetched_worker(worker.clone())],
            implementation: Some(checklist_plan_with_link_state(
                linked_state,
                fetched_worker(worker.clone()),
            )),
            github_error: None,
        }),
    );
    app.progress_rows.clear();
    app.apply_worker_pr_progress_result(Ok(vec![fetched_worker(worker.clone())]));

    assert_eq!(
        app.worker_pr_stage_for_session(&worker)
            .map(|stage| stage.label()),
        Some(expected_stage),
    );
}
