mod model;
mod plan;
mod render;
mod storage;
mod sync;

#[cfg(test)]
pub(super) use model::PrSeriesEntry;
pub(super) use model::{
    CreatedPullRequest, PrSeriesPlan, Repository, resolve_created_pull_requests,
    validate_checklist_leaf_ids,
};
pub(super) use render::preserve_managed_section;
pub(super) use storage::load_plan_if_present;

pub(super) async fn sync_after_submit(
    plan: &PrSeriesPlan,
    created_pull_requests: &[CreatedPullRequest],
    github: &(impl crate::infra::github::PrBodyClient + crate::infra::github::RepoClient),
) -> anyhow::Result<usize> {
    sync::sync_after_submit(plan, created_pull_requests, github).await
}

#[cfg(test)]
pub(super) fn synchronized_body_for_test(
    plan: &PrSeriesPlan,
    entry_id: &str,
    body: &str,
    repository_privacy: &std::collections::HashMap<String, bool>,
    created_pull_requests: &[CreatedPullRequest],
) -> anyhow::Result<String> {
    sync::synchronized_body(
        plan,
        entry_id,
        body,
        repository_privacy,
        created_pull_requests,
    )
}

use clap::Subcommand;

#[derive(Subcommand, Clone, PartialEq, Eq)]
pub enum PrSeriesCommands {
    /// Register or replace a task's validated PR series plan
    Set(plan::SetArgs),

    /// Synchronize the plan section in linked pull request bodies
    Sync(sync::SyncArgs),
}

impl PrSeriesCommands {
    pub async fn run(&self) -> anyhow::Result<()> {
        match self {
            Self::Set(args) => plan::run(args).await,
            Self::Sync(args) => sync::run(args).await,
        }
    }
}
