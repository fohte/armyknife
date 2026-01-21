use thiserror::Error;

use crate::commands::name_branch;
use crate::infra::git::GitError;

#[derive(Error, Debug)]
pub enum WmError {
    #[error("Not in a git repository")]
    NotInGitRepo,

    #[error("Worktree not found: {0}")]
    WorktreeNotFound(String),

    #[error("Cancelled: no prompt provided")]
    Cancelled,

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON parse error: {0}")]
    JsonParse(#[from] serde_json::Error),

    #[error("Git error: {0}")]
    Git(#[from] GitError),

    #[error("Branch name generation failed: {0}")]
    NameBranch(#[from] name_branch::Error),
}

pub type Result<T> = anyhow::Result<T>;
