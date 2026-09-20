use std::collections::BTreeSet;
use std::path::Path;

use anyhow::Result;
use chrono::Utc;

use crate::commands::agent::store;
use crate::commands::agent::types::{Engine, Session, SessionStatus};
use crate::infra::tmux::layout::AgentLaunchRoute;
use crate::shared::env_var::EnvVars;

pub(super) fn record_codex_daemon_metadata(
    route: &AgentLaunchRoute,
    cwd: &str,
    env_vars: &[(&str, &str)],
) {
    let AgentLaunchRoute::CodexDaemon { thread_id } = route else {
        return;
    };

    if let Err(error) = record_from_env(thread_id, Path::new(cwd), env_vars) {
        eprintln!(
            "[armyknife] warning: failed to save delegation metadata for Codex session {thread_id}: {error}"
        );
    }
}

fn record_from_env(session_id: &str, cwd: &Path, env_vars: &[(&str, &str)]) -> Result<()> {
    let label = env_vars
        .iter()
        .find(|(key, _)| *key == EnvVars::session_label_name())
        .map(|(_, value)| *value)
        .filter(|value| !value.is_empty());
    let ancestor_session_ids = env_vars
        .iter()
        .find(|(key, _)| *key == EnvVars::ancestor_session_ids_name())
        .map(|(_, value)| parse_ancestor_session_ids(value))
        .unwrap_or_default();

    record_in(
        &store::sessions_dir()?,
        session_id,
        cwd,
        label,
        &ancestor_session_ids,
    )
}

fn parse_ancestor_session_ids(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_string)
        .collect()
}

fn record_in(
    sessions_dir: &Path,
    session_id: &str,
    cwd: &Path,
    label: Option<&str>,
    ancestor_session_ids: &[String],
) -> Result<()> {
    let session_lock = store::lock_session_for_update(sessions_dir, session_id)?;
    let now = Utc::now();
    let mut session = session_lock.load()?.unwrap_or_else(|| Session {
        session_id: session_id.to_string(),
        cwd: cwd.to_path_buf(),
        transcript_path: None,
        tty: None,
        tmux_info: None,
        status: SessionStatus::Running,
        created_at: now,
        updated_at: now,
        last_message: None,
        current_tool: None,
        label: None,
        ancestor_session_ids: Vec::new(),
        pending_bg_task_ids: BTreeSet::new(),
        pending_agent_task_ids: BTreeSet::new(),
        pending_permission_agent_ids: BTreeSet::new(),
        read_at: None,
        sweep_signaled: false,
        engine: Engine::Codex,
    });

    if let Some(label) = label {
        session.label = Some(label.to_string());
    }
    if !ancestor_session_ids.is_empty() {
        session.ancestor_session_ids = ancestor_session_ids.to_vec();
    }

    session_lock.save(&session)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use rstest::{fixture, rstest};
    use serde_json::{Value, json};
    use std::path::PathBuf;
    use tempfile::TempDir;

    #[fixture]
    fn sessions_dir() -> TempDir {
        TempDir::new().expect("temp dir creation should succeed")
    }

    fn existing_session() -> Session {
        let timestamp = Utc
            .with_ymd_and_hms(2026, 1, 2, 3, 4, 5)
            .single()
            .expect("timestamp should be valid");
        Session {
            session_id: "thread-a".to_string(),
            cwd: PathBuf::from("/workspace/original"),
            transcript_path: Some(PathBuf::from("/tmp/transcript.jsonl")),
            tty: Some("/dev/ttys001".to_string()),
            tmux_info: None,
            status: SessionStatus::Stopped,
            created_at: timestamp,
            updated_at: timestamp,
            last_message: Some("done".to_string()),
            current_tool: None,
            label: Some("generated label".to_string()),
            ancestor_session_ids: vec!["existing-parent".to_string()],
            pending_bg_task_ids: BTreeSet::new(),
            pending_agent_task_ids: BTreeSet::new(),
            pending_permission_agent_ids: BTreeSet::new(),
            read_at: None,
            sweep_signaled: false,
            engine: Engine::Codex,
        }
    }

    fn session_json(sessions_dir: &Path) -> Value {
        let session = store::load_session_from(sessions_dir, "thread-a")
            .expect("load should succeed")
            .expect("session should exist");
        serde_json::to_value(session).expect("session should serialize")
    }

    #[rstest]
    fn creates_session_before_hook_runs(sessions_dir: TempDir) {
        record_in(
            sessions_dir.path(),
            "thread-a",
            Path::new("/workspace/delegate"),
            Some("explicit label"),
            &["root".to_string(), "parent".to_string()],
        )
        .expect("record should succeed");

        let mut actual = session_json(sessions_dir.path());
        actual["created_at"] = json!("<timestamp>");
        actual["updated_at"] = json!("<timestamp>");
        assert_eq!(
            actual,
            json!({
                "session_id": "thread-a",
                "cwd": "/workspace/delegate",
                "transcript_path": null,
                "tty": null,
                "tmux_info": null,
                "status": "running",
                "created_at": "<timestamp>",
                "updated_at": "<timestamp>",
                "last_message": null,
                "current_tool": null,
                "label": "explicit label",
                "ancestor_session_ids": ["root", "parent"],
                "pending_bg_task_ids": [],
                "pending_agent_task_ids": [],
                "pending_permission_agent_ids": [],
                "read_at": null,
                "sweep_signaled": false,
                "engine": "codex"
            })
        );
    }

    #[rstest]
    #[case::explicit_metadata(
        Some("explicit label"),
        &["new-parent"],
        Some("explicit label"),
        &["new-parent"]
    )]
    #[case::omitted_label_preserves_generated(
        None,
        &["new-parent"],
        Some("generated label"),
        &["new-parent"]
    )]
    #[case::empty_ancestors_preserve_existing(
        Some("explicit label"),
        &[],
        Some("explicit label"),
        &["existing-parent"]
    )]
    fn updates_only_non_empty_metadata_after_hook_runs(
        sessions_dir: TempDir,
        #[case] label: Option<&str>,
        #[case] ancestors: &[&str],
        #[case] expected_label: Option<&str>,
        #[case] expected_ancestors: &[&str],
    ) {
        let existing = existing_session();
        store::save_session_to(sessions_dir.path(), &existing).expect("save should succeed");
        let ancestors = ancestors
            .iter()
            .map(|id| (*id).to_string())
            .collect::<Vec<_>>();

        record_in(
            sessions_dir.path(),
            "thread-a",
            Path::new("/workspace/delegate"),
            label,
            &ancestors,
        )
        .expect("record should succeed");

        let mut expected = serde_json::to_value(existing).expect("session should serialize");
        expected["label"] = json!(expected_label);
        expected["ancestor_session_ids"] = json!(expected_ancestors);
        assert_eq!(session_json(sessions_dir.path()), expected);
    }

    #[rstest]
    fn metadata_survives_later_hook_style_update(sessions_dir: TempDir) {
        record_in(
            sessions_dir.path(),
            "thread-a",
            Path::new("/workspace/delegate"),
            None,
            &["parent".to_string()],
        )
        .expect("record should succeed");
        let mut expected = session_json(sessions_dir.path());
        expected["status"] = json!("stopped");

        let session_lock = store::lock_session_for_update(sessions_dir.path(), "thread-a")
            .expect("lock should succeed");
        let mut session = session_lock
            .load()
            .expect("load should succeed")
            .expect("session should exist");
        session.status = SessionStatus::Stopped;
        session_lock.save(&session).expect("save should succeed");
        drop(session_lock);

        let actual = session_json(sessions_dir.path());
        assert_eq!(actual, expected);
    }

    #[rstest]
    #[case::trims_ids(" root, parent ", &["root", "parent"])]
    #[case::empty_value("", &[])]
    fn parses_ancestor_session_ids(#[case] input: &str, #[case] expected: &[&str]) {
        assert_eq!(parse_ancestor_session_ids(input), expected);
    }
}
