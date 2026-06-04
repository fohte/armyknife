//! `a cc clean-detached` (hidden) subcommand.
//!
//! Non-interactive batch worktree cleanup designed to be spawned in the
//! background by `a cc watch`. The caller is responsible for detaching
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

use crate::shared::cleanup;
use crate::shared::log::short_run_id;

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
/// (`cc watch` clean view) filter the rotating log by exactly this
/// string when tailing / summarising.
pub const EVENT_TARGET: &str = "armyknife::commands::cc::clean";

pub fn run(args: &CleanDetachedArgs) -> Result<()> {
    // All failures stay off the parent TTY: `cc watch` spawns this process
    // detached and the silent contract is documented at the top of this
    // module. Failures that happen after the span is open are recorded
    // in the tracing log on a best-effort basis.
    let _ = run_inner(args);
    Ok(())
}

fn run_inner(args: &CleanDetachedArgs) -> Result<()> {
    let run_id = args.run_id.clone().unwrap_or_else(short_run_id);
    let span = tracing::info_span!("cc.clean", run_id = %run_id);
    let _entered = span.enter();

    let paths = collect_paths(args);
    let cleaner = RealCleaner;
    run_with(&paths, &cleaner);
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
                    event = "cc.clean.err",
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
    fn cleanup(&self, path: &Path) -> Result<()>;
}

struct RealCleaner;

impl Cleaner for RealCleaner {
    fn cleanup(&self, path: &Path) -> Result<()> {
        let result = cleanup::cleanup_worktree_resources(path)?;
        if !result.worktree_deleted {
            anyhow::bail!("worktree not deleted: {}", path.display());
        }
        Ok(())
    }
}

fn run_with<C: Cleaner>(paths: &[PathBuf], cleaner: &C) {
    tracing::info!(
        target: EVENT_TARGET,
        event = "cc.clean.start",
        total = paths.len(),
    );

    let mut ok = 0usize;
    let mut failed = 0usize;
    for path in paths {
        let path_str = path.to_string_lossy().into_owned();
        match cleaner.cleanup(path) {
            Ok(()) => {
                ok += 1;
                tracing::info!(
                    target: EVENT_TARGET,
                    event = "cc.clean.ok",
                    path = %path_str,
                );
            }
            Err(e) => {
                failed += 1;
                tracing::warn!(
                    target: EVENT_TARGET,
                    event = "cc.clean.err",
                    path = %path_str,
                    msg = format!("{e:#}"),
                );
            }
        }
    }

    tracing::info!(
        target: EVENT_TARGET,
        event = "cc.clean.done",
        ok = ok,
        failed = failed,
    );
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
        fn cleanup(&self, path: &Path) -> Result<()> {
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

    #[rstest]
    fn run_with_continues_after_error() {
        let cleaner = FakeCleaner {
            plan: vec![("/a".to_string(), Err("nope".to_string()))],
            calls: RefCell::new(Vec::new()),
        };
        run_with(&[PathBuf::from("/a"), PathBuf::from("/b")], &cleaner);
        assert_eq!(cleaner.calls.borrow().len(), 2);
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
