//! `a cc generate-title-detached` (hidden) subcommand.
//!
//! Non-interactive title generation designed to be spawned in the
//! background by `a cc watch` (see `tui::title_generate::spawn_detached_title_generation`).
//! The caller is responsible for detaching (`setsid`); this command never
//! reads stdin and never writes to stdout/stderr. There is no progress
//! reporting -- the TUI does not wait on or watch this process, it only
//! applies the eventual result directly to the session's `label`.

use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Args;

use crate::commands::cc::{store, window_status};
use crate::commands::name_branch::{Backend, detect_backend};
use crate::infra::tmux;

/// Tracing target for events emitted by this subcommand.
const EVENT_TARGET: &str = "armyknife::commands::cc::generate_title";

#[derive(Args, Clone, PartialEq, Eq)]
pub struct GenerateTitleDetachedArgs {
    /// Session to write the generated title into.
    pub session_id: String,
    /// File containing the LLM prompt (may be multi-KB; avoids argv limits).
    #[arg(long, value_name = "FILE")]
    pub prompt_file: PathBuf,
    /// The session's `label` at the moment Ctrl+g was pressed. The
    /// generated title is applied only if the on-disk label still equals
    /// this snapshot -- protects a manual rename made while generation
    /// was in flight. Absence means the label was unset at request time.
    #[arg(long, value_name = "LABEL")]
    pub previous_label: Option<String>,
}

pub fn run(args: &GenerateTitleDetachedArgs) -> Result<()> {
    // All failures stay off the parent TTY: `cc watch` spawns this process
    // detached and the silent contract is documented at the top of this
    // module.
    let _ = run_inner(args);
    Ok(())
}

fn run_inner(args: &GenerateTitleDetachedArgs) -> Result<()> {
    let backend = detect_backend();
    generate_and_apply(args, backend.as_ref())
}

/// Reads the prompt file, calls `backend.generate`, and -- on success --
/// writes the session's `label` via a compare-and-swap keyed on
/// `args.previous_label`, so a manual rename made while generation was
/// running is never clobbered. Pulled out of `run_inner` so tests can
/// inject a fake `Backend` instead of spawning a real `claude`/`opencode`
/// subprocess.
fn generate_and_apply(args: &GenerateTitleDetachedArgs, backend: &dyn Backend) -> Result<()> {
    let prompt = fs::read_to_string(&args.prompt_file)
        .with_context(|| format!("failed to read prompt file: {}", args.prompt_file.display()))?;

    let title = backend.generate(&prompt).inspect_err(|e| {
        tracing::warn!(
            target: EVENT_TARGET,
            event = "cc.generate_title.backend_err",
            session_id = %args.session_id,
            msg = format!("{e:#}"),
        );
    })?;

    let applied = store::update_session_label_if_unchanged(
        &args.session_id,
        args.previous_label.as_deref(),
        Some(title),
    )?;

    if !applied {
        tracing::info!(
            target: EVENT_TARGET,
            event = "cc.generate_title.stale",
            session_id = %args.session_id,
        );
        return Ok(());
    }

    tracing::info!(
        target: EVENT_TARGET,
        event = "cc.generate_title.applied",
        session_id = %args.session_id,
    );
    sync_tmux_window_title(&args.session_id);

    Ok(())
}

/// Best-effort push of the tmux window title option after the label write.
/// Mirrors `tui::title_edit`'s private `sync_tmux_window_title`, but reads
/// the session fresh from disk (this process has no in-memory `App`
/// state). Silently does nothing without a live tmux pane or an
/// unresolvable window -- this step must never fail the generation.
fn sync_tmux_window_title(session_id: &str) {
    let Ok(Some(session)) = store::load_session(session_id) else {
        return;
    };
    let Some(pane_id) = session.tmux_info.as_ref().map(|info| info.pane_id.as_str()) else {
        return;
    };
    let Some(window_id) = tmux::get_window_id_for_pane(pane_id) else {
        return;
    };
    let Ok(sessions_dir) = store::sessions_dir() else {
        return;
    };
    let _ = window_status::sync_window_option(&window_id, &sessions_dir);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::cc::types::{Session, SessionStatus};
    use rstest::rstest;
    use std::path::Path;
    use tempfile::TempDir;

    /// Test double for `Backend` that returns a fixed result instead of
    /// spawning a real `claude`/`opencode` subprocess.
    struct StubBackend(std::result::Result<String, String>);

    impl Backend for StubBackend {
        fn generate(&self, _prompt: &str) -> anyhow::Result<String> {
            self.0.clone().map_err(|e| anyhow::anyhow!(e))
        }
    }

    fn create_test_session(id: &str, label: Option<&str>) -> Session {
        Session {
            session_id: id.to_string(),
            cwd: PathBuf::from("/tmp/test"),
            transcript_path: None,
            tty: None,
            tmux_info: None,
            status: SessionStatus::Running,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            last_message: None,
            current_tool: None,
            label: label.map(str::to_string),
            ancestor_session_ids: Vec::new(),
            pending_bg_task_ids: std::collections::BTreeSet::new(),
            pending_agent_task_ids: std::collections::BTreeSet::new(),
            read_at: None,
            sweep_signaled: false,
        }
    }

    /// A temp cache root laid out so that, once `XDG_CACHE_HOME` is pointed
    /// at it, `store::sessions_dir()` resolves inside it -- letting
    /// `generate_and_apply`'s real `store` calls round-trip through a real
    /// (but disposable) session file. Mirrors `title_edit.rs`'s
    /// `TempCacheRoot` test helper.
    struct TempCacheRoot {
        #[expect(dead_code, reason = "kept alive to prevent cleanup until dropped")]
        temp_dir: TempDir,
        cache_home: String,
        sessions_dir: PathBuf,
    }

    fn temp_cache_root() -> TempCacheRoot {
        let temp_dir = TempDir::new().expect("temp dir creation should succeed");
        let cache_home = temp_dir.path().to_str().expect("utf8 path").to_string();
        let sessions_dir = temp_dir
            .path()
            .join("armyknife")
            .join("cc")
            .join("sessions");
        TempCacheRoot {
            temp_dir,
            cache_home,
            sessions_dir,
        }
    }

    fn prompt_file(dir: &Path) -> PathBuf {
        let path = dir.join("prompt.txt");
        fs::write(&path, "irrelevant prompt").expect("write prompt file");
        path
    }

    #[rstest]
    fn successful_generation_writes_the_label() {
        let root = temp_cache_root();
        let session = create_test_session("s1", None);
        store::save_session_to(&root.sessions_dir, &session).expect("save should succeed");

        let tmp = TempDir::new().expect("tempdir");
        let args = GenerateTitleDetachedArgs {
            session_id: "s1".to_string(),
            prompt_file: prompt_file(tmp.path()),
            previous_label: None,
        };
        let backend = StubBackend(Ok("Generated Title".to_string()));

        temp_env::with_vars([("XDG_CACHE_HOME", Some(root.cache_home.as_str()))], || {
            generate_and_apply(&args, &backend).expect("generation should succeed");
        });

        let reloaded = store::load_session_from(&root.sessions_dir, "s1")
            .expect("load should succeed")
            .expect("session exists");
        assert_eq!(reloaded.label, Some("Generated Title".to_string()));
    }

    #[rstest]
    fn cas_mismatch_leaves_label_untouched() {
        let root = temp_cache_root();
        let session = create_test_session("s1", Some("Manually Renamed"));
        store::save_session_to(&root.sessions_dir, &session).expect("save should succeed");

        let tmp = TempDir::new().expect("tempdir");
        let args = GenerateTitleDetachedArgs {
            session_id: "s1".to_string(),
            prompt_file: prompt_file(tmp.path()),
            // Snapshot taken before the manual rename above landed.
            previous_label: None,
        };
        let backend = StubBackend(Ok("Generated Title".to_string()));

        temp_env::with_vars([("XDG_CACHE_HOME", Some(root.cache_home.as_str()))], || {
            generate_and_apply(&args, &backend).expect("generation should succeed");
        });

        let reloaded = store::load_session_from(&root.sessions_dir, "s1")
            .expect("load should succeed")
            .expect("session exists");
        assert_eq!(reloaded.label, Some("Manually Renamed".to_string()));
    }

    #[rstest]
    fn backend_failure_leaves_label_untouched() {
        let root = temp_cache_root();
        let session = create_test_session("s1", None);
        store::save_session_to(&root.sessions_dir, &session).expect("save should succeed");

        let tmp = TempDir::new().expect("tempdir");
        let args = GenerateTitleDetachedArgs {
            session_id: "s1".to_string(),
            prompt_file: prompt_file(tmp.path()),
            previous_label: None,
        };
        let backend = StubBackend(Err("claude exited with status 1".to_string()));

        let result =
            temp_env::with_vars([("XDG_CACHE_HOME", Some(root.cache_home.as_str()))], || {
                generate_and_apply(&args, &backend)
            });
        assert!(result.is_err());

        let reloaded = store::load_session_from(&root.sessions_dir, "s1")
            .expect("load should succeed")
            .expect("session exists");
        assert_eq!(reloaded.label, None);
    }

    #[rstest]
    fn missing_prompt_file_returns_error() {
        let root = temp_cache_root();
        let session = create_test_session("s1", None);
        store::save_session_to(&root.sessions_dir, &session).expect("save should succeed");

        let args = GenerateTitleDetachedArgs {
            session_id: "s1".to_string(),
            prompt_file: PathBuf::from("/nonexistent/prompt.txt"),
            previous_label: None,
        };
        let backend = StubBackend(Ok("Generated Title".to_string()));

        let result =
            temp_env::with_vars([("XDG_CACHE_HOME", Some(root.cache_home.as_str()))], || {
                generate_and_apply(&args, &backend)
            });
        assert!(result.is_err());
    }
}
