//! `a cc peer` -- resolve the `SendMessage`/`ListAgents` target name for
//! related Claude Code sessions.
//!
//! `ListAgents` addresses other sessions by a `name` Claude Code derives
//! internally (see `claude_registry`); there is no way to address a session
//! by its `session_id` directly. This joins armyknife's own session store
//! (which tracks parent/child relationships via `ancestor_session_ids`,
//! populated by `a wm new --agent`) against that name so a session can find
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
    /// Print the SendMessage name of the session that delegated to this one
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

/// Prints the resolved name on its own on success so it can be used directly
/// as `SendMessage`'s `to` argument (e.g. `to=$(a cc peer parent)`). Fails
/// loudly instead -- there is no meaningful placeholder to print when no
/// parent is tracked or its name can't be resolved.
fn run_parent() -> Result<()> {
    let self_id = current_session_id()?;
    let session =
        store::load_session(&self_id)?.ok_or_else(|| CcError::SessionNotFound(self_id.clone()))?;
    let parent_id = session
        .ancestor_session_ids
        .last()
        .ok_or_else(|| CcError::NoParentSession(self_id.clone()))?;
    let name = claude_registry::resolve_name(parent_id)
        .ok_or_else(|| CcError::PeerNameUnresolved(parent_id.clone()))?;
    println!("{name}");
    Ok(())
}

fn run_children() -> Result<()> {
    let self_id = current_session_id()?;
    store::cleanup_stale_sessions()?;
    let sessions = store::list_sessions()?;
    let children: Vec<&Session> = sessions
        .iter()
        .filter(|s| s.ancestor_session_ids.last() == Some(&self_id))
        .collect();
    print_peers(&children)
}

fn run_list(args: &PeerListArgs) -> Result<()> {
    store::cleanup_stale_sessions()?;
    let sessions = store::list_sessions()?;
    let filtered: Vec<&Session> = sessions
        .iter()
        .filter(|s| match &args.repo {
            Some(repo) => s.cwd.to_string_lossy().contains(repo.as_str()),
            None => true,
        })
        .collect();
    print_peers(&filtered)
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
    use std::collections::HashMap;
    use std::path::PathBuf;

    use crate::commands::cc::types::SessionStatus;

    fn session(session_id: &str, ancestor_session_ids: Vec<String>) -> Session {
        Session {
            session_id: session_id.to_string(),
            cwd: PathBuf::from("/repo/.worktrees/child"),
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

    #[test]
    fn peer_from_session_resolves_name_from_map() {
        let s = session("child-1", vec!["root".to_string(), "parent-1".to_string()]);
        let name_map = HashMap::from([("child-1".to_string(), "repo-ab".to_string())]);

        assert_eq!(
            Peer::from_session(&s, &name_map),
            Peer {
                name: Some("repo-ab".to_string()),
                session_id: "child-1".to_string(),
                cwd: "/repo/.worktrees/child".to_string(),
                label: Some("my-label".to_string()),
                status: "running",
            }
        );
    }

    #[test]
    fn peer_from_session_leaves_name_none_when_unresolved() {
        let s = session("child-1", vec![]);
        let name_map = HashMap::new();

        assert_eq!(
            Peer::from_session(&s, &name_map),
            Peer {
                name: None,
                session_id: "child-1".to_string(),
                cwd: "/repo/.worktrees/child".to_string(),
                label: Some("my-label".to_string()),
                status: "running",
            }
        );
    }
}
