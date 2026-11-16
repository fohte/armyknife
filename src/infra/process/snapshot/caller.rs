use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use super::{ProcessSnapshot, list_process_cwds};

/// Returns the pids of the caller's processes whose cwd is inside `path`.
///
/// The caller's processes are this process and its ancestors, plus every
/// descendant of `roots` (e.g. the pane processes of the caller's tmux
/// window, which also covers background jobs the calling agent started).
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
        &resolved_path,
    ))
}

fn caller_pids_in_path(
    snapshot: &ProcessSnapshot,
    cwds: &HashMap<u32, PathBuf>,
    self_pid: u32,
    roots: &[u32],
    path: &Path,
) -> Vec<u32> {
    let mut caller_pids = snapshot.ancestors(self_pid);
    for &root in roots {
        caller_pids.extend(snapshot.self_and_descendants(root));
    }
    let mut pids: Vec<u32> = cwds
        .iter()
        .filter(|(pid, cwd)| caller_pids.contains(pid) && cwd.starts_with(path))
        .map(|(&pid, _)| pid)
        .collect();
    pids.sort_unstable();
    pids
}

impl ProcessSnapshot {
    fn self_and_descendants(&self, pid: u32) -> HashSet<u32> {
        let mut result = HashSet::from([pid]);
        let mut stack = vec![pid];
        while let Some(current) = stack.pop() {
            for &(child, _) in self.children.get(&current).into_iter().flatten() {
                if result.insert(child) {
                    stack.push(child);
                }
            }
        }
        result
    }
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
    #[case::caller_itself_inside(&[(40, WORKTREE)], &[20, 60], &[40])]
    #[case::agent_inside_subdirectory(&[(30, "/repo/.worktrees/feature/src")], &[20, 60], &[30])]
    #[case::background_job_inside(&[(50, "/repo"), (51, WORKTREE)], &[20, 60], &[51])]
    #[case::other_pane_in_caller_window_inside(&[(60, WORKTREE)], &[20, 60], &[60])]
    #[case::other_window_inside(&[(80, WORKTREE), (81, WORKTREE)], &[20, 60], &[])]
    #[case::sibling_worktree(&[(40, "/repo/.worktrees/feature-extra")], &[20, 60], &[])]
    #[case::background_job_without_roots(&[(51, WORKTREE)], &[], &[])]
    #[case::ancestor_without_roots(&[(20, WORKTREE)], &[], &[20])]
    fn finds_caller_processes_inside_the_path(
        snapshot: ProcessSnapshot,
        #[case] cwds: &[(u32, &str)],
        #[case] roots: &[u32],
        #[case] expected: &[u32],
    ) {
        let cwds: HashMap<u32, PathBuf> = cwds
            .iter()
            .map(|&(pid, cwd)| (pid, PathBuf::from(cwd)))
            .collect();

        assert_eq!(
            caller_pids_in_path(&snapshot, &cwds, 40, roots, Path::new(WORKTREE)),
            expected,
        );
    }
}
