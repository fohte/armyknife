//! `a cc clean-detached` (hidden) subcommand.
//!
//! Non-interactive batch worktree cleanup designed to be spawned in the
//! background by `a cc watch`. The caller is responsible for detaching
//! (`nohup`/`setsid`); this command never reads stdin and never writes to
//! stdout/stderr. Progress is journaled to a per-pid JSONL log under
//! `~/.cache/armyknife/clean/`.

use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use anyhow::{Context, Result};
use chrono::{SecondsFormat, Utc};
use clap::Args;
use serde::Serialize;

use crate::shared::cache;
use crate::shared::cleanup;

/// Logs older than this are removed on each invocation.
const LOG_TTL: Duration = Duration::from_secs(7 * 24 * 60 * 60);

#[derive(Args, Clone, PartialEq, Eq)]
pub struct CleanDetachedArgs {
    /// Worktree paths to clean up. Each must be the worktree root.
    pub paths: Vec<PathBuf>,

    /// Read additional paths from a file (newline-separated). Useful when the
    /// number of paths would exceed the OS argv limit.
    #[arg(long, value_name = "FILE")]
    pub paths_file: Option<PathBuf>,
}

pub fn run(args: &CleanDetachedArgs) -> Result<()> {
    // All failures stay off the parent TTY: `cc watch` spawns this process
    // detached and the silent contract is documented at the top of this
    // module. Failures that happen after the log file is open are recorded
    // there on a best-effort basis; failures before that point are
    // necessarily lost.
    let _ = run_inner(args);
    Ok(())
}

fn run_inner(args: &CleanDetachedArgs) -> Result<()> {
    let log_dir = log_dir().context("cache dir is unavailable")?;
    fs::create_dir_all(&log_dir)
        .with_context(|| format!("failed to create log dir: {}", log_dir.display()))?;

    gc_old_logs(&log_dir, LOG_TTL, SystemTime::now());

    let pid = std::process::id();
    let log_path = log_dir.join(format!("{pid}.jsonl"));
    let mut log = LogWriter::create(&log_path)?;

    let paths = collect_paths(args, &mut log);

    let cleaner = RealCleaner;
    run_with(&mut log, &paths, &cleaner);
    Ok(())
}

fn log_dir() -> Option<PathBuf> {
    cache::base_dir().map(|d| d.join("clean"))
}

fn collect_paths(args: &CleanDetachedArgs, log: &mut LogWriter) -> Vec<PathBuf> {
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
                let _ = log.write(&Event::Err {
                    ts: now_ts(),
                    path: &file.to_string_lossy(),
                    msg: format!("failed to open paths file: {e}"),
                });
            }
        }
    }
    paths
}

/// Removes `*.jsonl` files in `dir` older than `ttl` relative to `now`.
/// Errors are swallowed silently (best-effort cleanup, no stdout/stderr).
fn gc_old_logs(dir: &Path, ttl: Duration, now: SystemTime) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("jsonl") {
            continue;
        }
        let Ok(meta) = entry.metadata() else { continue };
        let Ok(modified) = meta.modified() else {
            continue;
        };
        if now
            .duration_since(modified)
            .map(|age| age > ttl)
            .unwrap_or(false)
        {
            let _ = fs::remove_file(&path);
        }
    }
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

#[derive(Serialize)]
#[serde(tag = "event", rename_all = "lowercase")]
enum Event<'a> {
    Start {
        ts: String,
        total: usize,
    },
    Ok {
        ts: String,
        path: &'a str,
    },
    Err {
        ts: String,
        path: &'a str,
        msg: String,
    },
    Done {
        ts: String,
        ok: usize,
        failed: usize,
    },
}

struct LogWriter {
    file: File,
}

impl LogWriter {
    fn create(path: &Path) -> Result<Self> {
        let file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(path)
            .with_context(|| format!("failed to open log file: {}", path.display()))?;
        Ok(Self { file })
    }

    fn write(&mut self, ev: &Event<'_>) -> Result<()> {
        let line = serde_json::to_string(ev)?;
        self.file.write_all(line.as_bytes())?;
        self.file.write_all(b"\n")?;
        self.file.flush()?;
        Ok(())
    }
}

fn now_ts() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)
}

fn run_with<C: Cleaner>(log: &mut LogWriter, paths: &[PathBuf], cleaner: &C) {
    let _ = log.write(&Event::Start {
        ts: now_ts(),
        total: paths.len(),
    });

    let mut ok = 0usize;
    let mut failed = 0usize;
    for path in paths {
        let path_str = path.to_string_lossy();
        match cleaner.cleanup(path) {
            Ok(()) => {
                ok += 1;
                let _ = log.write(&Event::Ok {
                    ts: now_ts(),
                    path: &path_str,
                });
            }
            Err(e) => {
                failed += 1;
                let _ = log.write(&Event::Err {
                    ts: now_ts(),
                    path: &path_str,
                    msg: format!("{e:#}"),
                });
            }
        }
    }

    let _ = log.write(&Event::Done {
        ts: now_ts(),
        ok,
        failed,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::DateTime;
    use indoc::indoc;
    use rstest::rstest;
    use serde_json::json;
    use std::cell::RefCell;
    use std::time::Duration;
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

    /// Reads JSONL events, validates each `ts` field parses as RFC3339, and
    /// strips it so the rest can be compared whole against `json!(...)`
    /// literals.
    fn read_events(path: &Path) -> Vec<serde_json::Value> {
        let s = fs::read_to_string(path).unwrap();
        s.lines()
            .filter(|l| !l.is_empty())
            .map(|l| {
                let mut v: serde_json::Value = serde_json::from_str(l).unwrap();
                let ts = v["ts"].as_str().unwrap();
                DateTime::parse_from_rfc3339(ts).unwrap();
                v.as_object_mut().unwrap().remove("ts");
                v
            })
            .collect()
    }

    #[rstest]
    fn run_with_writes_start_ok_err_done() {
        let tmp = TempDir::new().unwrap();
        let log_path = tmp.path().join("test.jsonl");
        let mut log = LogWriter::create(&log_path).unwrap();

        let cleaner = FakeCleaner {
            plan: vec![
                ("/a".to_string(), Ok(())),
                ("/b".to_string(), Err("boom".to_string())),
                ("/c".to_string(), Ok(())),
            ],
            calls: RefCell::new(Vec::new()),
        };
        let paths = vec![
            PathBuf::from("/a"),
            PathBuf::from("/b"),
            PathBuf::from("/c"),
        ];
        run_with(&mut log, &paths, &cleaner);

        assert_eq!(
            read_events(&log_path),
            vec![
                json!({"event": "start", "total": 3}),
                json!({"event": "ok", "path": "/a"}),
                json!({"event": "err", "path": "/b", "msg": "boom"}),
                json!({"event": "ok", "path": "/c"}),
                json!({"event": "done", "ok": 2, "failed": 1}),
            ]
        );
    }

    #[rstest]
    fn run_with_continues_after_error() {
        let tmp = TempDir::new().unwrap();
        let log_path = tmp.path().join("test.jsonl");
        let mut log = LogWriter::create(&log_path).unwrap();

        let cleaner = FakeCleaner {
            plan: vec![("/a".to_string(), Err("nope".to_string()))],
            calls: RefCell::new(Vec::new()),
        };
        run_with(
            &mut log,
            &[PathBuf::from("/a"), PathBuf::from("/b")],
            &cleaner,
        );

        assert_eq!(cleaner.calls.borrow().len(), 2);
    }

    #[rstest]
    fn run_with_empty_paths_writes_only_start_and_done() {
        let tmp = TempDir::new().unwrap();
        let log_path = tmp.path().join("test.jsonl");
        let mut log = LogWriter::create(&log_path).unwrap();

        let cleaner = FakeCleaner {
            plan: vec![],
            calls: RefCell::new(Vec::new()),
        };
        run_with(&mut log, &[], &cleaner);

        assert_eq!(
            read_events(&log_path),
            vec![
                json!({"event": "start", "total": 0}),
                json!({"event": "done", "ok": 0, "failed": 0}),
            ]
        );
    }

    #[rstest]
    #[case::expired(Duration::from_secs(60 * 60 * 24 * 8), true)]
    #[case::fresh(Duration::from_secs(60 * 60), false)]
    fn gc_removes_expired_jsonl(#[case] age: Duration, #[case] should_remove: bool) {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path();
        let target = dir.join("123.jsonl");
        fs::write(&target, "{}\n").unwrap();

        let modified = SystemTime::now() - age;
        let f = File::open(&target).unwrap();
        f.set_modified(modified).unwrap();
        drop(f);

        gc_old_logs(dir, LOG_TTL, SystemTime::now());

        assert_eq!(target.exists(), !should_remove);
    }

    #[rstest]
    fn gc_ignores_non_jsonl_files() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path();
        let other = dir.join("note.txt");
        fs::write(&other, "x").unwrap();
        let f = File::open(&other).unwrap();
        f.set_modified(SystemTime::now() - Duration::from_secs(60 * 60 * 24 * 30))
            .unwrap();
        drop(f);

        gc_old_logs(dir, LOG_TTL, SystemTime::now());

        assert!(other.exists());
    }

    #[rstest]
    fn gc_on_missing_dir_is_noop() {
        let tmp = TempDir::new().unwrap();
        let missing = tmp.path().join("does-not-exist");
        gc_old_logs(&missing, LOG_TTL, SystemTime::now());
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
        let log_path = tmp.path().join("test.jsonl");
        let mut log = LogWriter::create(&log_path).unwrap();

        let args = CleanDetachedArgs {
            paths: vec![PathBuf::from("/from/argv")],
            paths_file: Some(file),
        };
        let paths = collect_paths(&args, &mut log);
        assert_eq!(
            paths,
            vec![
                PathBuf::from("/from/argv"),
                PathBuf::from("/from/file/1"),
                PathBuf::from("/from/file/2"),
            ]
        );
        // No err event should be logged for a successful read.
        assert!(read_events(&log_path).is_empty());
    }

    #[rstest]
    fn collect_paths_logs_err_when_paths_file_missing() {
        let tmp = TempDir::new().unwrap();
        let missing = tmp.path().join("nonexistent").join("paths-file");
        let log_path = tmp.path().join("test.jsonl");
        let mut log = LogWriter::create(&log_path).unwrap();

        // Reproduce the exact OS error string the production code will format.
        let expected_msg = format!(
            "failed to open paths file: {}",
            File::open(&missing).unwrap_err()
        );

        let args = CleanDetachedArgs {
            paths: vec![PathBuf::from("/from/argv")],
            paths_file: Some(missing.clone()),
        };
        let paths = collect_paths(&args, &mut log);
        assert_eq!(paths, vec![PathBuf::from("/from/argv")]);

        assert_eq!(
            read_events(&log_path),
            vec![json!({
                "event": "err",
                "path": missing.to_string_lossy(),
                "msg": expected_msg,
            })]
        );
    }
}
