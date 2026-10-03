//! `a agent clean-detached` (hidden) subcommand.
//!
//! Non-interactive batch worktree cleanup designed to be spawned in the
//! background by `a agent watch`. The caller is responsible for detaching
//! (`nohup`/`setsid`); this command never reads stdin and never writes
//! to stdout/stderr. Progress is journaled to the shared tracing log
//! (`~/.cache/armyknife/logs/armyknife.log.YYYY-MM-DD`) under a span
//! whose `run_id` the caller passes in via `--run-id`, so it can later
//! tail the same log and pick out just this run's events.

use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use anyhow::Result;
use clap::Args;
use tracing::Instrument;

use crate::shared::cleanup;
use crate::shared::hooks;
use crate::shared::log::short_run_id;
use crate::shared::merge_notify::notify_delegator_of_merge;
use crate::shared::worktree_merge::find_worktree_cleanup_info;

#[derive(Args, Clone, PartialEq, Eq)]
pub struct CleanDetachedArgs {
    /// Worktree paths to clean up. Each must be the worktree root.
    pub paths: Vec<PathBuf>,

    /// Read additional paths from a file (newline-separated). Useful when the
    /// number of paths would exceed the OS argv limit.
    #[arg(long, value_name = "FILE")]
    pub paths_file: Option<PathBuf>,

    /// Tag every event in the tracing log with this run id so the
    /// caller can filter the shared log for just this run. Generated
    /// when absent.
    #[arg(long, value_name = "ID")]
    pub run_id: Option<String>,
}

/// Tracing target for events emitted by this subcommand. Callers
/// (`agent watch` clean view) filter the rotating log by exactly this
/// string when tailing / summarising.
pub const EVENT_TARGET: &str = "armyknife::commands::agent::clean";

pub async fn run(args: &CleanDetachedArgs) -> Result<()> {
    // All failures stay off the parent TTY: `agent watch` spawns this process
    // detached and the silent contract is documented at the top of this
    // module. Failures that happen after the span is open are recorded
    // in the tracing log on a best-effort basis.
    let _ = run_inner(args).await;
    Ok(())
}

async fn run_inner(args: &CleanDetachedArgs) -> Result<()> {
    let run_id = args.run_id.clone().unwrap_or_else(short_run_id);
    let span = tracing::info_span!("agent.clean", run_id = %run_id);

    let paths = collect_paths(args);
    let cleaner = RealCleaner;
    async {
        let (ok, failed) = run_with(&paths, &cleaner).await;
        tracing::info!(
            target: EVENT_TARGET,
            event = "agent.clean.done",
            ok,
            failed,
        );
    }
    .instrument(span)
    .await;
    Ok(())
}

fn collect_paths(args: &CleanDetachedArgs) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = args.paths.clone();
    if let Some(file) = &args.paths_file {
        match File::open(file) {
            Ok(f) => {
                // Skip undecodable lines instead of aborting: one bad line
                // must not drop the remaining paths from a batch cleanup.
                for line in BufReader::new(f).lines().map_while(std::result::Result::ok) {
                    let trimmed = line.trim();
                    if !trimmed.is_empty() {
                        paths.push(PathBuf::from(trimmed));
                    }
                }
            }
            Err(e) => {
                tracing::warn!(
                    target: EVENT_TARGET,
                    event = "agent.clean.err",
                    path = %file.display(),
                    msg = format!("failed to open paths file: {e}"),
                );
            }
        }
    }
    paths
}

/// Abstracts the worktree cleanup boundary so tests can avoid invoking
/// real git/tmux.
trait Cleaner {
    async fn cleanup(&self, path: &Path) -> Result<()>;
}

struct RealCleaner;

impl Cleaner for RealCleaner {
    async fn cleanup(&self, path: &Path) -> Result<()> {
        let worktree = find_worktree_cleanup_info(path).await;
        if let Some(worktree) = &worktree
            && worktree.merged
            && let Some(branch) = worktree.branch.as_deref()
        {
            notify_delegator_of_merge(&worktree.main_repo, branch, &worktree.path).await;
        }

        let result = cleanup::cleanup_worktree_resources_with_post_delete(path, || {
            if let Some(worktree) = &worktree {
                hooks::spawn_post_worktree_delete_hook(
                    worktree.main_repo.workdir(),
                    &worktree.path,
                    worktree.branch.as_deref(),
                    worktree.merged,
                );
            }
        })?;
        if !result.worktree_deleted {
            anyhow::bail!("worktree not deleted: {}", path.display());
        }
        Ok(())
    }
}

async fn run_with<C: Cleaner>(paths: &[PathBuf], cleaner: &C) -> (usize, usize) {
    tracing::info!(
        target: EVENT_TARGET,
        event = "agent.clean.start",
        total = paths.len(),
    );

    let mut ok = 0usize;
    let mut failed = 0usize;
    for path in paths {
        let path_str = path.to_string_lossy().into_owned();
        match cleaner.cleanup(path).await {
            Ok(()) => {
                ok += 1;
                tracing::info!(
                    target: EVENT_TARGET,
                    event = "agent.clean.ok",
                    path = %path_str,
                );
            }
            Err(e) => {
                failed += 1;
                tracing::warn!(
                    target: EVENT_TARGET,
                    event = "agent.clean.err",
                    path = %path_str,
                    msg = format!("{e:#}"),
                );
            }
        }
    }

    (ok, failed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use indoc::indoc;
    use rstest::rstest;
    use std::cell::RefCell;
    use std::fs;
    use tempfile::TempDir;

    struct FakeCleaner {
        plan: Vec<(String, std::result::Result<(), String>)>,
        calls: RefCell<Vec<PathBuf>>,
    }

    impl Cleaner for FakeCleaner {
        async fn cleanup(&self, path: &Path) -> Result<()> {
            self.calls.borrow_mut().push(path.to_path_buf());
            let s = path.to_string_lossy().to_string();
            for (p, outcome) in &self.plan {
                if p == &s {
                    return match outcome {
                        Ok(()) => Ok(()),
                        Err(msg) => Err(anyhow::anyhow!(msg.clone())),
                    };
                }
            }
            Ok(())
        }
    }

    #[tokio::test]
    async fn run_with_continues_after_error() {
        let cleaner = FakeCleaner {
            plan: vec![
                ("/a".to_string(), Ok(())),
                ("/b".to_string(), Err("nope".to_string())),
                ("/c".to_string(), Ok(())),
            ],
            calls: RefCell::new(Vec::new()),
        };
        let outcome = run_with(
            &[
                PathBuf::from("/a"),
                PathBuf::from("/b"),
                PathBuf::from("/c"),
            ],
            &cleaner,
        )
        .await;

        assert_eq!(
            (outcome, cleaner.calls.borrow().clone()),
            (
                (2, 1),
                vec![
                    PathBuf::from("/a"),
                    PathBuf::from("/b"),
                    PathBuf::from("/c")
                ],
            )
        );
    }

    #[rstest]
    fn collect_paths_merges_argv_and_file() {
        let tmp = TempDir::new().unwrap();
        let file = tmp.path().join("paths.txt");
        fs::write(
            &file,
            indoc! {"
                /from/file/1

                /from/file/2
            "},
        )
        .unwrap();

        let args = CleanDetachedArgs {
            paths: vec![PathBuf::from("/from/argv")],
            paths_file: Some(file),
            run_id: None,
        };
        let paths = collect_paths(&args);
        assert_eq!(
            paths,
            vec![
                PathBuf::from("/from/argv"),
                PathBuf::from("/from/file/1"),
                PathBuf::from("/from/file/2"),
            ]
        );
    }
}
