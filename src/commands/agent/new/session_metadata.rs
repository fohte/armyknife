use std::collections::BTreeSet;
use std::path::Path;

use anyhow::Result;
use chrono::Utc;

use crate::commands::agent::store;
use crate::commands::agent::types::{Engine, Session, SessionStatus};
use crate::infra::tmux::layout::AgentLaunchRoute;
use crate::shared::env_var::{EnvVars, parse_ancestor_session_ids};

pub(super) fn record_codex_daemon_metadata(
    route: &AgentLaunchRoute,
    cwd: &str,
    env_vars: &[(&str, &str)],
) {
    let Some(thread_id) = route.codex_thread_id() else {
        return;
    };

    if let Err(error) = record_from_env(thread_id, Path::new(cwd), env_vars) {
        eprintln!(
            "[armyknife] warning: failed to save session metadata for Codex session {thread_id}: {error}"
        );
    }
}

fn record_from_env(session_id: &str, cwd: &Path, env_vars: &[(&str, &str)]) -> Result<()> {
    let sessions_dir = store::sessions_dir()?;
    record_from_env_in(&sessions_dir, session_id, cwd, env_vars)
}

fn record_from_env_in(
    sessions_dir: &Path,
    session_id: &str,
    cwd: &Path,
    env_vars: &[(&str, &str)],
) -> Result<()> {
    let metadata = metadata_from_env(env_vars);
    if metadata.label.is_none()
        && metadata.work_type.is_none()
        && metadata.ancestor_session_ids.is_empty()
    {
        return Ok(());
    }

    record_in(sessions_dir, session_id, cwd, metadata)
}

#[derive(Debug, PartialEq, Eq)]
struct SessionMetadata<'a> {
    label: Option<&'a str>,
    work_type: Option<&'a str>,
    ancestor_session_ids: Vec<String>,
}

fn non_empty_env<'a>(env_vars: &[(&str, &'a str)], name: &str) -> Option<&'a str> {
    env_vars
        .iter()
        .find(|(key, _)| *key == name)
        .map(|(_, value)| *value)
        .filter(|value| !value.is_empty())
}

fn metadata_from_env<'a>(env_vars: &[(&str, &'a str)]) -> SessionMetadata<'a> {
    SessionMetadata {
        label: non_empty_env(env_vars, EnvVars::session_label_name()),
        work_type: non_empty_env(env_vars, EnvVars::session_work_type_name()),
        ancestor_session_ids: non_empty_env(env_vars, EnvVars::ancestor_session_ids_name())
            .map(parse_ancestor_session_ids)
            .unwrap_or_default(),
    }
}

fn record_in(
    sessions_dir: &Path,
    session_id: &str,
    cwd: &Path,
    metadata: SessionMetadata<'_>,
) -> Result<()> {
    let session_lock = store::lock_session_for_update(sessions_dir, session_id)?;
    let now = Utc::now();
    let mut session = session_lock.load()?.unwrap_or_else(|| Session {
        session_id: session_id.to_string(),
        work_type: None,
        work_type_pinned: false,
        crit_urls: Vec::new(),
        pending_human_review_ids: Default::default(),
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
        pending_permission_request_ids: Default::default(),
        read_at: None,
        sweep_signaled: false,
        engine: Engine::Codex,
    });

    if let Some(label) = metadata.label {
        session.label = Some(label.to_string());
    }
    // An explicit kind must win if a hook persisted a detected skill first.
    if let Some(work_type) = metadata.work_type {
        session.work_type = Some(work_type.to_string());
        session.work_type_pinned = true;
    }
    if !metadata.ancestor_session_ids.is_empty() {
        session.ancestor_session_ids = metadata.ancestor_session_ids;
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
            work_type: Some("detected-skill".to_string()),
            work_type_pinned: false,
            crit_urls: Vec::new(),
            pending_human_review_ids: Default::default(),
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
            pending_permission_request_ids: Default::default(),
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
    #[case::with_work_type(Some("sample-skill"), Some("sample-skill"), true)]
    #[case::without_work_type(None, None, false)]
    fn creates_session_before_hook_runs(
        sessions_dir: TempDir,
        #[case] work_type: Option<&str>,
        #[case] expected_work_type: Option<&str>,
        #[case] expected_work_type_pinned: bool,
    ) {
        record_in(
            sessions_dir.path(),
            "thread-a",
            Path::new("/workspace/delegate"),
            SessionMetadata {
                label: Some("explicit label"),
                work_type,
                ancestor_session_ids: vec!["root".to_string(), "parent".to_string()],
            },
        )
        .expect("record should succeed");

        let mut actual = session_json(sessions_dir.path());
        actual["created_at"] = json!("<timestamp>");
        actual["updated_at"] = json!("<timestamp>");
        let mut expected = json!({
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
            "work_type": expected_work_type,
            "ancestor_session_ids": ["root", "parent"],
            "pending_bg_task_ids": [],
            "pending_agent_task_ids": [],
            "pending_permission_agent_ids": [],
            "read_at": null,
            "sweep_signaled": false,
            "engine": "codex"
        });
        if expected_work_type_pinned {
            expected["work_type_pinned"] = json!(true);
        }
        assert_eq!(actual, expected);
    }

    #[rstest]
    fn records_kind_without_other_metadata(sessions_dir: TempDir) {
        let env_vars = [(EnvVars::session_work_type_name(), "sample-skill")];
        record_from_env_in(
            sessions_dir.path(),
            "thread-a",
            Path::new("/workspace/delegate"),
            &env_vars,
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
                "label": null,
                "work_type": "sample-skill",
                "work_type_pinned": true,
                "ancestor_session_ids": [],
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
    #[case::explicit_metadata_overrides_detected_kind(
        Some("explicit label"),
        Some("sample-skill"),
        &["new-parent"],
        Some("explicit label"),
        Some("sample-skill"),
        &["new-parent"]
    )]
    #[case::omitted_metadata_preserves_existing(
        None,
        None,
        &["new-parent"],
        Some("generated label"),
        Some("detected-skill"),
        &["new-parent"]
    )]
    #[case::empty_ancestors_preserve_ancestor_chain(
        Some("explicit label"),
        Some("sample-skill"),
        &[],
        Some("explicit label"),
        Some("sample-skill"),
        &["existing-parent"]
    )]
    fn updates_metadata_after_hook_runs(
        sessions_dir: TempDir,
        #[case] label: Option<&str>,
        #[case] work_type: Option<&str>,
        #[case] ancestors: &[&str],
        #[case] expected_label: Option<&str>,
        #[case] expected_work_type: Option<&str>,
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
            SessionMetadata {
                label,
                work_type,
                ancestor_session_ids: ancestors,
            },
        )
        .expect("record should succeed");

        let mut expected = serde_json::to_value(existing).expect("session should serialize");
        expected["label"] = json!(expected_label);
        expected["work_type"] = json!(expected_work_type);
        expected["ancestor_session_ids"] = json!(expected_ancestors);
        if work_type.is_some() {
            expected["work_type_pinned"] = json!(true);
        }
        assert_eq!(session_json(sessions_dir.path()), expected);
    }

    #[rstest]
    fn metadata_survives_later_hook_style_update(sessions_dir: TempDir) {
        record_in(
            sessions_dir.path(),
            "thread-a",
            Path::new("/workspace/delegate"),
            SessionMetadata {
                label: None,
                work_type: None,
                ancestor_session_ids: vec!["parent".to_string()],
            },
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

    #[rstest]
    fn extracts_delegation_metadata_from_env() {
        let env_vars = [
            ("UNRELATED", "value"),
            (EnvVars::session_label_name(), "delegate label"),
            (EnvVars::session_work_type_name(), "sample-skill"),
            (EnvVars::ancestor_session_ids_name(), "root, parent"),
        ];

        assert_eq!(
            metadata_from_env(&env_vars),
            SessionMetadata {
                label: Some("delegate label"),
                work_type: Some("sample-skill"),
                ancestor_session_ids: vec!["root".to_string(), "parent".to_string()],
            }
        );
    }
}
