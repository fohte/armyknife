use std::path::{Path, PathBuf};

use super::types::Session;

pub(crate) fn repo_name(cwd: &Path) -> String {
    crate::infra::git::get_repo_root_in(cwd)
        .ok()
        .map(PathBuf::from)
        .unwrap_or_else(|| cwd.to_path_buf())
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("unknown")
        .to_string()
}

pub(crate) fn display_label(session: Option<&Session>, session_id: &str) -> String {
    session
        .and_then(|session| session.label.as_deref())
        .filter(|label| !label.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| session_id.chars().take(8).collect())
}
