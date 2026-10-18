//! `a agent peer` -- resolve the `SendMessage`/`ListAgents` target name for
//! related Claude Code sessions.
//!
//! `ListAgents` addresses other sessions by a `name` Claude Code derives
//! internally (see `claude_registry`); there is no way to address a session
//! by its `session_id` directly. This joins armyknife's own session store
//! (which tracks parent/child relationships via `ancestor_session_ids`,
//! populated whenever `a agent new` resolves a parent session -- see
//! `agent::new::delegation::build_ancestor_chain`) against that name so a
//! session can find the right `SendMessage` target for itself.
//!
//! `me` covers the case with no tracked parent/child relationship to join
//! on: a human names an unrelated session by running `a agent peer me` in the
//! pane it's running in, resolving it the same way `a agent resume` does (via
//! the pane's tmux user option, not any session's own env var -- see
//! `resume::resolve_session_id_from_pane`).

use anyhow::Result;
use clap::{Args, Subcommand};
use serde::Serialize;

use super::claude_registry;
use super::error::CcError;
use super::resume;
use super::store;
use super::types::{AgentStatus, Engine, Session};
use crate::shared::env_var::EnvVars;

mod adopt;
pub(crate) mod notify;
mod wake;
use adopt::AdoptArgs;
use notify::NotifyArgs;
use wake::WakeArgs;

#[derive(Subcommand, Clone, PartialEq, Eq)]
pub enum PeerCommands {
    /// List the session that delegated to this one, if any (JSON, a subset
    /// of `list`)
    Parent,

    /// List sessions this one delegated to, with their SendMessage names
    /// (JSON, a subset of `list`)
    Children,

    /// List tracked sessions with their SendMessage names
    List(PeerListArgs),

    /// Print the Claude Code session running in the caller's own tmux pane
    /// (JSON, a subset of `list`) -- for a human to point at "this session"
    /// from inside it, e.g. via bash mode (`!a agent peer me`)
    Me,

    /// Resume a paused peer session by session ID and print its resolved
    /// SendMessage name -- nothing for Codex, which has none (see the `wake`
    /// module doc for why this isn't a flag on `a agent resume`)
    Wake(WakeArgs),

    /// Send a message to another session without going through any session's
    /// own SendMessage tool call (SendMessage socket for Claude Code,
    /// app-server turn steering for Codex, with `codex queue` as a fallback)
    Notify(NotifyArgs),

    /// Adopt an existing session as a direct child of the calling session
    Adopt(AdoptArgs),
}

#[derive(Args, Clone, PartialEq, Eq)]
pub struct PeerListArgs {
    /// Only include sessions whose working directory contains this substring
    /// (e.g. a repo name)
    #[arg(short = 'R', long = "repo")]
    pub repo: Option<String>,
}

/// A session as a `SendMessage` candidate.
///
/// `name` is `None` when Claude Code's own session registry has no matching
/// entry (e.g. the process already exited) -- callers must not treat that as
/// "session doesn't exist", only as "no `SendMessage` target available".
#[derive(Debug, Serialize, PartialEq, Eq)]
struct Peer {
    name: Option<String>,
    session_id: String,
    cwd: String,
    label: Option<String>,
    status: &'static str,
    work_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    agent_status: Option<AgentStatus>,
    /// The tmux pane hosting this session, `None` when it wasn't started
    /// inside tmux. Lets a human match a JSON row against the pane they're
    /// looking at, which `name`/`session_id`/`cwd` alone don't convey.
    pane_id: Option<String>,
    /// Which coding agent CLI this session belongs to -- lets a caller
    /// juggling several peers tell a Codex delegate from a Claude Code one
    /// before deciding how to reach it (e.g. Codex has no `SendMessage`
    /// tool).
    engine: Engine,
}

impl Peer {
    fn from_session(
        session: &Session,
        name_map: &std::collections::HashMap<String, String>,
    ) -> Self {
        Self {
            name: name_map.get(&session.session_id).cloned(),
            session_id: session.session_id.clone(),
            cwd: session.cwd.to_string_lossy().into_owned(),
            label: session.label.clone(),
            status: session.status.display_name(),
            work_type: session.work_type.clone(),
            agent_status: session.agent_status.clone(),
            pane_id: session.tmux_info.as_ref().map(|t| t.pane_id.clone()),
            engine: session.engine,
        }
    }
}

pub fn run(cmd: &PeerCommands) -> Result<()> {
    match cmd {
        PeerCommands::Parent => run_parent(),
        PeerCommands::Children => run_children(),
        PeerCommands::List(args) => run_list(args),
        PeerCommands::Me => run_me(),
        PeerCommands::Wake(args) => wake::run(args),
        PeerCommands::Notify(args) => notify::run(args),
        PeerCommands::Adopt(args) => adopt::run(args),
    }
}

fn current_session_id() -> Result<String> {
    EnvVars::load()
        .own_session_id()
        .ok_or_else(|| CcError::SelfSessionUnknown.into())
}

fn run_parent() -> Result<()> {
    let self_id = current_session_id()?;
    store::cleanup_stale_sessions()?;
    let sessions = store::list_sessions()?;
    let session = find_session_by_id(&sessions, &self_id)?;
    print_peers(&filter_parent(&sessions, session))
}

pub(super) fn resolve_parent_session() -> Result<Session> {
    let self_id = current_session_id()?;
    store::cleanup_stale_sessions()?;
    let sessions = store::list_sessions()?;
    Ok(resolve_parent_in_sessions(&sessions, &self_id)?.clone())
}

fn run_children() -> Result<()> {
    let self_id = current_session_id()?;
    store::cleanup_stale_sessions()?;
    let sessions = store::list_sessions()?;
    print_peers(&filter_children(&sessions, &self_id))
}

fn run_list(args: &PeerListArgs) -> Result<()> {
    store::cleanup_stale_sessions()?;
    let sessions = store::list_sessions()?;
    print_peers(&filter_by_repo(&sessions, args.repo.as_deref()))
}

/// Resolves the session running in the caller's own tmux pane, via the
/// pane's `TMUX_SESSION_OPTION` (see `resume::resolve_session_id_from_pane`)
/// rather than `current_session_id`'s session-ID env var --
/// `me` is meant to be run from a human-typed shell command (e.g. bash mode
/// in the target session's own prompt), which doesn't carry that env var.
fn run_me() -> Result<()> {
    let self_id = resume::resolve_session_id_from_pane()?;
    store::cleanup_stale_sessions()?;
    let sessions = store::list_sessions()?;
    let session = find_session_by_id(&sessions, &self_id)?;
    print_peers(&[session])
}

/// Looks up `self_id` among `sessions`, the "self" lookup shared by
/// `run_parent` and `run_me` -- they differ only in how `self_id` is
/// resolved (env var vs. tmux pane).
fn find_session_by_id<'a>(sessions: &'a [Session], self_id: &str) -> Result<&'a Session> {
    sessions
        .iter()
        .find(|s| s.session_id == self_id)
        .ok_or_else(|| CcError::SessionNotFound(self_id.to_string()).into())
}

/// The session that is `session`'s immediate parent -- a subset of `list`
/// containing zero entries (no parent tracked, or the tracked parent is no
/// longer in `sessions`) or one.
fn filter_parent<'a>(sessions: &'a [Session], session: &Session) -> Vec<&'a Session> {
    find_tracked_parent(sessions, session).into_iter().collect()
}

fn find_tracked_parent<'a>(sessions: &'a [Session], session: &Session) -> Option<&'a Session> {
    let parent_id = session.ancestor_session_ids.last()?;
    sessions.iter().find(|s| &s.session_id == parent_id)
}

fn resolve_tracked_parent<'a>(sessions: &'a [Session], session: &Session) -> Result<&'a Session> {
    let Some(parent_id) = session.ancestor_session_ids.last() else {
        return Err(CcError::NoTrackedParent.into());
    };
    find_tracked_parent(sessions, session)
        .ok_or_else(|| CcError::ParentSessionNotTracked(parent_id.clone()).into())
}

fn resolve_parent_in_sessions<'a>(sessions: &'a [Session], self_id: &str) -> Result<&'a Session> {
    let session = find_session_by_id(sessions, self_id)?;
    resolve_tracked_parent(sessions, session)
}

/// Sessions whose nearest ancestor (immediate parent) is `self_id`.
fn filter_children<'a>(sessions: &'a [Session], self_id: &str) -> Vec<&'a Session> {
    sessions
        .iter()
        .filter(|session| session.is_direct_child_of(self_id))
        .collect()
}

/// Sessions whose working directory contains `repo` as a substring, or all
/// sessions when `repo` is `None`.
fn filter_by_repo<'a>(sessions: &'a [Session], repo: Option<&str>) -> Vec<&'a Session> {
    sessions
        .iter()
        .filter(|s| match repo {
            Some(repo) => s.cwd.to_string_lossy().contains(repo),
            None => true,
        })
        .collect()
}

fn print_peers(sessions: &[&Session]) -> Result<()> {
    let name_map = claude_registry::load_name_map();
    let peers: Vec<Peer> = sessions
        .iter()
        .map(|s| Peer::from_session(s, &name_map))
        .collect();
    println!("{}", serde_json::to_string(&peers)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use rstest::rstest;
    use std::collections::HashMap;
    use std::path::PathBuf;

    use crate::commands::agent::types::{SessionStatus, TmuxInfo};

    fn session(session_id: &str, cwd: &str, ancestor_session_ids: Vec<String>) -> Session {
        Session {
            archived_at: None,
            pr_context: None,
            session_id: session_id.to_string(),
            work_type: None,
            work_type_pinned: false,
            vibe_enabled: false,
            launched_with_vibe: false,
            is_delegated: false,
            crit_urls: Vec::new(),
            pending_human_review_ids: Default::default(),
            cwd: PathBuf::from(cwd),
            transcript_path: None,
            tty: None,
            tmux_info: None,
            status: SessionStatus::Running,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            last_message: None,
            current_tool: None,
            label: Some("my-label".to_string()),
            ancestor_session_ids,
            pending_bg_task_ids: Default::default(),
            pending_agent_task_ids: Default::default(),
            pending_permission_agent_ids: Default::default(),
            pending_permission_request_ids: Default::default(),
            read_at: None,
            sweep_signaled: false,
            stop_blocked_by_armyknife: false,
            agent_status: None,
            agent_status_clear_pending: false,
            engine: Engine::Claude,
        }
    }

    #[rstest]
    #[case::resolved_no_tmux(
        HashMap::from([("child-1".to_string(), "repo-ab".to_string())]),
        None,
        Engine::Claude,
        Some("repo-ab".to_string()),
        None
    )]
    #[case::unresolved_no_tmux(HashMap::new(), None, Engine::Claude, None, None)]
    #[case::resolved_in_tmux(
        HashMap::from([("child-1".to_string(), "repo-ab".to_string())]),
        Some(TmuxInfo {
            session_name: "main".to_string(),
            window_name: "editor".to_string(),
            window_index: 0,
            pane_id: "%3".to_string(),
            socket_path: None,
        }),
        Engine::Claude,
        Some("repo-ab".to_string()),
        Some("%3".to_string())
    )]
    #[case::propagates_codex_engine(HashMap::new(), None, Engine::Codex, None, None)]
    fn peer_from_session(
        #[case] name_map: HashMap<String, String>,
        #[case] tmux_info: Option<TmuxInfo>,
        #[case] engine: Engine,
        #[case] expected_name: Option<String>,
        #[case] expected_pane_id: Option<String>,
    ) {
        let s = Session {
            tmux_info,
            engine,
            ..session("child-1", "/repo/.worktrees/child", vec![])
        };

        assert_eq!(
            Peer::from_session(&s, &name_map),
            Peer {
                name: expected_name,
                session_id: "child-1".to_string(),
                cwd: "/repo/.worktrees/child".to_string(),
                label: Some("my-label".to_string()),
                status: "running",
                work_type: None,
                agent_status: None,
                pane_id: expected_pane_id,
                engine,
            }
        );
    }

    #[test]
    fn peer_json_includes_agent_status_when_set() {
        let mut session = session("status-session", "/repo", vec![]);
        session.agent_status = Some(AgentStatus {
            kind: "external_wait".to_string(),
            note: "waiting for CI".to_string(),
        });

        assert_eq!(
            serde_json::to_value(Peer::from_session(&session, &HashMap::new()))
                .expect("peer should serialize"),
            serde_json::json!({
                "name": null,
                "session_id": "status-session",
                "cwd": "/repo",
                "label": "my-label",
                "status": "running",
                "work_type": null,
                "agent_status": {"kind": "external_wait", "note": "waiting for CI"},
                "pane_id": null,
                "engine": "claude",
            }),
        );
    }

    #[test]
    fn peer_json_omits_agent_status_when_unset() {
        let session = session("status-session", "/repo", vec![]);

        assert_eq!(
            serde_json::to_value(Peer::from_session(&session, &HashMap::new()))
                .expect("peer should serialize"),
            serde_json::json!({
                "name": null,
                "session_id": "status-session",
                "cwd": "/repo",
                "label": "my-label",
                "status": "running",
                "work_type": null,
                "pane_id": null,
                "engine": "claude",
            }),
        );
    }

    #[test]
    fn peer_list_json_identifies_work_type_within_a_repo() {
        let mut project_manager = session(
            "session-a",
            "/work/sample-repo/.worktrees/coordinator",
            vec![],
        );
        project_manager.work_type = Some("project-manager".to_string());
        let worker = session("session-b", "/work/sample-repo/.worktrees/feature", vec![]);
        let sessions = [project_manager, worker];
        let name_map = HashMap::new();
        let peers = serde_json::to_value(
            filter_by_repo(&sessions, Some("sample-repo"))
                .iter()
                .map(|session| Peer::from_session(session, &name_map))
                .collect::<Vec<_>>(),
        )
        .expect("peers should serialize");
        let project_managers = peers
            .as_array()
            .expect("peer list should be an array")
            .iter()
            .filter(|peer| peer["work_type"] == "project-manager")
            .cloned()
            .collect::<Vec<_>>();

        assert_eq!(
            serde_json::json!({
                "peers": peers,
                "project_managers": project_managers,
            }),
            serde_json::json!({
                "peers": [
                    {
                        "name": null,
                        "session_id": "session-a",
                        "cwd": "/work/sample-repo/.worktrees/coordinator",
                        "label": "my-label",
                        "status": "running",
                        "work_type": "project-manager",
                        "pane_id": null,
                        "engine": "claude",
                    },
                    {
                        "name": null,
                        "session_id": "session-b",
                        "cwd": "/work/sample-repo/.worktrees/feature",
                        "label": "my-label",
                        "status": "running",
                        "work_type": null,
                        "pane_id": null,
                        "engine": "claude",
                    },
                ],
                "project_managers": [
                    {
                        "name": null,
                        "session_id": "session-a",
                        "cwd": "/work/sample-repo/.worktrees/coordinator",
                        "label": "my-label",
                        "status": "running",
                        "work_type": "project-manager",
                        "pane_id": null,
                        "engine": "claude",
                    },
                ],
            }),
        );
    }

    #[rstest]
    #[case::returns_the_matching_parent(
        vec!["root".to_string(), "parent-1".to_string()],
        vec!["parent-1"]
    )]
    #[case::empty_when_no_parent_is_tracked(vec![], vec![])]
    #[case::empty_when_the_parent_is_no_longer_listed(
        vec!["root".to_string(), "gone".to_string()],
        vec![]
    )]
    fn filter_parent_cases(
        #[case] ancestor_session_ids: Vec<String>,
        #[case] expected_ids: Vec<&str>,
    ) {
        let sessions = vec![
            session("parent-1", "/repo", vec!["root".to_string()]),
            session("unrelated", "/repo", vec![]),
        ];
        let self_session = session("self", "/repo", ancestor_session_ids);

        let parent = filter_parent(&sessions, &self_session);

        assert_eq!(
            parent
                .iter()
                .map(|s| s.session_id.as_str())
                .collect::<Vec<_>>(),
            expected_ids
        );
    }

    #[rstest]
    #[case::returns_the_tracked_parent(
        vec!["root-session".to_string(), "parent-session".to_string()],
        "child-session",
        Ok("parent-session")
    )]
    #[case::errors_when_no_parent_is_recorded(
        vec![],
        "child-session",
        Err("No tracked parent session is recorded for the current session")
    )]
    #[case::errors_when_parent_is_not_tracked(
        vec!["root-session".to_string(), "missing-parent".to_string()],
        "child-session",
        Err("Parent session 'missing-parent' is not in the tracked session list")
    )]
    #[case::errors_when_current_session_is_not_tracked(
        vec!["root-session".to_string(), "parent-session".to_string()],
        "missing-child",
        Err("Session not found: missing-child")
    )]
    fn resolve_parent_in_sessions_cases(
        #[case] ancestor_session_ids: Vec<String>,
        #[case] self_id: &str,
        #[case] expected: std::result::Result<&str, &str>,
    ) {
        let sessions = vec![
            session("parent-session", "/repo", vec![]),
            session("child-session", "/repo", ancestor_session_ids),
        ];

        let actual = resolve_parent_in_sessions(&sessions, self_id)
            .map(|parent| parent.session_id.as_str())
            .map_err(|error| error.to_string());

        assert_eq!(actual, expected.map_err(str::to_string));
    }

    #[test]
    fn resolve_parent_session_errors_when_current_session_id_is_unknown() {
        let result = temp_env::with_vars(
            [
                ("ARMYKNIFE_SESSION_ID", None::<&str>),
                ("CODEX_SESSION_ID", None::<&str>),
            ],
            resolve_parent_session,
        );

        assert_eq!(
            result
                .map(|session| session.session_id)
                .map_err(|error| error.to_string()),
            Err("Neither ARMYKNIFE_SESSION_ID nor CODEX_SESSION_ID is set (not running inside an armyknife-tracked session)".to_string()),
        );
    }

    #[test]
    fn filter_children_matches_only_immediate_children() {
        let sessions = vec![
            session(
                "child-1",
                "/repo",
                vec!["root".to_string(), "self".to_string()],
            ),
            session(
                "grandchild-1",
                "/repo",
                vec![
                    "root".to_string(),
                    "self".to_string(),
                    "child-1".to_string(),
                ],
            ),
            session(
                "unrelated",
                "/repo",
                vec!["root".to_string(), "other".to_string()],
            ),
        ];

        let children = filter_children(&sessions, "self");

        assert_eq!(
            children
                .iter()
                .map(|s| s.session_id.as_str())
                .collect::<Vec<_>>(),
            vec!["child-1"]
        );
    }

    #[rstest]
    #[case::no_filter_returns_all(None, vec!["a", "b"])]
    #[case::filters_by_cwd_substring(Some("armyknife"), vec!["a"])]
    #[case::matches_nothing(Some("no-such-repo"), vec![])]
    fn filter_by_repo_cases(#[case] repo: Option<&str>, #[case] expected_ids: Vec<&str>) {
        let sessions = vec![
            session("a", "/Users/x/ghq/github.com/fohte/armyknife", vec![]),
            session("b", "/Users/x/ghq/github.com/fohte/dotfiles", vec![]),
        ];

        let filtered = filter_by_repo(&sessions, repo);

        assert_eq!(
            filtered
                .iter()
                .map(|s| s.session_id.as_str())
                .collect::<Vec<_>>(),
            expected_ids
        );
    }
}
