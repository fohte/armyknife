//! Async PR-status fetch used by the clean view.
//!
//! Given a snapshot of discovered worktrees, opens each repository to
//! resolve its GitHub owner/repo, batches one GraphQL query for all
//! worktrees, and produces [`CleanRowInput`] entries that the
//! `build_clean_rows` partitioning function consumes.

use std::time::Duration;

use chrono::Utc;

use super::clean_view::{CleanRowInput, build_clean_rows};
use super::worktree_view::WorktreeRow;
use crate::commands::cc::auto_pause::parse_duration;
use crate::commands::cc::types::Session;
use crate::infra::git::{GitRepo, github_owner_and_repo, merge_status_from_pr};
use crate::infra::github::{BranchPrQuery, GitHubClient, PrInfo};
use crate::shared::config::load_config;

/// Fetch PR statuses for `rows` and build the partitioned clean-row
/// list. Pure read-only against GitHub; never touches local state.
pub async fn fetch_clean_inputs(
    rows: Vec<WorktreeRow>,
    sessions: Vec<Session>,
) -> Result<Vec<super::clean_view::CleanRow>, String> {
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
            }
        })
        .collect();

    // Match wm clean / cc sweep so "active" means the same thing across
    // commands; without this, a session active per one definition could
    // be torn down per another.
    let timeout = load_config()
        .ok()
        .and_then(|c| parse_duration(&c.cc.auto_pause.timeout).ok())
        .unwrap_or_else(|| Duration::from_secs(30 * 60));

    Ok(build_clean_rows(inputs, &sessions, Utc::now(), timeout))
}
