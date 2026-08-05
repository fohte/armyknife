//! Ctrl+g title generation from a session's transcript, triggered from
//! `AppMode::Edit` (see `title_edit`).
//!
//! Generation itself never runs inside `cc watch`: pressing Ctrl+g builds
//! the prompt and returns to `AppMode::Normal` immediately, and the actual
//! LLM call happens in a fully detached `a cc generate-title-detached`
//! process (see `spawn_detached_title_generation`) that keeps running even
//! if the whole TUI is closed. There is deliberately no in-process
//! progress tracking or result channel -- the detached process applies the
//! generated title directly to the session's `label` on disk.

use anyhow::{Context, Result};
use indoc::formatdoc;

use crate::commands::cc::claude_sessions;

use super::app::{App, AppMode};

/// Everything needed to spawn the detached title-generation process for
/// one Ctrl+g press.
#[derive(Debug, PartialEq, Eq)]
pub struct SpawnTitleGenerationRequest {
    pub session_id: String,
    pub prompt: String,
    /// The session's `label` at the moment Ctrl+g was pressed. Passed
    /// through to the detached process so it can no-op if the label has
    /// since changed (e.g. a manual rename while generation was running).
    pub previous_label: Option<String>,
}

/// Handles `Ctrl+g` from `AppMode::Edit`: reads the session's first user
/// message and last assistant message from its `.jsonl`, builds the LLM
/// prompt, and snapshots the session's current `label` (so the eventual
/// detached write can no-op if the user renames it manually before
/// generation finishes). Returns `None` (after setting an error) when
/// neither transcript message is available, or the app isn't in
/// `AppMode::Edit` for a known session. Does not spawn anything itself --
/// callers hand the returned request to `spawn_detached_title_generation`.
pub(super) fn request_generate_title(app: &mut App) -> Option<SpawnTitleGenerationRequest> {
    let session_id = match &app.mode {
        AppMode::Edit { session_id } => session_id.clone(),
        _ => return None,
    };
    let session = app.sessions.iter().find(|s| s.session_id == session_id)?;
    let cwd = session.cwd.clone();
    // The on-disk label at request time, not `app.edit_title_query` -- the
    // edit buffer may already hold unsaved keystrokes that are not `label`.
    let previous_label = session.label.clone();

    let first_user_message = claude_sessions::get_first_user_message(&cwd, &session_id);
    let last_assistant_message = claude_sessions::get_last_assistant_message(&cwd, &session_id);

    if first_user_message.is_none() && last_assistant_message.is_none() {
        app.set_error("No conversation content to generate a title from".to_string());
        return None;
    }

    let prompt = build_prompt(
        first_user_message.as_deref().unwrap_or(""),
        last_assistant_message.as_deref().unwrap_or(""),
    );

    Some(SpawnTitleGenerationRequest {
        session_id,
        prompt,
        previous_label,
    })
}

/// Builds the LLM prompt for generating a session title from its transcript.
pub fn build_prompt(first_user_message: &str, last_assistant_message: &str) -> String {
    formatdoc! {r#"
        Task: Generate a short session title in Japanese describing what the user is working on in this Claude Code session.

        Requirements:
        - Japanese, 10-25 characters
        - Include concrete identifiers (repository, feature, command name) when available
        - No trailing punctuation
        - One line only

        <first-user-message>
        {first_user_message}
        </first-user-message>

        <latest-assistant-message>
        {last_assistant_message}
        </latest-assistant-message>

        IMPORTANT: Output ONLY the title. Do not explain."#}
}

/// Spawns `a cc generate-title-detached` as a fully detached process,
/// mirroring `clean_progress::spawn_detached_clean` (own session via
/// `setsid`, cwd `/`, stdio to `/dev/null`) so title generation survives
/// `cc watch` exiting. The prompt is written to a persisted temp file
/// (never unlinked -- relies on OS `/tmp` GC, same policy as the
/// clean-detached paths file) since it can exceed a comfortable argv size.
pub(super) fn spawn_detached_title_generation(request: SpawnTitleGenerationRequest) -> Result<()> {
    use std::io::Write;
    use std::os::unix::process::CommandExt;
    use std::process::Stdio;

    use crate::shared::command;

    let exe = std::env::current_exe().context("failed to resolve current exe")?;

    // Persisted (not auto-deleted) -- the child only reads this file and
    // never unlinks it. Cleanup relies on OS `/tmp` GC.
    let mut prompt_file =
        tempfile::NamedTempFile::new().context("failed to create temp prompt file")?;
    prompt_file
        .write_all(request.prompt.as_bytes())
        .context("failed to write prompt file")?;
    prompt_file.flush().context("failed to flush prompt file")?;
    let (_file, file_path) = prompt_file
        .keep()
        .map_err(|e| anyhow::anyhow!("failed to persist prompt file: {e}"))?;

    let mut cmd = command::new(&exe);
    cmd.arg("cc")
        .arg("generate-title-detached")
        .arg(&request.session_id)
        .arg("--prompt-file")
        .arg(&file_path)
        .current_dir("/")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Some(label) = &request.previous_label {
        cmd.arg("--previous-label").arg(label);
    }

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
        Ok(_child) => Ok(()),
        Err(e) => {
            // Roll back the persisted prompt file; without the child it
            // would only ever be cleaned up by the OS `/tmp` GC.
            let _ = std::fs::remove_file(&file_path);
            Err(anyhow::Error::new(e).context("failed to spawn generate-title-detached child"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::cc::types::{Session, SessionStatus};
    use std::fs;
    use std::path::PathBuf;

    fn test_session(id: &str) -> Session {
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
            label: None,
            ancestor_session_ids: Vec::new(),
            pending_bg_task_ids: std::collections::BTreeSet::new(),
            pending_agent_task_ids: std::collections::BTreeSet::new(),
            read_at: None,
            sweep_signaled: false,
        }
    }

    #[test]
    fn build_prompt_includes_both_messages() {
        let prompt = build_prompt("fix the login bug", "I've updated auth.rs");

        assert_eq!(
            prompt,
            formatdoc! {r#"
                Task: Generate a short session title in Japanese describing what the user is working on in this Claude Code session.

                Requirements:
                - Japanese, 10-25 characters
                - Include concrete identifiers (repository, feature, command name) when available
                - No trailing punctuation
                - One line only

                <first-user-message>
                fix the login bug
                </first-user-message>

                <latest-assistant-message>
                I've updated auth.rs
                </latest-assistant-message>

                IMPORTANT: Output ONLY the title. Do not explain."#}
        );
    }

    #[test]
    fn request_generate_title_is_noop_outside_edit_mode() {
        let mut app = App::with_sessions(vec![test_session("s1")]);

        let request = request_generate_title(&mut app);

        assert!(request.is_none());
    }

    #[test]
    fn request_generate_title_sets_error_without_transcript() {
        let mut app = App::with_sessions(vec![test_session("s1")]);
        app.enter_edit_title();

        let request = request_generate_title(&mut app);

        assert_eq!(
            (request, app.error_message.clone()),
            (
                None,
                Some("No conversation content to generate a title from".to_string())
            )
        );
    }

    /// Creates a `.claude/projects/{encoded}/{session_id}.jsonl` fixture
    /// under `home_dir`, mirroring `claude_sessions.rs`'s own test harness --
    /// `get_first_user_message`/`get_last_assistant_message` resolve the
    /// project directory from `HOME`, so this is the only way to exercise
    /// `request_generate_title`'s success path without a fake reader.
    fn create_test_project_with_jsonl(
        home_dir: &std::path::Path,
        project_path: &std::path::Path,
        session_id: &str,
        jsonl_content: &str,
    ) {
        let encoded = claude_sessions::encode_project_path(project_path);
        let project_dir = home_dir.join(".claude").join("projects").join(&encoded);
        fs::create_dir_all(&project_dir).expect("create project dir");
        let jsonl_path = project_dir.join(format!("{session_id}.jsonl"));
        fs::write(&jsonl_path, jsonl_content).expect("write jsonl fixture");
    }

    #[test]
    fn request_generate_title_snapshots_the_on_disk_label() {
        use indoc::indoc;
        use tempfile::TempDir;

        let temp_dir = TempDir::new().expect("tempdir");
        let home_dir = temp_dir.path();
        let cwd = PathBuf::from("/test/project");

        let mut session = test_session("s1");
        session.cwd = cwd.clone();
        session.label = Some("Old Title".to_string());
        let mut app = App::with_sessions(vec![session]);
        app.enter_edit_title();

        create_test_project_with_jsonl(
            home_dir,
            &cwd,
            "s1",
            indoc! {r#"
                {"type":"user","message":{"content":"Fix the login bug"}}
                {"type":"assistant","message":{"content":[{"type":"text","text":"Updated auth.rs"}]}}
            "#},
        );

        let request = temp_env::with_vars(
            [("HOME", Some(home_dir.to_str().expect("utf8 path")))],
            || request_generate_title(&mut app),
        );

        assert_eq!(
            (request, app.mode.clone()),
            (
                Some(SpawnTitleGenerationRequest {
                    session_id: "s1".to_string(),
                    prompt: build_prompt("Fix the login bug", "Updated auth.rs"),
                    previous_label: Some("Old Title".to_string()),
                }),
                AppMode::Edit {
                    session_id: "s1".to_string()
                },
            )
        );
    }
}
