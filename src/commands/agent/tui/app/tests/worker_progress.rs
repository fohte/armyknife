use std::collections::HashMap;

use super::{App, create_test_app, create_test_session};
use crate::commands::agent::tui::progress::{FetchedDelegate, FetchedProgress};
use crate::commands::agent::tui::progress_batches::{
    ImplementationPlan, build_implementation_plan,
};
use crate::commands::agent::tui::session_rows::SessionTask;
use crate::commands::agent::tui::tq_snapshot::TqSnapshot;
use crate::commands::agent::types::{Session, SessionPrContext};
use crate::infra::tq::{TqChecklist, TqChecklistItem, TqGithubLink};
use rstest::{fixture, rstest};

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

struct LinkedWorkerContext {
    app: App,
    selected: Session,
    worker: Session,
}

impl LinkedWorkerContext {
    fn apply_progress_with_link(&mut self, linked_state: &str) {
        self.app.apply_progress_result(
            &self.selected.session_id,
            Ok(FetchedProgress {
                selected_session_id: self.selected.session_id.clone(),
                request_scope: None,
                delegated: vec![fetched_worker(self.worker.clone())],
                implementation: Some(checklist_plan_with_link_state(
                    linked_state,
                    fetched_worker(self.worker.clone()),
                )),
                github_error: None,
            }),
        );
    }
}

#[fixture]
fn linked_worker_context() -> LinkedWorkerContext {
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
    LinkedWorkerContext {
        app,
        selected,
        worker,
    }
}

#[rstest]
#[case::open_link("open", "PR open")]
#[case::closed_link("closed", "closed")]
#[case::merged_link("merged", "merged")]
fn keeps_linked_worker_stage_after_its_plan_rows_close(
    #[case] linked_state: &str,
    #[case] expected_stage: &str,
    mut linked_worker_context: LinkedWorkerContext,
) {
    linked_worker_context.apply_progress_with_link(linked_state);
    linked_worker_context.app.progress_rows.clear();
    linked_worker_context
        .app
        .apply_worker_pr_progress_result(Ok(vec![fetched_worker(
            linked_worker_context.worker.clone(),
        )]));

    assert_eq!(
        linked_worker_context
            .app
            .worker_pr_stage_for_session(&linked_worker_context.worker)
            .map(|stage| stage.label()),
        Some(expected_stage),
    );
}

#[rstest]
fn drops_linked_stage_when_worker_session_disappears(
    mut linked_worker_context: LinkedWorkerContext,
) {
    linked_worker_context.apply_progress_with_link("closed");
    linked_worker_context
        .app
        .sessions
        .retain(|session| session.session_id != linked_worker_context.worker.session_id);
    linked_worker_context
        .app
        .apply_worker_pr_progress_result(Ok(Vec::new()));

    assert_eq!(
        linked_worker_context
            .app
            .worker_pr_stage_for_session(&linked_worker_context.worker)
            .map(|stage| stage.label()),
        Some("implementing"),
    );
}
