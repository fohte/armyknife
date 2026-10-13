use std::path::Path;

use crate::commands::agent::types::SessionPrContext;
use crate::infra::git::{GitRepo, github_owner_and_repo};

pub(super) fn capture(cwd: &Path) -> Option<SessionPrContext> {
    let repo = GitRepo::open_at(cwd).ok()?;
    if !repo.is_worktree() {
        return None;
    }

    let branch = repo.current_branch().ok()?;
    if branch == "HEAD" {
        return None;
    }

    let (owner, repo_name) = github_owner_and_repo(&repo).ok()?;
    let checklist_item_id =
        repo.local_config_value(&format!("branch.{branch}.x-tq-checklist-item-id"));

    Some(SessionPrContext {
        owner,
        repo: repo_name,
        branch,
        checklist_item_id,
    })
}
