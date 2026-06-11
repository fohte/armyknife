//! Two-phase clean-view input pipeline.
//!
//! Phase 1 ([`build_initial_clean_rows`]) is synchronous and produces
//! placeholder rows from the worktree snapshot so the clean view can
//! render immediately. Phase 2 ([`fetch_clean_inputs`]) opens each
//! repository, batches a single GraphQL query for every worktree, and
//! returns PR-enriched rows that replace the placeholders.

use std::time::Duration;

use chrono::Utc;

use super::clean_view::{CleanRow, CleanRowInput, build_clean_rows};
use super::worktree_view::WorktreeRow;
use crate::commands::cc::auto_pause::parse_duration;
use crate::commands::cc::types::Session;
use crate::infra::git::{GitRepo, github_owner_and_repo, merge_status_from_pr};
use crate::infra::github::{BranchPrQuery, GitHubClient, PrInfo};
use crate::shared::config::load_config;

/// Default `auto_pause.timeout` used when the config file cannot be
/// loaded, matching the `wm clean` / `cc sweep` definition of "active".
fn default_active_timeout() -> Duration {
    load_config()
        .ok()
        .and_then(|c| parse_duration(&c.cc.auto_pause.timeout).ok())
        .unwrap_or_else(|| Duration::from_secs(30 * 60))
}

/// Build placeholder clean rows from the worktree snapshot without
/// hitting GitHub. Used to render the clean view immediately on entry.
pub fn build_initial_clean_rows(rows: Vec<WorktreeRow>, sessions: &[Session]) -> Vec<CleanRow> {
    let inputs: Vec<CleanRowInput> = rows
        .into_iter()
        .map(|row| CleanRowInput {
            row,
            merge_status: None,
            pr_number: None,
            pr_state: None,
            pr_loaded: false,
        })
        .collect();
    build_clean_rows(inputs, sessions, Utc::now(), default_active_timeout())
}

/// Fetch PR statuses for `rows` and build the PR-enriched clean-row
/// list. Pure read-only against GitHub; never touches local state.
pub async fn fetch_clean_inputs(
    rows: Vec<WorktreeRow>,
    sessions: Vec<Session>,
) -> Result<Vec<CleanRow>, String> {
    let mut repo_ids: Vec<Option<(String, String)>> = Vec::with_capacity(rows.len());
    for row in &rows {
        let id = GitRepo::open_at(&row.path)
            .ok()
            .and_then(|repo| github_owner_and_repo(&repo).ok());
        repo_ids.push(id);
    }

    let queries: Vec<BranchPrQuery> = rows
        .iter()
        .zip(repo_ids.iter())
        .filter_map(|(row, id)| {
            let (owner, repo) = id.as_ref()?;
            if row.branch.is_empty() || row.branch == "(unknown)" {
                return None;
            }
            Some(BranchPrQuery {
                owner: owner.clone(),
                repo: repo.clone(),
                branch: row.branch.clone(),
            })
        })
        .collect();

    let pr_map = if queries.is_empty() {
        std::collections::HashMap::new()
    } else {
        let client = GitHubClient::get().map_err(|e| e.to_string())?;
        client
            .get_prs_for_branches_batch(&queries)
            .await
            .map_err(|e| e.to_string())?
    };

    let inputs: Vec<CleanRowInput> = rows
        .into_iter()
        .zip(repo_ids)
        .map(|(row, id)| {
            let pr_info: Option<&PrInfo> = id.as_ref().and_then(|(owner, repo)| {
                pr_map
                    .get(&(owner.clone(), repo.clone(), row.branch.clone()))
                    .and_then(|opt| opt.as_ref())
            });
            let (merge_status, pr_number, pr_state) = match pr_info {
                Some(info) => (
                    Some(merge_status_from_pr(info)),
                    Some(info.number),
                    Some(info.state.clone()),
                ),
                None => (None, None, None),
            };
            CleanRowInput {
                row,
                merge_status,
                pr_number,
                pr_state,
                pr_loaded: true,
            }
        })
        .collect();

    Ok(build_clean_rows(
        inputs,
        &sessions,
        Utc::now(),
        default_active_timeout(),
    ))
}
