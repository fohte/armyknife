use crate::commands::agent::tui::progress::marks::{session_stage_marks, stage_marks};
use crate::commands::agent::tui::progress::{
    CurrentPrStage, FetchedDelegate, PrStageInfo, ProgressOpenTarget, ProgressRow, current_pr_stage,
};
use crate::commands::agent::types::Session;

use super::App;

const WORKER_PR_REFRESH_ERROR_PREFIX: &str = "PR stage refresh failed:";

impl App {
    pub(in crate::commands::agent::tui) fn apply_worker_pr_progress_result(
        &mut self,
        result: Result<Vec<FetchedDelegate>, String>,
    ) {
        let delegates = match result {
            Ok(delegates) => delegates,
            Err(error) => {
                self.set_error(format!("{WORKER_PR_REFRESH_ERROR_PREFIX} {error}"));
                return;
            }
        };
        if self
            .error_message
            .as_deref()
            .is_some_and(|error| error.starts_with(WORKER_PR_REFRESH_ERROR_PREFIX))
        {
            self.clear_error();
        }

        self.worker_pr_progress = delegates
            .into_iter()
            .filter_map(|delegate| {
                let has_checklist_item = delegate
                    .checklist_item_id
                    .as_ref()
                    .or_else(|| {
                        delegate
                            .session
                            .pr_context
                            .as_ref()
                            .and_then(|context| context.checklist_item_id.as_ref())
                    })
                    .is_some();
                (has_checklist_item || delegate.pull_request.is_some())
                    .then(|| (delegate.session.session_id.clone(), delegate))
            })
            .collect();
    }

    pub(in crate::commands::agent::tui) fn worker_pr_stage_for_session(
        &self,
        session: &Session,
    ) -> Option<CurrentPrStage> {
        let delegate = self.worker_pr_progress.get(&session.session_id);
        let pull_request = delegate.and_then(|delegate| delegate.pull_request.as_ref());
        let linked_pr_stage_info = self.linked_pr_stage_info(&session.session_id);
        let pr_stage_info = pull_request
            .map(PrStageInfo::from_progress)
            .or(linked_pr_stage_info);
        let has_checklist_item = delegate.is_some_and(|delegate| {
            delegate.checklist_item_id.is_some()
                || delegate
                    .session
                    .pr_context
                    .as_ref()
                    .and_then(|context| context.checklist_item_id.as_ref())
                    .is_some()
        }) || session
            .pr_context
            .as_ref()
            .and_then(|context| context.checklist_item_id.as_ref())
            .is_some();

        if delegate.is_none() && !has_checklist_item && pr_stage_info.is_none() {
            return None;
        }

        let stages = match pull_request {
            Some(pull_request) => stage_marks(session, Some(pull_request), &self.agent_config),
            None => session_stage_marks(session, pr_stage_info.is_some(), &self.agent_config),
        };
        Some(current_pr_stage(stages, pr_stage_info.as_ref()))
    }

    fn linked_pr_stage_info(&self, session_id: &str) -> Option<PrStageInfo> {
        self.progress_rows
            .iter()
            .find_map(|progress_row| match progress_row {
                ProgressRow::ChecklistPullRequest { row, .. } | ProgressRow::PullRequest(row)
                    if row.session_id == session_id =>
                {
                    row.pr_stage_info
                }
                _ => None,
            })
    }

    pub(in crate::commands::agent::tui) fn worker_pr_open_target(
        &self,
        session_id: &str,
    ) -> Option<ProgressOpenTarget> {
        self.worker_pr_progress
            .get(session_id)
            .and_then(|delegate| delegate.pull_request.as_ref())
            .map(|progress| ProgressOpenTarget::OpenUrl(progress.pull_request.url.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::agent::types::{Engine, SessionPrContext, SessionStatus};
    use crate::infra::github::{PrCiState, PrInfo, PrProgressInfo, PrState};
    use chrono::{TimeZone, Utc};
    use rstest::rstest;
    use std::collections::BTreeSet;
    use std::path::PathBuf;

    fn session(id: &str, checklist_item_id: Option<&str>) -> Session {
        let now = Utc.with_ymd_and_hms(2026, 10, 1, 0, 0, 0).single().unwrap();
        Session {
            archived_at: None,
            pr_context: checklist_item_id.map(|checklist_item_id| SessionPrContext {
                owner: "sample-owner".to_string(),
                repo: "sample-repo".to_string(),
                branch: "sample-branch".to_string(),
                checklist_item_id: Some(checklist_item_id.to_string()),
            }),
            session_id: id.to_string(),
            work_type: None,
            work_type_pinned: false,
            vibe_enabled: false,
            launched_with_vibe: false,
            child_launch: Default::default(),
            crit_urls: Vec::new(),
            pending_human_review_ids: BTreeSet::new(),
            cwd: PathBuf::from("/project"),
            transcript_path: None,
            tty: None,
            tmux_info: None,
            status: SessionStatus::Running,
            created_at: now,
            updated_at: now,
            last_message: None,
            current_tool: None,
            label: None,
            ancestor_session_ids: Vec::new(),
            pending_bg_task_ids: BTreeSet::new(),
            pending_agent_task_ids: BTreeSet::new(),
            pending_permission_agent_ids: BTreeSet::new(),
            pending_permission_request_ids: Default::default(),
            read_at: None,
            sweep_signaled: false,
            stop_blocked_by_armyknife: false,
            agent_status: None,
            agent_status_clear_pending: false,
            engine: Engine::Claude,
            model: None,
            effort: None,
        }
    }

    #[test]
    fn applies_pr_stages_by_worker_without_changing_the_session_rows() {
        let mut app = App::with_sessions(vec![
            session("design", None),
            session("worker", Some("item-1")),
        ]);
        let before = app
            .build_display_rows()
            .iter()
            .map(|row| row.row_key())
            .collect::<Vec<_>>();
        app.apply_worker_pr_progress_result(Ok(vec![FetchedDelegate {
            session: session("worker", Some("item-1")),
            checklist_item_id: Some("item-1".to_string()),
            pull_request: Some(PrProgressInfo {
                pull_request: PrInfo {
                    number: 17,
                    title: "Example change".to_string(),
                    state: PrState::Open,
                    url: "https://example.test/pull/17".to_string(),
                },
                ci_state: PrCiState::InProgress,
            }),
            lookup_unavailable: false,
        }]));
        let after = app
            .build_display_rows()
            .iter()
            .map(|row| row.row_key())
            .collect::<Vec<_>>();
        let mut current_session = session("worker", Some("item-1"));
        current_session.crit_urls = vec!["https://example.test/review".to_string()];

        assert_eq!(
            (
                before,
                after,
                app.worker_pr_stage_for_session(&session("worker", Some("item-1")))
                    .map(CurrentPrStage::label),
                app.worker_pr_stage_for_session(&current_session)
                    .map(CurrentPrStage::label),
                app.worker_pr_open_target("worker"),
            ),
            (
                vec![
                    None,
                    Some(
                        crate::commands::agent::tui::session_rows::SessionRowKey::Task(
                            "design".to_string()
                        )
                    ),
                    Some(
                        crate::commands::agent::tui::session_rows::SessionRowKey::Task(
                            "worker".to_string()
                        )
                    ),
                ],
                vec![
                    None,
                    Some(
                        crate::commands::agent::tui::session_rows::SessionRowKey::Task(
                            "design".to_string()
                        )
                    ),
                    Some(
                        crate::commands::agent::tui::session_rows::SessionRowKey::Task(
                            "worker".to_string()
                        )
                    ),
                ],
                Some("CI running"),
                Some("CI running"),
                Some(ProgressOpenTarget::OpenUrl(
                    "https://example.test/pull/17".to_string()
                )),
            ),
        );
    }

    #[test]
    fn reports_worker_pr_fetch_failures_in_the_error_banner() {
        let mut app = App::with_sessions(Vec::new());

        app.apply_worker_pr_progress_result(Err("GitHub is unavailable".to_string()));

        assert_eq!(
            app.error_message.as_deref(),
            Some("PR stage refresh failed: GitHub is unavailable"),
        );
    }

    #[rstest]
    #[case::with_checklist(Some("item-1"), Some("implementing"))]
    #[case::without_checklist(None, None)]
    fn derives_worker_stage_before_the_batch_fetch(
        #[case] checklist_item_id: Option<&str>,
        #[case] expected: Option<&str>,
    ) {
        let app = App::with_sessions(vec![session("worker", checklist_item_id)]);
        let current = app.sessions[0].clone();

        assert_eq!(
            app.worker_pr_stage_for_session(&current)
                .map(CurrentPrStage::label),
            expected,
        );
    }
}
