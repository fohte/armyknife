use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::Duration;

use chrono::{TimeZone, Utc};

use crate::commands::agent::tq_session_tasks::SessionTask;
use crate::commands::agent::types::{Engine, Session, SessionPrContext, SessionStatus};

use super::super::EnrichmentUpdates;
use super::{
    BranchPrKey, PullRequestFetcher, SessionEnrichmentCache, SessionTaskMetadata,
    TaskMetadataFetcher,
};

pub(super) fn cache_with_fetchers(
    updates: EnrichmentUpdates,
    fetch_timeout: Duration,
    task_fetcher: TaskMetadataFetcher,
    pull_request_fetcher: PullRequestFetcher,
) -> SessionEnrichmentCache {
    SessionEnrichmentCache::with_fetchers(
        updates,
        fetch_timeout,
        task_fetcher,
        pull_request_fetcher,
    )
}

pub(super) fn metadata(session_ids: HashSet<String>) -> SessionTaskMetadata {
    let links = session_ids
        .into_iter()
        .map(|session_id| {
            let task_id = format!("task-{session_id}");
            (session_id, vec![linked_task(&task_id)])
        })
        .collect();
    SessionTaskMetadata {
        links,
        priorities: HashMap::new(),
        priority_order: HashMap::new(),
        task_projects: HashMap::new(),
    }
}

pub(super) fn linked_task(task_id: &str) -> SessionTask {
    SessionTask {
        task_id: task_id.to_string(),
        task_number: 7,
        task_title: "Example task".to_string(),
        parent_task_id: None,
        is_closed: false,
    }
}

pub(super) fn session(session_id: &str, pr_context: Option<SessionPrContext>) -> Session {
    let timestamp = Utc.timestamp_opt(1_234_567, 0).single().unwrap();
    Session {
        session_id: session_id.to_string(),
        archived_at: None,
        pr_context,
        crit_urls: Vec::new(),
        web_uis: Vec::new(),
        pending_human_review_ids: Default::default(),
        cwd: PathBuf::from("/workspace/example"),
        transcript_path: None,
        tty: None,
        tmux_info: None,
        status: SessionStatus::Stopped,
        agent_status: None,
        agent_status_clear_pending: false,
        created_at: timestamp,
        updated_at: timestamp,
        last_message: None,
        current_tool: None,
        current_tool_name: None,
        label: None,
        work_type: None,
        vibe_enabled: false,
        launched_with_vibe: false,
        work_type_pinned: false,
        child_launch: Default::default(),
        ancestor_session_ids: Vec::new(),
        pending_bg_task_ids: Default::default(),
        pending_agent_task_ids: Default::default(),
        pending_permission_agent_ids: Default::default(),
        pending_permission_request_ids: Default::default(),
        read_at: None,
        sweep_signaled: false,
        stop_blocked_by_armyknife: false,
        engine: Engine::Claude,
        model: None,
        effort: None,
    }
}

pub(super) fn session_with_branch(session_id: &str) -> Session {
    session_with_branch_name(session_id, "feature/example")
}

pub(super) fn session_with_branch_name(session_id: &str, branch: &str) -> Session {
    session(
        session_id,
        Some(SessionPrContext {
            owner: "example-owner".to_string(),
            repo: "example-repository".to_string(),
            branch: branch.to_string(),
            checklist_item_id: None,
        }),
    )
}

pub(super) async fn wait_for_refresh(receiver: &mut tokio::sync::watch::Receiver<()>) {
    tokio::time::timeout(Duration::from_secs(1), receiver.changed())
        .await
        .unwrap()
        .unwrap();
}

pub(super) fn branch_key(branch: &str) -> BranchPrKey {
    (
        "example-owner".to_string(),
        "example-repository".to_string(),
        branch.to_string(),
    )
}
