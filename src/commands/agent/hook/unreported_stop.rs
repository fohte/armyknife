use std::path::Path;

use anyhow::Result;

use super::super::store;
use super::super::types::{ChildLaunchPermission, HookEvent, HookInput, Session};
use crate::shared::config::AgentConfig;
use crate::shared::sanitize::strip_angle_brackets;

pub(super) fn can_check_stop(event: HookEvent, blocked_by_armyknife: bool) -> bool {
    event == HookEvent::Stop && !blocked_by_armyknife
}

pub(super) fn notify_if_unreported(
    session: &Session,
    input: &HookInput,
    sessions_dir: &Path,
    agent_config: &AgentConfig,
    mut notify: impl FnMut(&str, &str),
) -> Result<()> {
    let Some(parent_session_id) = session.ancestor_session_ids.last() else {
        return Ok(());
    };
    if session.child_launch != ChildLaunchPermission::Deny || has_pending_work(session, input) {
        return Ok(());
    }

    let has_reported = store::check_unreported_stop(
        sessions_dir,
        &session.session_id,
        parent_session_id,
        session.created_at,
    )?;
    if has_reported {
        tracing::debug!(
            event = "agent.hook.unreported_stop.skipped",
            session = %session.session_id,
            parent_session = %parent_session_id,
            reason = "reported",
        );
        return Ok(());
    }

    let Some(template) = agent_config.unreported_stop_notification.as_deref() else {
        tracing::info!(
            event = "agent.hook.unreported_stop.skipped",
            session = %session.session_id,
            parent_session = %parent_session_id,
            reason = "template_unconfigured",
        );
        return Ok(());
    };
    let session_id = strip_angle_brackets(&session.session_id);
    let final_message = input
        .last_assistant_message
        .as_deref()
        .filter(|message| !message.is_empty())
        .or(session.last_message.as_deref())
        .unwrap_or_default();
    let last_message = strip_angle_brackets(final_message);
    let message = template
        .replace("{session_id}", &session_id)
        .replace("{last_message}", &last_message);
    notify(parent_session_id, &message);
    Ok(())
}

fn has_pending_work(session: &Session, input: &HookInput) -> bool {
    session.has_pending_bg_tasks()
        || !input.background_tasks.is_empty()
        || !input.session_crons.is_empty()
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use chrono::{DateTime, TimeZone, Utc};
    use indoc::indoc;
    use rstest::{fixture, rstest};
    use tempfile::TempDir;

    use super::*;
    use crate::commands::agent::store::{
        check_unreported_stop, insert_watch_web_peer_out_message_from,
    };
    use crate::commands::agent::types::{ChildLaunchPermission, Engine, SessionStatus};

    const TEMPLATE: &str = indoc! {"
        <delegation-update>
        Session: {session_id}
        Last message: {last_message}
        </delegation-update>"
    };

    #[rstest]
    #[case::eligible(HookEvent::Stop, false, true)]
    #[case::armyknife_blocked(HookEvent::Stop, true, false)]
    #[case::other_event(HookEvent::PreToolUse, false, false)]
    fn checks_only_unblocked_stop_events(
        #[case] event: HookEvent,
        #[case] blocked_by_armyknife: bool,
        #[case] expected: bool,
    ) {
        assert_eq!(can_check_stop(event, blocked_by_armyknife), expected);
    }

    #[rstest]
    #[case::claude(Engine::Claude)]
    #[case::codex(Engine::Codex)]
    fn sends_the_full_rendered_notification_to_the_direct_parent(
        #[case] engine: Engine,
        sessions_dir: TempDir,
    ) {
        let session = delegate_session(engine, "fixture-delegate", "fixture-parent");
        let calls = Arc::new(Mutex::new(Vec::new()));
        let calls_for_notify = Arc::clone(&calls);
        notify_if_unreported(
            &session,
            &stop_input(engine),
            sessions_dir.path(),
            &config(Some(TEMPLATE)),
            move |recipient, message| {
                calls_for_notify
                    .lock()
                    .unwrap()
                    .push((recipient.to_string(), message.to_string()));
            },
        )
        .unwrap();

        assert_eq!(
            *calls.lock().unwrap(),
            vec![(
                "fixture-parent".to_string(),
                indoc! {"
                    <delegation-update>
                    Session: fixture-delegate
                    Last message: I finished sample change.
                    </delegation-update>"
                }
                .to_string(),
            )],
        );
    }

    #[rstest]
    #[case::claude(Engine::Claude)]
    #[case::codex(Engine::Codex)]
    fn stop_hook_sends_the_full_rendered_notification_to_the_direct_parent(
        #[case] engine: Engine,
        sessions_dir: TempDir,
    ) {
        let mut session = delegate_session(engine, "fixture-delegate", "fixture-parent");
        session.status = SessionStatus::Running;
        store::save_session_to(sessions_dir.path(), &session).unwrap();

        let calls = Arc::new(Mutex::new(Vec::new()));
        let calls_for_notify = Arc::clone(&calls);
        let agent_config = config(Some(TEMPLATE));
        let notify_hook = move |session: &Session, input: &HookInput, sessions_dir: &Path| {
            notify_if_unreported(
                session,
                input,
                sessions_dir,
                &agent_config,
                |recipient, message| {
                    calls_for_notify
                        .lock()
                        .unwrap()
                        .push((recipient.to_string(), message.to_string()));
                },
            )
        };
        let result = super::super::process_hook_event_impl_with_env_and_unreported_stop(
            HookEvent::Stop,
            stop_input(engine),
            sessions_dir.path(),
            &super::super::side_effects::SideEffects::none(),
            None,
            crate::shared::env_var::EnvVars::default(),
            notify_hook,
        )
        .unwrap();

        assert_eq!(
            (result, calls.lock().unwrap().clone()),
            (
                super::super::ProcessResult::SessionSaved,
                vec![(
                    "fixture-parent".to_string(),
                    indoc! {"
                        <delegation-update>
                        Session: fixture-delegate
                        Last message: I finished sample change.
                        </delegation-update>"
                    }
                    .to_string(),
                )],
            ),
        );
    }

    #[rstest]
    #[case::delivered(false)]
    #[case::failed(true)]
    fn does_not_notify_after_a_parent_report(#[case] failed: bool, sessions_dir: TempDir) {
        let session = delegate_session(Engine::Claude, "fixture-delegate", "fixture-parent");
        insert_watch_web_peer_out_message_from(
            sessions_dir.path(),
            &session.session_id,
            "fixture-parent",
            "A fictional report.",
            at(12, 0),
            failed,
        )
        .unwrap();
        let calls = Arc::new(Mutex::new(Vec::new()));
        let calls_for_notify = Arc::clone(&calls);
        notify_if_unreported(
            &session,
            &stop_input(Engine::Claude),
            sessions_dir.path(),
            &config(Some(TEMPLATE)),
            move |recipient, message| {
                calls_for_notify
                    .lock()
                    .unwrap()
                    .push((recipient.to_string(), message.to_string()));
            },
        )
        .unwrap();

        assert_eq!(*calls.lock().unwrap(), Vec::<(String, String)>::new());
    }

    #[rstest]
    fn notifies_after_a_parent_instruction_and_deferred_stop_without_a_new_report(
        sessions_dir: TempDir,
    ) {
        let mut session = delegate_session(Engine::Claude, "fixture-delegate", "fixture-parent");
        let first_check = check_unreported_stop(
            sessions_dir.path(),
            &session.session_id,
            "fixture-parent",
            session.created_at,
        )
        .unwrap();
        insert_watch_web_peer_out_message_from(
            sessions_dir.path(),
            &session.session_id,
            "fixture-parent",
            "A fictional report.",
            at(11, 0),
            false,
        )
        .unwrap();
        insert_watch_web_peer_out_message_from(
            sessions_dir.path(),
            "fixture-parent",
            &session.session_id,
            "A fictional follow-up instruction.",
            at(12, 0),
            true,
        )
        .unwrap();

        session
            .pending_bg_task_ids
            .insert("fixture-task".to_string());
        notify_if_unreported(
            &session,
            &stop_input(Engine::Claude),
            sessions_dir.path(),
            &config(Some(TEMPLATE)),
            |_, _| {},
        )
        .unwrap();
        session.pending_bg_task_ids.clear();

        let calls = Arc::new(Mutex::new(Vec::new()));
        let calls_for_notify = Arc::clone(&calls);
        notify_if_unreported(
            &session,
            &stop_input(Engine::Claude),
            sessions_dir.path(),
            &config(Some(TEMPLATE)),
            move |recipient, message| {
                calls_for_notify
                    .lock()
                    .unwrap()
                    .push((recipient.to_string(), message.to_string()));
            },
        )
        .unwrap();

        assert_eq!(
            (first_check, calls.lock().unwrap().clone()),
            (
                false,
                vec![(
                    "fixture-parent".to_string(),
                    indoc! {"
                        <delegation-update>
                        Session: fixture-delegate
                        Last message: I finished sample change.
                        </delegation-update>"
                    }
                    .to_string(),
                )],
            ),
        );
    }

    #[rstest]
    fn counts_a_report_sent_before_a_deferred_stop_and_ignores_the_previous_turn(
        sessions_dir: TempDir,
    ) {
        let session = delegate_session(Engine::Claude, "fixture-delegate", "fixture-parent");
        insert_watch_web_peer_out_message_from(
            sessions_dir.path(),
            &session.session_id,
            "fixture-parent",
            "A report from the previous turn.",
            at(11, 0),
            false,
        )
        .unwrap();
        check_unreported_stop(
            sessions_dir.path(),
            &session.session_id,
            "fixture-parent",
            session.created_at,
        )
        .unwrap();
        insert_watch_web_peer_out_message_from(
            sessions_dir.path(),
            &session.session_id,
            "fixture-parent",
            "A report before the deferred stop.",
            at(12, 0),
            false,
        )
        .unwrap();
        let mut deferred_session = session.clone();
        deferred_session
            .pending_bg_task_ids
            .insert("fixture-task".to_string());
        notify_if_unreported(
            &deferred_session,
            &stop_input(Engine::Claude),
            sessions_dir.path(),
            &config(Some(TEMPLATE)),
            |_, _| {},
        )
        .unwrap();
        deferred_session.pending_bg_task_ids.clear();
        let calls = Arc::new(Mutex::new(Vec::new()));
        let calls_for_notify = Arc::clone(&calls);
        notify_if_unreported(
            &deferred_session,
            &stop_input(Engine::Claude),
            sessions_dir.path(),
            &config(Some(TEMPLATE)),
            move |recipient, message| {
                calls_for_notify
                    .lock()
                    .unwrap()
                    .push((recipient.to_string(), message.to_string()));
            },
        )
        .unwrap();

        assert_eq!(*calls.lock().unwrap(), Vec::<(String, String)>::new());
    }

    #[rstest]
    #[case::no_parent(None, ChildLaunchPermission::Deny)]
    #[case::child_launch_allowed(Some("fixture-parent"), ChildLaunchPermission::Allow)]
    fn skips_sessions_that_are_not_delegates(
        #[case] parent: Option<&str>,
        #[case] child_launch: ChildLaunchPermission,
        sessions_dir: TempDir,
    ) {
        let mut session = delegate_session(Engine::Claude, "fixture-delegate", "fixture-parent");
        session.ancestor_session_ids = parent.into_iter().map(str::to_string).collect();
        session.child_launch = child_launch;
        let calls = Arc::new(Mutex::new(Vec::new()));
        let calls_for_notify = Arc::clone(&calls);
        notify_if_unreported(
            &session,
            &stop_input(Engine::Claude),
            sessions_dir.path(),
            &config(Some(TEMPLATE)),
            move |recipient, message| {
                calls_for_notify
                    .lock()
                    .unwrap()
                    .push((recipient.to_string(), message.to_string()));
            },
        )
        .unwrap();

        assert_eq!(*calls.lock().unwrap(), Vec::<(String, String)>::new());
    }

    #[rstest]
    #[case::background_tasks(true, false)]
    #[case::session_crons(false, true)]
    fn pending_work_does_not_advance_the_report_boundary(
        #[case] background_tasks: bool,
        #[case] session_crons: bool,
        sessions_dir: TempDir,
    ) {
        let session_without_report =
            delegate_session(Engine::Claude, "fixture-pending-delegate", "fixture-parent");
        let input = stop_input_with_pending_work(Engine::Claude, background_tasks, session_crons);
        let calls = Arc::new(Mutex::new(Vec::new()));
        let calls_for_notify = Arc::clone(&calls);
        notify_if_unreported(
            &session_without_report,
            &input,
            sessions_dir.path(),
            &config(Some(TEMPLATE)),
            move |recipient, message| {
                calls_for_notify
                    .lock()
                    .unwrap()
                    .push((recipient.to_string(), message.to_string()));
            },
        )
        .unwrap();

        let session_with_report = delegate_session(
            Engine::Claude,
            "fixture-reported-delegate",
            "fixture-parent",
        );
        insert_watch_web_peer_out_message_from(
            sessions_dir.path(),
            &session_with_report.session_id,
            "fixture-parent",
            "A report before the pending check.",
            at(12, 0),
            false,
        )
        .unwrap();
        let calls_for_pending = Arc::clone(&calls);
        notify_if_unreported(
            &session_with_report,
            &input,
            sessions_dir.path(),
            &config(Some(TEMPLATE)),
            |recipient, message| {
                calls_for_pending
                    .lock()
                    .unwrap()
                    .push((recipient.to_string(), message.to_string()));
            },
        )
        .unwrap();
        let calls_for_next_stop = Arc::clone(&calls);
        notify_if_unreported(
            &session_with_report,
            &stop_input(Engine::Claude),
            sessions_dir.path(),
            &config(Some(TEMPLATE)),
            move |recipient, message| {
                calls_for_next_stop
                    .lock()
                    .unwrap()
                    .push((recipient.to_string(), message.to_string()));
            },
        )
        .unwrap();

        assert_eq!(*calls.lock().unwrap(), Vec::<(String, String)>::new());
    }

    #[rstest]
    fn does_not_send_without_a_template_and_records_the_check(sessions_dir: TempDir) {
        let session = delegate_session(Engine::Claude, "fixture-delegate", "fixture-parent");
        let calls = Arc::new(Mutex::new(Vec::new()));
        let calls_for_notify = Arc::clone(&calls);
        notify_if_unreported(
            &session,
            &stop_input(Engine::Claude),
            sessions_dir.path(),
            &config(None),
            move |recipient, message| {
                calls_for_notify
                    .lock()
                    .unwrap()
                    .push((recipient.to_string(), message.to_string()));
            },
        )
        .unwrap();

        insert_watch_web_peer_out_message_from(
            sessions_dir.path(),
            &session.session_id,
            "fixture-parent",
            "A fictional report after the first check.",
            at(12, 1)
                .checked_add_signed(chrono::Duration::seconds(30))
                .unwrap(),
            false,
        )
        .unwrap();
        notify_if_unreported(
            &session,
            &stop_input(Engine::Claude),
            sessions_dir.path(),
            &config(Some(TEMPLATE)),
            |recipient, message| {
                calls
                    .lock()
                    .unwrap()
                    .push((recipient.to_string(), message.to_string()));
            },
        )
        .unwrap();

        assert_eq!(*calls.lock().unwrap(), Vec::<(String, String)>::new());
    }

    #[rstest]
    #[case::absent(None)]
    #[case::empty(Some(""))]
    fn renders_the_session_message_when_the_stop_payload_has_none(
        #[case] input_message: Option<&str>,
        sessions_dir: TempDir,
    ) {
        let mut session = delegate_session(Engine::Claude, "fixture-delegate", "fixture-parent");
        session.last_message = Some("Saved final response.".to_string());
        let mut input = stop_input(Engine::Claude);
        input.last_assistant_message = input_message.map(str::to_string);
        let calls = Arc::new(Mutex::new(Vec::new()));
        let calls_for_notify = Arc::clone(&calls);
        notify_if_unreported(
            &session,
            &input,
            sessions_dir.path(),
            &config(Some(TEMPLATE)),
            move |recipient, message| {
                calls_for_notify
                    .lock()
                    .unwrap()
                    .push((recipient.to_string(), message.to_string()));
            },
        )
        .unwrap();

        assert_eq!(
            *calls.lock().unwrap(),
            vec![(
                "fixture-parent".to_string(),
                indoc! {"
                    <delegation-update>
                    Session: fixture-delegate
                    Last message: Saved final response.
                    </delegation-update>"
                }
                .to_string(),
            )],
        );
    }

    #[fixture]
    fn sessions_dir() -> TempDir {
        tempfile::tempdir().unwrap()
    }

    fn config(template: Option<&str>) -> AgentConfig {
        AgentConfig {
            unreported_stop_notification: template.map(str::to_string),
            ..AgentConfig::default()
        }
    }

    fn delegate_session(engine: Engine, session_id: &str, parent_id: &str) -> Session {
        Session {
            session_id: session_id.to_string(),
            launch_id: None,
            archived_at: None,
            pr_context: None,
            created_pr_urls: Vec::new(),
            crit_urls: Vec::new(),
            crit_not_waiting_urls: Default::default(),
            web_uis: Vec::new(),
            pending_human_review_ids: Default::default(),
            cwd: "/fictional/repo".into(),
            transcript_path: None,
            tty: None,
            tmux_info: None,
            status: SessionStatus::Stopped,
            agent_status: None,
            agent_status_clear_pending: false,
            created_at: at(10, 0),
            updated_at: at(12, 0),
            last_prompted_at: None,
            last_message: Some("A stale previous message.".to_string()),
            current_tool: None,
            current_tool_name: None,
            label: None,
            work_type: None,
            vibe_enabled: false,
            launched_with_vibe: false,
            work_type_pinned: false,
            child_launch: ChildLaunchPermission::Deny,
            ancestor_session_ids: vec!["fixture-root".to_string(), parent_id.to_string()],
            pending_bg_task_ids: Default::default(),
            pending_agent_task_ids: Default::default(),
            pending_permission_agent_ids: Default::default(),
            pending_permission_request_ids: Default::default(),
            read_at: None,
            sweep_signaled: false,
            stop_blocked_by_armyknife: false,
            engine,
            model: None,
            effort: None,
        }
    }

    fn stop_input(engine: Engine) -> HookInput {
        serde_json::from_value(serde_json::json!({
            "session_id": "fixture-delegate",
            "cwd": "/fictional/repo",
            "engine": match engine {
                Engine::Claude => "claude",
                Engine::Codex => "codex",
            },
            "last_assistant_message": "I finished <sample> change.",
        }))
        .unwrap()
    }

    fn stop_input_with_pending_work(
        engine: Engine,
        background_tasks: bool,
        session_crons: bool,
    ) -> HookInput {
        serde_json::from_value(serde_json::json!({
            "session_id": "fixture-delegate",
            "cwd": "/fictional/repo",
            "engine": match engine {
                Engine::Claude => "claude",
                Engine::Codex => "codex",
            },
            "background_tasks": if background_tasks {
                serde_json::json!([{ "id": "fixture-task", "type": "shell" }])
            } else {
                serde_json::json!([])
            },
            "session_crons": if session_crons {
                serde_json::json!([{ "id": "fixture-cron" }])
            } else {
                serde_json::json!([])
            },
        }))
        .unwrap()
    }

    fn at(hour: u32, minute: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 6, 1, hour, minute, 0).unwrap()
    }
}
