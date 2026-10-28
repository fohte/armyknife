use anyhow::Result;

use super::pr_series::{
    model::{PrSeriesPlan, Repository, resolve_created_pull_requests, validate_checklist_leaf_ids},
    storage, sync,
};
use crate::infra::github::{PrBodyClient, RepoClient};
use crate::infra::tq::{TqChecklist, TqClient, TqTaskDetails};

pub(super) struct SubmittedPrContext<'a> {
    pub(super) task_id: &'a str,
    pub(super) checklist_item_id: &'a str,
    pub(super) owner: &'a str,
    pub(super) repo: &'a str,
    pub(super) url: &'a str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SubmitSyncStage {
    Prepare,
    Link,
    Sync,
}

#[derive(Debug)]
pub(super) struct SubmitSyncFailure {
    pub(super) stage: SubmitSyncStage,
    pub(super) error: anyhow::Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SubmitSyncOutcome {
    NoPlan,
    Synchronized,
}

impl SubmitSyncFailure {
    fn new(stage: SubmitSyncStage, error: impl Into<anyhow::Error>) -> Self {
        Self {
            stage,
            error: error.into(),
        }
    }
}

pub(super) async fn link_and_sync_after_submit(
    github: &(impl PrBodyClient + RepoClient),
    task_id: &str,
    checklist_item_id: &str,
    owner: &str,
    repo: &str,
    pr_url: &str,
) -> std::result::Result<SubmitSyncOutcome, SubmitSyncFailure> {
    let tq_client = TqClient::detect().ok_or_else(|| {
        SubmitSyncFailure::new(
            SubmitSyncStage::Prepare,
            anyhow::anyhow!("tq CLI is required to synchronize a PR series"),
        )
    })?;
    let context = SubmittedPrContext {
        task_id,
        checklist_item_id,
        owner,
        repo,
        url: pr_url,
    };
    link_and_sync_after_submit_with(&tq_client, storage::load_plan_if_present, github, &context)
        .await
}

pub(super) trait SubmitSyncTqClient {
    async fn task_details(&self, task_id: &str) -> Result<TqTaskDetails>;

    async fn list_checklists(&self, task_id: &str) -> Result<Vec<TqChecklist>>;

    async fn link_checklist_item(&self, item_id: &str, github_url: &str) -> Result<()>;
}

impl SubmitSyncTqClient for TqClient {
    async fn task_details(&self, task_id: &str) -> Result<TqTaskDetails> {
        TqClient::task_details(self, task_id)
            .await
            .map_err(Into::into)
    }

    async fn list_checklists(&self, task_id: &str) -> Result<Vec<TqChecklist>> {
        TqClient::list_checklists(self, task_id)
            .await
            .map_err(Into::into)
    }

    async fn link_checklist_item(&self, item_id: &str, github_url: &str) -> Result<()> {
        TqClient::link_checklist_item(self, item_id, github_url)
            .await
            .map_err(Into::into)
    }
}

pub(super) async fn link_and_sync_after_submit_with(
    tq_client: &impl SubmitSyncTqClient,
    load_plan: impl FnOnce(&str) -> Result<Option<PrSeriesPlan>>,
    github: &(impl PrBodyClient + RepoClient),
    context: &SubmittedPrContext<'_>,
) -> std::result::Result<SubmitSyncOutcome, SubmitSyncFailure> {
    let task = tq_client
        .task_details(context.task_id)
        .await
        .map_err(|error| SubmitSyncFailure::new(SubmitSyncStage::Prepare, error))?;
    let Some(plan) = load_plan(&task.id)
        .map_err(|error| SubmitSyncFailure::new(SubmitSyncStage::Prepare, error))?
    else {
        return Ok(SubmitSyncOutcome::NoPlan);
    };
    let checklists = tq_client
        .list_checklists(&task.id)
        .await
        .map_err(|error| SubmitSyncFailure::new(SubmitSyncStage::Prepare, error))?;
    validate_checklist_leaf_ids(&plan, &checklists)
        .map_err(|error| SubmitSyncFailure::new(SubmitSyncStage::Prepare, error))?;
    let Some(entry) = plan
        .pull_requests
        .iter()
        .find(|entry| entry.tq_leaf_id == context.checklist_item_id)
    else {
        return Err(SubmitSyncFailure::new(
            SubmitSyncStage::Prepare,
            anyhow::anyhow!(
                "checklist item `{}` is not part of the saved PR series plan",
                context.checklist_item_id
            ),
        ));
    };
    let repository = Repository::parse(&entry.repository)
        .map_err(|error| SubmitSyncFailure::new(SubmitSyncStage::Prepare, error))?;
    if !repository.matches(context.owner, context.repo) {
        return Err(SubmitSyncFailure::new(
            SubmitSyncStage::Prepare,
            anyhow::anyhow!(
                "checklist item `{}` belongs to `{}`, not `{}/{}`",
                context.checklist_item_id,
                repository.full_name(),
                context.owner,
                context.repo,
            ),
        ));
    }
    let entry_id = entry.id.clone();

    tq_client
        .link_checklist_item(context.checklist_item_id, context.url)
        .await
        .map_err(|error| SubmitSyncFailure::new(SubmitSyncStage::Link, error))?;

    let task = tq_client
        .task_details(&task.id)
        .await
        .map_err(|error| SubmitSyncFailure::new(SubmitSyncStage::Sync, error))?;
    let checklists = tq_client
        .list_checklists(&task.id)
        .await
        .map_err(|error| SubmitSyncFailure::new(SubmitSyncStage::Sync, error))?;
    let created_pull_requests =
        resolve_created_pull_requests(&plan, &checklists, &task.github_links)
            .map_err(|error| SubmitSyncFailure::new(SubmitSyncStage::Sync, error))?;
    let current_pr_was_linked = created_pull_requests.iter().any(|pull_request| {
        pull_request.entry_id == entry_id && pull_request.url.eq_ignore_ascii_case(context.url)
    });

    let mut report = sync::sync_created_pull_requests(&plan, &created_pull_requests, github)
        .await
        .map_err(|error| SubmitSyncFailure::new(SubmitSyncStage::Sync, error))?;
    if !current_pr_was_linked {
        report.failed.push((
            context.url.to_string(),
            "the checklist link is not visible in tq yet".to_string(),
        ));
    }
    sync::print_report(&report);
    if !report.failed.is_empty() {
        let (stage, message) = if current_pr_was_linked {
            (
                SubmitSyncStage::Sync,
                format!(
                    "{} pull request(s) could not be synchronized",
                    report.failed.len()
                ),
            )
        } else {
            (
                SubmitSyncStage::Link,
                "the checklist link is not visible in tq yet".to_string(),
            )
        };
        return Err(SubmitSyncFailure::new(stage, anyhow::anyhow!(message)));
    }

    Ok(SubmitSyncOutcome::Synchronized)
}
