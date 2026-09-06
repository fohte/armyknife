use std::path::Path;

use crate::commands::cc::store as cc_store;
use crate::infra::git::get_repo_root_in;
use crate::infra::tmux;
use crate::shared::env_var::EnvVars;

/// Repo root of the invoking Claude Code session, via `ARMYKNIFE_SESSION_ID`
/// in the session store; falls back to `current_dir` if untracked.
/// `$TMUX_PANE` won't do -- `a cc new` may run in the background.
pub(super) fn caller_repo_root(current_dir: &str) -> String {
    EnvVars::load()
        .session_id
        .and_then(|id| cc_store::load_session(&id).ok().flatten())
        .and_then(|session| get_repo_root_in(&session.cwd).ok())
        .or_else(|| get_repo_root_in(Path::new(current_dir)).ok())
        .unwrap_or_else(|| current_dir.to_string())
}

/// Whether the target and caller repos map to different tmux sessions.
/// Compares session names, not raw paths, so a worktree checkout of the
/// caller's own repo still counts as the same target.
pub(super) fn should_open_window(
    target_repo_root: &str,
    caller_repo_root: &str,
    worktrees_dir: &str,
) -> bool {
    tmux::get_session_name(target_repo_root, worktrees_dir)
        != tmux::get_session_name(caller_repo_root, worktrees_dir)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    use crate::shared::testing::TestRepo;

    #[test]
    fn caller_repo_root_resolves_subdir_fallback_to_repo_root() {
        let repo = TestRepo::new();
        let subdir = repo.path().join("subdir");
        std::fs::create_dir(&subdir).unwrap();

        temp_env::with_vars([("ARMYKNIFE_SESSION_ID", None::<&str>)], || {
            let result = caller_repo_root(subdir.to_str().unwrap());
            assert_eq!(result, repo.path().to_string_lossy());
        });
    }

    #[rstest]
    #[case::same_repo("/home/user/repo", "/home/user/repo", false)]
    #[case::different_repo("/home/user/repo-a", "/home/user/repo-b", true)]
    #[case::worktree_of_same_repo(
        "/home/user/repo",
        "/home/user/repo/.worktrees/some-branch",
        false
    )]
    #[case::worktree_of_different_repo(
        "/home/user/repo-a",
        "/home/user/repo-b/.worktrees/some-branch",
        true
    )]
    fn should_open_window_cases(
        #[case] target_repo_root: &str,
        #[case] caller_repo_root: &str,
        #[case] expected: bool,
    ) {
        assert_eq!(
            should_open_window(target_repo_root, caller_repo_root, ".worktrees"),
            expected
        );
    }
}
