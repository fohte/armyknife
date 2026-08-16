//! `a cc peer` -- resolve the `SendMessage`/`ListAgents` target name for
//! related Claude Code sessions.
//!
//! `ListAgents` addresses other sessions by a `name` Claude Code derives
//! internally (see `claude_registry`); there is no way to address a session
//! by its `session_id` directly. This joins armyknife's own session store
//! (which tracks parent/child relationships via `ancestor_session_ids`,
//! populated whenever `a wm new` resolves a parent session -- see
//! `wm::new::build_ancestor_chain`) against that name so a session can find
//! the right `SendMessage` target for itself.

use anyhow::Result;
use clap::{Args, Subcommand};
use serde::Serialize;

use super::claude_registry;
use super::error::CcError;
use super::store;
use super::types::Session;
use crate::shared::env_var::EnvVars;

#[derive(Subcommand, Clone, PartialEq, Eq)]
pub enum PeerCommands {
    /// List the session that delegated to this one, if any (JSON, a subset
    /// of `list`)
    Parent,

    /// List the SendMessage names of sessions this one delegated to
    Children,

    /// List tracked sessions with their SendMessage names
    List(PeerListArgs),
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
        }
    }
}

pub fn run(cmd: &PeerCommands) -> Result<()> {
    match cmd {
        PeerCommands::Parent => run_parent(),
        PeerCommands::Children => run_children(),
        PeerCommands::List(args) => run_list(args),
    }
}

fn current_session_id() -> Result<String> {
    EnvVars::load()
        .session_id
        .ok_or_else(|| CcError::SelfSessionUnknown.into())
}

fn run_parent() -> Result<()> {
    let self_id = current_session_id()?;
    store::cleanup_stale_sessions()?;
    let sessions = store::list_sessions()?;
    let session = sessions
        .iter()
        .find(|s| s.session_id == self_id)
        .ok_or_else(|| CcError::SessionNotFound(self_id.clone()))?;
    print_peers(&filter_parent(&sessions, session))
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

/// The session that is `session`'s immediate parent -- a subset of `list`
/// containing zero entries (no parent tracked) or one.
fn filter_parent<'a>(sessions: &'a [Session], session: &Session) -> Vec<&'a Session> {
    let Some(parent_id) = session.ancestor_session_ids.last() else {
        return Vec::new();
    };
    sessions
        .iter()
        .filter(|s| &s.session_id == parent_id)
        .collect()
}

/// Sessions whose nearest ancestor (immediate parent) is `self_id`.
fn filter_children<'a>(sessions: &'a [Session], self_id: &str) -> Vec<&'a Session> {
    sessions
        .iter()
        .filter(|s| s.ancestor_session_ids.last().map(String::as_str) == Some(self_id))
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

    use crate::commands::cc::types::SessionStatus;

    fn session(session_id: &str, cwd: &str, ancestor_session_ids: Vec<String>) -> Session {
        Session {
            session_id: session_id.to_string(),
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
            read_at: None,
            sweep_signaled: false,
        }
    }

    #[rstest]
    #[case::resolved(
        HashMap::from([("child-1".to_string(), "repo-ab".to_string())]),
        Some("repo-ab".to_string())
    )]
    #[case::unresolved(HashMap::new(), None)]
    fn peer_from_session_name(
        #[case] name_map: HashMap<String, String>,
        #[case] expected_name: Option<String>,
    ) {
        let s = session("child-1", "/repo/.worktrees/child", vec![]);

        assert_eq!(
            Peer::from_session(&s, &name_map),
            Peer {
                name: expected_name,
                session_id: "child-1".to_string(),
                cwd: "/repo/.worktrees/child".to_string(),
                label: Some("my-label".to_string()),
                status: "running",
            }
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
