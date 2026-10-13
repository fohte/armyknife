use std::path::Path;

use crate::commands::agent::types::SessionPrContext;
use crate::infra::git::{GitRepo, github_owner_and_repo};

pub(super) type Capture = fn(&Path) -> Option<SessionPrContext>;

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Resolution {
    Context(SessionPrContext),
    ChecklistOnly(String),
    NotApplicable,
    Unavailable,
}

pub(super) fn capture(cwd: &Path) -> Option<SessionPrContext> {
    let repo = GitRepo::open_at(cwd).ok()?;
    match resolve(&repo) {
        Resolution::Context(context) => Some(context),
        Resolution::ChecklistOnly(_) | Resolution::NotApplicable | Resolution::Unavailable => None,
    }
}

pub(super) fn resolve(repo: &GitRepo) -> Resolution {
    if !repo.is_worktree() {
        return Resolution::NotApplicable;
    }

    let branch = match repo.current_branch() {
        Ok(branch) => branch,
        Err(_) => return Resolution::Unavailable,
    };
    if branch == "HEAD" {
        return Resolution::NotApplicable;
    }

    let checklist_item_id = repo.local_config_value(&branch_checklist_item_config_key(&branch));
    match github_owner_and_repo(repo) {
        Ok((owner, repo_name)) => Resolution::Context(SessionPrContext {
            owner,
            repo: repo_name,
            branch,
            checklist_item_id,
        }),
        Err(_) => checklist_item_id
            .map(Resolution::ChecklistOnly)
            .unwrap_or(Resolution::Unavailable),
    }
}

fn branch_checklist_item_config_key(branch: &str) -> String {
    format!("branch.{branch}.x-tq-checklist-item-id")
}

#[cfg(test)]
pub(super) fn fixed_context_capture(_: &Path) -> Option<SessionPrContext> {
    Some(SessionPrContext {
        owner: "sample-owner".to_string(),
        repo: "sample-repo".to_string(),
        branch: "sample-branch".to_string(),
        checklist_item_id: Some("sample-item".to_string()),
    })
}
