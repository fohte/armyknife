use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::{ProcessSnapshot, list_process_cwds};

/// Returns the pids of the caller's processes whose cwd is inside `path`.
///
/// The caller's processes are this process and its ancestors, plus descendants
/// of `roots` (e.g. panes of the caller's tmux window). Descendants of the
/// caller's pane are ignored when every process from the caller to that pane
/// has a known cwd outside `path` and no caller ancestor has a known cwd inside.
///
/// Returns `None` if `lsof` or `ps` is unavailable, so callers can refuse to
/// act instead of assuming the caller is elsewhere.
pub fn find_caller_pids_in_path(path: &Path, roots: &[u32]) -> Option<Vec<u32>> {
    let cwds = list_process_cwds()?;
    let snapshot = ProcessSnapshot::capture()?;
    // lsof reports fully resolved cwds (e.g. macOS's /tmp -> /private/tmp).
    let resolved_path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    Some(caller_pids_in_path(
        &snapshot,
        &cwds,
        std::process::id(),
        roots,
        super::current_caller_pane_pid(),
        &resolved_path,
    ))
}

fn caller_pids_in_path(
    snapshot: &ProcessSnapshot,
    cwds: &HashMap<u32, PathBuf>,
    self_pid: u32,
    roots: &[u32],
    caller_pane_pid: Option<u32>,
    path: &Path,
) -> Vec<u32> {
    let ignored_descendants =
        snapshot.caller_descendants_outside_path(self_pid, caller_pane_pid, cwds, path);
    let mut caller_pids = snapshot.ancestors(self_pid);
    for &root in roots {
        caller_pids.extend(snapshot.self_and_descendants(root));
    }
    caller_pids.retain(|pid| !ignored_descendants.contains(pid));
    let mut pids: Vec<u32> = cwds
        .iter()
        .filter(|(pid, cwd)| caller_pids.contains(pid) && cwd.starts_with(path))
        .map(|(&pid, _)| pid)
        .collect();
    pids.sort_unstable();
    pids
}

#[cfg(test)]
mod tests {
    use indoc::indoc;
    use rstest::{fixture, rstest};

    use super::*;

    const WORKTREE: &str = "/repo/.worktrees/feature";

    /// tmux server 10 hosts two panes in the caller's window (pane pids 20
    /// and 60) and one pane in another window (pane pid 80). The caller
    /// `a` (pid 40) runs under the agent (pid 30) in pane 20; the agent also
    /// started a background job (pid 50, with its own child 51).
    #[fixture]
    fn snapshot() -> ProcessSnapshot {
        ProcessSnapshot::from_ps_output(indoc! {"
            1 0 launchd
            10 1 tmux
            20 10 zsh
            30 20 claude
            35 30 zsh
            40 35 a
            50 30 python
            51 50 python
            60 10 zsh
            80 10 zsh
            81 80 nvim
        "})
    }

    #[rstest]
    #[case::caller_itself_inside(&[(10, "/repo"), (20, "/repo"), (30, "/repo"), (35, "/repo"), (40, WORKTREE)], &[20, 60], Some(20), &[40])]
    #[case::agent_inside_subdirectory(&[(10, "/repo"), (20, "/repo"), (30, "/repo/.worktrees/feature/src"), (35, "/repo"), (40, "/repo")], &[20, 60], Some(20), &[30])]
    #[case::background_job_inside_is_ignored_when_caller_is_outside(&[(10, "/repo"), (20, "/repo"), (30, "/repo"), (35, "/repo"), (40, "/repo"), (50, "/repo"), (51, WORKTREE)], &[20, 60], Some(20), &[])]
    #[case::missing_cwd_above_pane_does_not_block_exclusion(&[(20, "/repo"), (30, "/repo"), (35, "/repo"), (40, "/repo"), (51, WORKTREE)], &[20, 60], Some(20), &[])]
    #[case::missing_caller_to_pane_cwd_keeps_background_job(&[(10, "/repo"), (20, "/repo"), (30, "/repo"), (40, "/repo"), (51, WORKTREE)], &[20, 60], Some(20), &[51])]
    #[case::missing_pane_cwd_keeps_background_job(&[(10, "/repo"), (30, "/repo"), (35, "/repo"), (40, "/repo"), (51, WORKTREE)], &[20, 60], Some(20), &[51])]
    #[case::known_ancestor_above_pane_inside_keeps_background_job(&[(10, WORKTREE), (20, "/repo"), (30, "/repo"), (35, "/repo"), (40, "/repo"), (51, WORKTREE)], &[20, 60], Some(20), &[10, 51])]
    #[case::background_job_inside_is_kept_when_caller_is_inside(&[(10, "/repo"), (20, "/repo"), (30, WORKTREE), (35, "/repo"), (40, "/repo"), (51, WORKTREE)], &[20, 60], Some(20), &[30, 51])]
    #[case::other_pane_in_caller_window_inside(&[(10, "/repo"), (20, "/repo"), (30, "/repo"), (35, "/repo"), (40, "/repo"), (60, WORKTREE)], &[20, 60], Some(20), &[60])]
    #[case::pane_pid_not_in_caller_ancestry_keeps_other_pane(&[(10, "/repo"), (20, "/repo"), (30, "/repo"), (35, "/repo"), (40, "/repo"), (60, WORKTREE)], &[20, 60], Some(60), &[60])]
    #[case::other_window_inside(&[(10, "/repo"), (20, "/repo"), (30, "/repo"), (35, "/repo"), (40, "/repo"), (80, WORKTREE), (81, WORKTREE)], &[20, 60], Some(20), &[])]
    #[case::sibling_worktree(&[(10, "/repo"), (20, "/repo"), (30, "/repo"), (35, "/repo"), (40, "/repo/.worktrees/feature-extra")], &[20, 60], Some(20), &[])]
    #[case::caller_pane_unavailable_keeps_background_job(&[(10, "/repo"), (20, "/repo"), (30, "/repo"), (35, "/repo"), (40, "/repo"), (51, WORKTREE)], &[20, 60], None, &[51])]
    #[case::background_job_without_roots(&[(10, "/repo"), (20, "/repo"), (30, "/repo"), (35, "/repo"), (40, "/repo"), (51, WORKTREE)], &[], None, &[])]
    #[case::ancestor_without_roots(&[(20, WORKTREE)], &[], None, &[20])]
    fn finds_caller_processes_inside_the_path(
        snapshot: ProcessSnapshot,
        #[case] cwds: &[(u32, &str)],
        #[case] roots: &[u32],
        #[case] caller_pane_pid: Option<u32>,
        #[case] expected: &[u32],
    ) {
        let cwds: HashMap<u32, PathBuf> = cwds
            .iter()
            .map(|&(pid, cwd)| (pid, PathBuf::from(cwd)))
            .collect();

        assert_eq!(
            caller_pids_in_path(
                &snapshot,
                &cwds,
                40,
                roots,
                caller_pane_pid,
                Path::new(WORKTREE),
            ),
            expected,
        );
    }
}
