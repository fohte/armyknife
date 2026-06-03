//! Detached `a cc clean-detached` spawn + JSONL log tailing.
//!
//! The clean view spawns the cleanup as a fully detached process (its own
//! session, stdio redirected to `/dev/null`) so that closing `cc watch`
//! never aborts an in-flight cleanup. While `cc watch` is alive, a small
//! polling thread tails the child's per-PID JSONL log and forwards each
//! event to the UI for the bottom-bar progress display.

use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::shared::cache;

/// Logs older than this are GC'd on `cc watch` startup. Mirrors the TTL
/// the `clean-detached` subcommand itself uses so both ends agree.
pub const LOG_TTL: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// How often the tail thread polls the log file.
pub const TAIL_INTERVAL: Duration = Duration::from_millis(500);

/// Default `~/.cache/armyknife/clean/` location used by detached runs.
pub fn log_dir() -> Option<PathBuf> {
    cache::base_dir().map(|d| d.join("clean"))
}

/// Single event recorded by `clean-detached` in JSONL form. `tag` mirrors
/// the `event` field in the producer code.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(tag = "event", rename_all = "lowercase")]
pub enum CleanLogEvent {
    Start { total: usize },
    Ok { path: String },
    Err { path: String, msg: String },
    Done { ok: usize, failed: usize },
}

/// Aggregated state derived from a stream of [`CleanLogEvent`]s. The UI
/// renders this directly in the bottom bar.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CleanProgress {
    pub pid: u32,
    pub total: usize,
    pub completed: usize,
    pub failed: usize,
    pub last_path: Option<String>,
    pub done: bool,
    /// Pending delete notifications. The UI drains this on every apply
    /// pass to drop matching rows from the in-memory worktree list
    /// without re-running discovery; the field is intentionally not a
    /// cumulative record.
    pub deleted_paths: Vec<String>,
    /// Cumulative log of paths the child has confirmed deleted across
    /// the entire run. Survives drains so callers (e.g. re-entering
    /// the clean view mid-cleanup) can filter out stale entries from
    /// a fresh worktree snapshot.
    pub confirmed_deleted: Vec<String>,
}

impl CleanProgress {
    pub fn new(pid: u32) -> Self {
        Self {
            pid,
            ..Self::default()
        }
    }

    /// Fold one event into the progress state.
    pub fn apply(&mut self, event: &CleanLogEvent) {
        // Producer guarantees `Done` is final; ignore stragglers so a
        // misbehaving log cannot mutate the displayed summary.
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

    /// Bottom-bar text for the live progress phase.
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

/// Summary of the most recently completed clean run, used for the
/// one-shot "last clean: N ok, M failed" banner on watch startup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LastCleanSummary {
    pub log_path: PathBuf,
    pub ok: usize,
    pub failed: usize,
}

impl LastCleanSummary {
    pub fn message(&self) -> String {
        format!("Last clean: {} ok, {} failed", self.ok, self.failed)
    }
}

/// Spawn `a cc clean-detached` as a fully detached process. The child
/// gets its own session (so `cc watch` exiting cannot send it SIGHUP),
/// its working directory is `/`, and stdio is wired to `/dev/null`.
///
/// `paths` is passed via a temporary `--paths-file` so we do not stuff
/// thousands of paths through argv. The temp file is deleted by the
/// child once it has been read (best-effort).
pub fn spawn_detached_clean(paths: &[PathBuf]) -> Result<u32> {
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};

    let exe = std::env::current_exe().context("failed to resolve current exe")?;

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

    let mut cmd = Command::new(&exe);
    cmd.arg("cc")
        .arg("clean-detached")
        .arg("--paths-file")
        .arg(&file_path)
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

    let child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            // Roll back the persisted paths file; without the child it
            // would only ever be cleaned up by the OS `/tmp` GC.
            let _ = fs::remove_file(&file_path);
            return Err(anyhow::Error::new(e).context("failed to spawn clean-detached child"));
        }
    };

    Ok(child.id())
}

/// Read a JSONL log file and parse all events that have appeared since
/// the previous read. Returns the new events plus the new cursor offset
/// the caller should pass in next time.
pub fn read_new_events(path: &Path, mut cursor: u64) -> Result<(Vec<CleanLogEvent>, u64)> {
    let mut file = match File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok((Vec::new(), cursor));
        }
        Err(e) => return Err(e).context("failed to open clean log"),
    };
    let len = file.metadata().context("failed to stat clean log")?.len();
    if len < cursor {
        // The producer truncated/rotated the file; restart from the top.
        cursor = 0;
    }
    file.seek(SeekFrom::Start(cursor))
        .context("failed to seek clean log")?;
    let mut buf = String::new();
    file.read_to_string(&mut buf)
        .context("failed to read clean log")?;
    // Only consume complete lines so partial writes do not produce
    // parse errors; the unterminated tail stays for next poll.
    let (consumable, _trailing) = match buf.rfind('\n') {
        Some(idx) => (&buf[..=idx], &buf[idx + 1..]),
        None => ("", buf.as_str()),
    };
    let mut events = Vec::new();
    for line in consumable.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        // Skip undecodable lines instead of aborting: one corrupted line
        // must not stop the live progress display.
        if let Ok(ev) = serde_json::from_str::<CleanLogEvent>(trimmed) {
            events.push(ev);
        }
    }
    let new_cursor = cursor + consumable.len() as u64;
    Ok((events, new_cursor))
}

/// On watch startup, find the most recently completed clean log,
/// return its summary, and delete it ("read receipt"). Walks
/// newest-first so an in-progress log from a concurrent watch never
/// masks an older completed log.
pub fn pop_last_summary(dir: &Path) -> Option<LastCleanSummary> {
    let entries = fs::read_dir(dir).ok()?;
    let mut candidates: Vec<(SystemTime, PathBuf)> = entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("jsonl") {
                return None;
            }
            let modified = entry.metadata().ok()?.modified().ok()?;
            Some((modified, path))
        })
        .collect();
    candidates.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    for (_, path) in candidates {
        if let Some(summary) = extract_summary(&path) {
            // Consume only completed logs; partial logs stay until
            // they finish or are GC'd by TTL.
            let _ = fs::remove_file(&path);
            return Some(summary);
        }
    }
    None
}

fn extract_summary(path: &Path) -> Option<LastCleanSummary> {
    let file = File::open(path).ok()?;
    for line in BufReader::new(file).lines().map_while(Result::ok) {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Ok(CleanLogEvent::Done { ok, failed }) =
            serde_json::from_str::<CleanLogEvent>(trimmed)
        {
            return Some(LastCleanSummary {
                log_path: path.to_path_buf(),
                ok,
                failed,
            });
        }
    }
    None
}

/// Removes `*.jsonl` files older than `ttl` from `dir`. Best-effort.
/// `cc clean-detached` runs the same GC at its own end; doing it here
/// too means a never-spawned-again system still drains the directory.
pub fn gc_old_logs(dir: &Path, ttl: Duration, now: SystemTime) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use indoc::indoc;
    use rstest::rstest;
    use tempfile::TempDir;

    #[rstest]
    fn apply_aggregates_events() {
        let mut p = CleanProgress::new(42);
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

        assert_eq!(p.pid, 42);
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
        let mut p = CleanProgress::new(1);
        p.apply(&CleanLogEvent::Start { total: 1 });
        p.apply(&CleanLogEvent::Done { ok: 1, failed: 0 });
        // Producer normally guarantees nothing comes after Done; verify
        // the guard so a misbehaving log cannot corrupt the summary.
        p.apply(&CleanLogEvent::Ok {
            path: "/late".to_string(),
        });
        assert_eq!(p.completed, 1);
        assert!(p.deleted_paths.is_empty());
        assert!(p.confirmed_deleted.is_empty());
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
            pid: 1,
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
    fn read_new_events_returns_complete_lines_only() {
        let tmp = TempDir::new().expect("tempdir");
        let log = tmp.path().join("1.jsonl");
        let payload = indoc! {r#"
            {"event":"start","ts":"2024-01-01T00:00:00Z","total":2}
            {"event":"ok","ts":"2024-01-01T00:00:01Z","path":"/a"}
        "#};
        fs::write(&log, payload).expect("write");

        let (events, cursor) = read_new_events(&log, 0).expect("read");
        assert_eq!(events.len(), 2);
        assert_eq!(cursor, payload.len() as u64);

        // No new lines after the cursor → empty result.
        let (events, cursor2) = read_new_events(&log, cursor).expect("read2");
        assert!(events.is_empty());
        assert_eq!(cursor2, cursor);
    }

    #[rstest]
    fn read_new_events_skips_partial_tail() {
        let tmp = TempDir::new().expect("tempdir");
        let log = tmp.path().join("1.jsonl");
        // Two complete lines + one in-progress write without terminator.
        let payload = concat!(
            r#"{"event":"start","ts":"2024-01-01T00:00:00Z","total":2}"#,
            "\n",
            r#"{"event":"ok","ts":"2024-01-01T00:00:01Z","path":"/a"}"#,
            "\n",
            r#"{"event":"ok","ts":"2024-01-"#,
        );
        fs::write(&log, payload).expect("write");

        let (events, cursor) = read_new_events(&log, 0).expect("read");
        assert_eq!(events.len(), 2);
        // Cursor stops at the last newline so the partial tail will be
        // retried next poll once the producer flushes it.
        assert!(cursor < payload.len() as u64);
    }

    #[rstest]
    fn read_new_events_handles_missing_file() {
        let tmp = TempDir::new().expect("tempdir");
        let log = tmp.path().join("missing.jsonl");
        let (events, cursor) = read_new_events(&log, 0).expect("read");
        assert!(events.is_empty());
        assert_eq!(cursor, 0);
    }

    #[rstest]
    fn read_new_events_skips_corrupt_lines() {
        let tmp = TempDir::new().expect("tempdir");
        let log = tmp.path().join("1.jsonl");
        let payload = indoc! {r#"
            {"event":"start","ts":"2024-01-01T00:00:00Z","total":1}
            not-json-at-all
            {"event":"done","ts":"2024-01-01T00:00:01Z","ok":1,"failed":0}
        "#};
        fs::write(&log, payload).expect("write");

        let (events, _) = read_new_events(&log, 0).expect("read");
        assert_eq!(events.len(), 2);
        assert!(matches!(events[0], CleanLogEvent::Start { total: 1 }));
        assert!(matches!(
            events[1],
            CleanLogEvent::Done { ok: 1, failed: 0 }
        ));
    }

    #[rstest]
    fn pop_last_summary_returns_and_removes_done_log() {
        let tmp = TempDir::new().expect("tempdir");
        let log = tmp.path().join("123.jsonl");
        let payload = indoc! {r#"
            {"event":"start","ts":"2024-01-01T00:00:00Z","total":2}
            {"event":"ok","ts":"2024-01-01T00:00:01Z","path":"/a"}
            {"event":"done","ts":"2024-01-01T00:00:02Z","ok":1,"failed":1}
        "#};
        fs::write(&log, payload).expect("write");

        let summary = pop_last_summary(tmp.path()).expect("summary");
        assert_eq!(summary.ok, 1);
        assert_eq!(summary.failed, 1);
        assert_eq!(summary.message(), "Last clean: 1 ok, 1 failed");
        // Reading consumed the file.
        assert!(!log.exists());
    }

    #[rstest]
    fn pop_last_summary_ignores_incomplete_log() {
        let tmp = TempDir::new().expect("tempdir");
        let log = tmp.path().join("321.jsonl");
        let payload = indoc! {r#"
            {"event":"start","ts":"2024-01-01T00:00:00Z","total":1}
            {"event":"ok","ts":"2024-01-01T00:00:01Z","path":"/a"}
        "#};
        fs::write(&log, payload).expect("write");

        assert!(pop_last_summary(tmp.path()).is_none());
        assert!(log.exists()); // Not consumed because no done event.
    }

    #[rstest]
    fn pop_last_summary_picks_newest() {
        let tmp = TempDir::new().expect("tempdir");
        let older = tmp.path().join("100.jsonl");
        let newer = tmp.path().join("200.jsonl");
        let payload = |ok: usize| {
            format!(
                "{{\"event\":\"start\",\"ts\":\"2024-01-01T00:00:00Z\",\"total\":1}}\n\
                 {{\"event\":\"done\",\"ts\":\"2024-01-01T00:00:01Z\",\"ok\":{ok},\"failed\":0}}\n"
            )
        };
        fs::write(&older, payload(7)).expect("write older");
        std::thread::sleep(std::time::Duration::from_millis(20));
        fs::write(&newer, payload(11)).expect("write newer");

        let summary = pop_last_summary(tmp.path()).expect("summary");
        assert_eq!(summary.ok, 11);
        assert!(!newer.exists());
        // Older log is left for a future startup or the time-based GC.
        assert!(older.exists());
    }

    #[rstest]
    fn pop_last_summary_skips_in_progress_log_to_surface_older_done() {
        // A concurrent watch's in-progress log must not mask a prior
        // completed log: the user would otherwise never see the
        // summary even though it is on disk.
        let tmp = TempDir::new().expect("tempdir");
        let done = tmp.path().join("100.jsonl");
        let in_progress = tmp.path().join("200.jsonl");
        fs::write(
            &done,
            indoc! {r#"
                {"event":"start","ts":"2024-01-01T00:00:00Z","total":1}
                {"event":"done","ts":"2024-01-01T00:00:01Z","ok":3,"failed":0}
            "#},
        )
        .expect("write done");
        std::thread::sleep(std::time::Duration::from_millis(20));
        fs::write(
            &in_progress,
            indoc! {r#"
                {"event":"start","ts":"2024-01-01T01:00:00Z","total":5}
                {"event":"ok","ts":"2024-01-01T01:00:01Z","path":"/x"}
            "#},
        )
        .expect("write in-progress");

        let summary = pop_last_summary(tmp.path()).expect("summary");
        assert_eq!(summary.ok, 3);
        // Done log consumed; in-progress log preserved.
        assert!(!done.exists());
        assert!(in_progress.exists());
    }

    #[rstest]
    #[case::expired(Duration::from_secs(60 * 60 * 24 * 8), false)]
    #[case::fresh(Duration::from_secs(60), true)]
    fn gc_old_logs_respects_ttl(#[case] age: Duration, #[case] should_remain: bool) {
        let tmp = TempDir::new().expect("tempdir");
        let target = tmp.path().join("42.jsonl");
        fs::write(&target, "{}\n").expect("write");
        let f = File::open(&target).expect("open");
        f.set_modified(SystemTime::now() - age).expect("set time");
        drop(f);

        gc_old_logs(tmp.path(), LOG_TTL, SystemTime::now());
        assert_eq!(target.exists(), should_remain);
    }

    #[rstest]
    fn gc_old_logs_ignores_non_jsonl() {
        let tmp = TempDir::new().expect("tempdir");
        let other = tmp.path().join("note.txt");
        fs::write(&other, "x").expect("write");
        let f = File::open(&other).expect("open");
        f.set_modified(SystemTime::now() - Duration::from_secs(60 * 60 * 24 * 30))
            .expect("set time");
        drop(f);

        gc_old_logs(tmp.path(), LOG_TTL, SystemTime::now());
        assert!(other.exists());
    }
}
