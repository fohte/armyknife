//! Detached `a cc clean-detached` spawn + tracing-log tailing.
//!
//! The clean view spawns the cleanup as a fully detached process (its own
//! session, stdio redirected to `/dev/null`) so that closing `cc watch`
//! never aborts an in-flight cleanup. The child emits its progress as
//! tracing events into the shared rotating log at
//! `~/.cache/armyknife/logs/armyknife.log.YYYY-MM-DD`, tagged with a
//! per-run `run_id` so this side can tail the same file and pick out
//! only the events that belong to this run. Reusing the shared log
//! infrastructure means failure messages survive past a single TUI
//! cycle and obey the same daily rotation / 7-day retention as every
//! other command.

use std::io::Write;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};

use crate::commands::cc::clean_detached;
use crate::shared::log::{current_log_path, read_jsonl_lines_since, short_run_id};

/// How often the tail thread polls the rotating log.
pub const TAIL_INTERVAL: Duration = Duration::from_millis(500);

/// One semantic event extracted from the tracing log. Independent of
/// the wire encoding so the rest of the TUI doesn't have to know
/// whether the producer used tracing, JSONL, or something else.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CleanLogEvent {
    Start { total: usize },
    Ok { path: String },
    Err { path: String, msg: String },
    Done { ok: usize, failed: usize },
}

impl CleanLogEvent {
    /// Decode one parsed tracing-log line into a `CleanLogEvent` when
    /// it belongs to the requested run, else `None`. The producer side
    /// is responsible for emitting events under [`EVENT_TARGET`] with a
    /// span carrying `run_id`.
    pub fn from_tracing_value(value: &serde_json::Value, expected_run_id: &str) -> Option<Self> {
        if value.get("target")?.as_str()? != clean_detached::EVENT_TARGET {
            return None;
        }
        let span_run_id = value.get("span")?.get("run_id")?.as_str()?;
        if span_run_id != expected_run_id {
            return None;
        }
        match value.get("event")?.as_str()? {
            "cc.clean.start" => Some(CleanLogEvent::Start {
                total: value.get("total")?.as_u64()? as usize,
            }),
            "cc.clean.ok" => Some(CleanLogEvent::Ok {
                path: value.get("path")?.as_str()?.to_string(),
            }),
            "cc.clean.err" => Some(CleanLogEvent::Err {
                path: value.get("path")?.as_str()?.to_string(),
                msg: value
                    .get("msg")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
            }),
            "cc.clean.done" => Some(CleanLogEvent::Done {
                ok: value.get("ok")?.as_u64()? as usize,
                failed: value.get("failed")?.as_u64()? as usize,
            }),
            _ => None,
        }
    }
}

/// Aggregated state derived from a stream of [`CleanLogEvent`]s.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CleanProgress {
    pub run_id: String,
    pub total: usize,
    pub completed: usize,
    pub failed: usize,
    pub last_path: Option<String>,
    pub done: bool,
    /// Pending delete notifications drained by the UI on every apply
    /// pass; intentionally not cumulative.
    pub deleted_paths: Vec<String>,
    /// Cumulative record of confirmed deletions for the lifetime of
    /// this progress object. Used to filter stale rows out of a fresh
    /// PR-fetch snapshot if the user re-enters the clean view while
    /// the child is still running.
    pub confirmed_deleted: Vec<String>,
}

impl CleanProgress {
    pub fn new(run_id: String) -> Self {
        Self {
            run_id,
            ..Self::default()
        }
    }

    pub fn apply(&mut self, event: &CleanLogEvent) {
        // Producer guarantees `Done` is terminal; ignore any straggler
        // so a misbehaving log cannot mutate the displayed summary.
        if self.done {
            return;
        }
        match event {
            CleanLogEvent::Start { total } => {
                self.total = *total;
            }
            CleanLogEvent::Ok { path } => {
                self.completed += 1;
                self.last_path = Some(path.clone());
                self.deleted_paths.push(path.clone());
                self.confirmed_deleted.push(path.clone());
            }
            CleanLogEvent::Err { path, .. } => {
                self.failed += 1;
                self.last_path = Some(path.clone());
            }
            CleanLogEvent::Done { ok, failed } => {
                self.done = true;
                self.completed = *ok;
                self.failed = *failed;
            }
        }
    }

    pub fn render_line(&self) -> String {
        if self.done {
            return format!(
                "Cleaned {} worktree{}, {} failed",
                self.completed,
                if self.completed == 1 { "" } else { "s" },
                self.failed
            );
        }
        let head = if self.total > 0 {
            format!("Cleaning... ({}/{})", self.completed, self.total)
        } else {
            "Cleaning...".to_string()
        };
        let mut out = head;
        if let Some(p) = &self.last_path {
            out.push(' ');
            out.push_str(p);
        }
        if self.failed > 0 {
            out.push_str(&format!(
                " ({} error{})",
                self.failed,
                if self.failed == 1 { "" } else { "s" }
            ));
        }
        out
    }
}

/// Spawn `a cc clean-detached` as a fully detached process. The child
/// gets its own session (so `cc watch` exiting cannot send it SIGHUP),
/// its working directory is `/`, and stdio is wired to `/dev/null`.
/// Returns the `run_id` so the caller can tail the shared log for
/// matching events.
pub fn spawn_detached_clean(paths: &[PathBuf]) -> Result<String> {
    use std::os::unix::process::CommandExt;
    use std::process::Stdio;

    use crate::shared::command;

    let exe = std::env::current_exe().context("failed to resolve current exe")?;
    let run_id = short_run_id();

    // Persisted (not auto-deleted) — the child only reads this file
    // and never unlinks it. Cleanup relies on OS `/tmp` GC.
    let mut paths_file =
        tempfile::NamedTempFile::new().context("failed to create temp paths file")?;
    for p in paths {
        writeln!(paths_file, "{}", p.display()).context("failed to write paths file")?;
    }
    paths_file.flush().context("failed to flush paths file")?;
    let (_file, file_path) = paths_file
        .keep()
        .map_err(|e| anyhow::anyhow!("failed to persist paths file: {e}"))?;

    let mut cmd = command::new(&exe);
    cmd.arg("cc")
        .arg("clean-detached")
        .arg("--paths-file")
        .arg(&file_path)
        .arg("--run-id")
        .arg(&run_id)
        .current_dir("/")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    // SAFETY: `setsid` only manipulates the calling process's session
    // membership; it is async-signal-safe and documented as one of the
    // operations safe to call in `pre_exec`. Detaching here is what
    // prevents the parent TTY's HUP / SIGINT from reaching the child.
    unsafe {
        cmd.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }

    match cmd.spawn() {
        Ok(_child) => Ok(run_id),
        Err(e) => {
            // Roll back the persisted paths file; without the child it
            // would only ever be cleaned up by the OS `/tmp` GC.
            let _ = std::fs::remove_file(&file_path);
            Err(anyhow::Error::new(e).context("failed to spawn clean-detached child"))
        }
    }
}

/// Tail today's rotating log file for tracing events that belong to
/// `run_id` and convert them to [`CleanLogEvent`]s. Returns the new
/// cursor for the next poll. When the date rolls over mid-run, callers
/// should swap to the new file via [`current_log_path`].
pub fn read_new_events(
    log_path: &std::path::Path,
    cursor: u64,
    run_id: &str,
) -> std::io::Result<(Vec<CleanLogEvent>, u64)> {
    let (values, new_cursor) = read_jsonl_lines_since(log_path, cursor)?;
    let events = values
        .into_iter()
        .filter_map(|v| CleanLogEvent::from_tracing_value(&v, run_id))
        .collect();
    Ok((events, new_cursor))
}

/// Convenience for the spawn site: today's tracing log path that the
/// tail thread should open. Returns `None` if no cache dir is
/// available (in which case progress display is silently skipped).
pub fn live_log_path() -> Option<PathBuf> {
    current_log_path()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;
    use serde_json::json;
    use std::fs;
    use tempfile::TempDir;

    fn line(value: serde_json::Value) -> String {
        let mut s = serde_json::to_string(&value).expect("serialize");
        s.push('\n');
        s
    }

    fn tracing_event(target: &str, run_id: &str, body: serde_json::Value) -> serde_json::Value {
        let mut obj = body.as_object().expect("obj").clone();
        obj.insert("target".to_string(), json!(target));
        obj.insert("span".to_string(), json!({ "run_id": run_id }));
        serde_json::Value::Object(obj)
    }

    #[rstest]
    fn apply_aggregates_events() {
        let mut p = CleanProgress::new("rid".to_string());
        p.apply(&CleanLogEvent::Start { total: 3 });
        p.apply(&CleanLogEvent::Ok {
            path: "/a".to_string(),
        });
        p.apply(&CleanLogEvent::Err {
            path: "/b".to_string(),
            msg: "boom".to_string(),
        });
        p.apply(&CleanLogEvent::Ok {
            path: "/c".to_string(),
        });
        p.apply(&CleanLogEvent::Done { ok: 2, failed: 1 });

        assert_eq!(p.run_id, "rid");
        assert_eq!(p.total, 3);
        assert_eq!(p.completed, 2);
        assert_eq!(p.failed, 1);
        assert!(p.done);
        assert_eq!(p.deleted_paths, vec!["/a".to_string(), "/c".to_string()]);
        assert_eq!(
            p.confirmed_deleted,
            vec!["/a".to_string(), "/c".to_string()]
        );
    }

    #[rstest]
    fn apply_ignores_events_after_done() {
        let mut p = CleanProgress::new("rid".to_string());
        p.apply(&CleanLogEvent::Start { total: 1 });
        p.apply(&CleanLogEvent::Done { ok: 1, failed: 0 });
        p.apply(&CleanLogEvent::Ok {
            path: "/late".to_string(),
        });
        assert_eq!(p.completed, 1);
        assert!(p.deleted_paths.is_empty());
    }

    #[rstest]
    #[case::live(false, 3, 1, 0, "Cleaning... (1/3) /a")]
    #[case::with_failures(false, 3, 1, 1, "Cleaning... (1/3) /a (1 error)")]
    #[case::done(true, 0, 5, 1, "Cleaned 5 worktrees, 1 failed")]
    fn render_line_formats(
        #[case] done: bool,
        #[case] total: usize,
        #[case] completed: usize,
        #[case] failed: usize,
        #[case] expected: &str,
    ) {
        let p = CleanProgress {
            run_id: "r".to_string(),
            total,
            completed,
            failed,
            last_path: if completed > 0 || failed > 0 {
                Some("/a".to_string())
            } else {
                None
            },
            done,
            deleted_paths: Vec::new(),
            confirmed_deleted: Vec::new(),
        };
        assert_eq!(p.render_line(), expected);
    }

    #[rstest]
    fn from_tracing_value_filters_target_and_run_id() {
        let target = clean_detached::EVENT_TARGET;
        let mine = tracing_event(
            target,
            "mine",
            json!({"event": "cc.clean.start", "total": 2}),
        );
        let theirs = tracing_event(
            target,
            "theirs",
            json!({"event": "cc.clean.ok", "path": "/x"}),
        );
        let other_target = tracing_event(
            "armyknife::other",
            "mine",
            json!({"event": "cc.clean.ok", "path": "/y"}),
        );

        assert_eq!(
            CleanLogEvent::from_tracing_value(&mine, "mine"),
            Some(CleanLogEvent::Start { total: 2 })
        );
        assert_eq!(CleanLogEvent::from_tracing_value(&theirs, "mine"), None);
        assert_eq!(
            CleanLogEvent::from_tracing_value(&other_target, "mine"),
            None
        );
    }

    #[rstest]
    fn read_new_events_extracts_only_matching_run_id() {
        let tmp = TempDir::new().expect("tempdir");
        let log = tmp.path().join("log");
        let target = clean_detached::EVENT_TARGET;
        let mut body = String::new();
        body.push_str(&line(tracing_event(
            target,
            "rid",
            json!({"event": "cc.clean.start", "total": 2}),
        )));
        body.push_str(&line(tracing_event(
            target,
            "other",
            json!({"event": "cc.clean.ok", "path": "/x"}),
        )));
        body.push_str(&line(tracing_event(
            target,
            "rid",
            json!({"event": "cc.clean.ok", "path": "/a"}),
        )));
        fs::write(&log, body).expect("write");

        let (events, _) = read_new_events(&log, 0, "rid").expect("read");
        assert_eq!(
            events,
            vec![
                CleanLogEvent::Start { total: 2 },
                CleanLogEvent::Ok {
                    path: "/a".to_string()
                }
            ]
        );
    }
}
