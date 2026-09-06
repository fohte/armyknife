use crate::commands::cc::store as cc_store;
use crate::infra::git::get_repo_root_in;
use crate::infra::tmux;
use crate::shared::env_var::EnvVars;

/// Resolves the repo root of the Claude Code session that invoked this
/// process, used by `should_open_window` to decide whether `a cc new`
/// without `--worktree` should split the caller's pane or open a window in
/// the target repo's session. Looks up `ARMYKNIFE_SESSION_ID` in the session
/// store to find that session's `cwd`, then resolves it to a repo root.
/// Falls back to `current_dir` -- this process's own, unresolved working
/// directory -- when there is no tracked parent session or its cwd doesn't
/// resolve to a repo (e.g. invoked outside Claude Code). `$TMUX_PANE`/tmux's
/// current session can't be used as a fallback instead: `a cc new` may run
/// in the background, where they don't reliably point at the caller.
pub(super) fn caller_repo_root(current_dir: &str) -> String {
    EnvVars::load()
        .session_id
        .and_then(|id| cc_store::load_session(&id).ok().flatten())
        .and_then(|session| get_repo_root_in(&session.cwd).ok())
        .unwrap_or_else(|| current_dir.to_string())
}

/// True when `a cc new` without `--worktree` should open a new tmux window
/// in the target repo's session instead of splitting the caller's pane --
/// i.e. the target repo isn't the one the invoking session belongs to.
/// Compares tmux session *names* rather than the root paths directly, so a
/// worktree checkout of the caller's own repo still counts as the same
/// target (`get_session_name` normalizes a worktree path to its parent
/// repo's session name).
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
