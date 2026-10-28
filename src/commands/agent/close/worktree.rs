use std::future::Future;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use super::child;
use super::{CloseArgs, EnvVars, LiveCloseRuntime, Session, close_tracked_session};
use super::{prepare_close_session, worktree_delete};
use crate::infra::git::GitRepo;

pub(super) async fn close_worktree(
    worktree_root: &Path,
    args: &CloseArgs,
    refuse_current_session: bool,
    child_parent_session_id: Option<&str>,
) -> Result<()> {
    let sessions = super::super::store::list_sessions()?;
    let matching_sessions = select_worktree_sessions(&sessions, worktree_root);
    let caller_pane_id = if !matching_sessions.is_empty() {
        crate::shared::caller_pane::resolve_caller_pane_id()
    } else {
        None
    };
    let caller_session_id = if refuse_current_session {
        EnvVars::load().own_session_id()
    } else {
        None
    };
    let context = WorktreeCloseContext {
        args,
        sessions: &matching_sessions,
        worktree_root,
        child_parent_session_id,
        caller_session_id: caller_session_id.as_deref(),
        caller_pane_id: caller_pane_id.as_deref(),
    };
    let skip_hooks = args.skip_hooks;
    close_worktree_with(
        context,
        refuse_current_session,
        worktree_delete::prepare(worktree_root, args.force),
        |session, force, current_pane_id| {
            validate_worktree_session(session, force, current_pane_id, &LiveCloseRuntime)
        },
        super::super::store::load_session,
        close_worktree_session,
        |plan| async move { worktree_delete::execute(plan, skip_hooks).await },
    )
    .await
}

pub(crate) async fn close_worktree_path(worktree_path: &Path) -> Result<()> {
    let args = CloseArgs::default();
    close_worktree(worktree_path, &args, true, None).await
}

struct WorktreeCloseContext<'a> {
    args: &'a CloseArgs,
    sessions: &'a [&'a Session],
    worktree_root: &'a Path,
    child_parent_session_id: Option<&'a str>,
    caller_session_id: Option<&'a str>,
    caller_pane_id: Option<&'a str>,
}

async fn close_worktree_with<P, F>(
    context: WorktreeCloseContext<'_>,
    refuse_current_session: bool,
    prepare: impl Future<Output = Result<P>>,
    mut validate_session: impl FnMut(&Session, bool, Option<&str>) -> Result<()>,
    mut load_session: impl FnMut(&str) -> Result<Option<Session>>,
    mut close_session: impl FnMut(&Session, bool, Option<&str>) -> Result<()>,
    remove_worktree: impl FnOnce(P) -> F,
) -> Result<()>
where
    F: Future<Output = Result<()>>,
{
    validate_worktree_sessions(
        context.sessions,
        &context,
        refuse_current_session,
        &mut validate_session,
    )?;
    close_with_worktree_plan(
        prepare,
        || {
            let mut refreshed_sessions = Vec::with_capacity(context.sessions.len());
            for session in context.sessions {
                let latest_session = load_session(&session.session_id)?.with_context(|| {
                    format!(
                        "Could not reload agent session `{}` after worktree confirmation",
                        session.session_id
                    )
                })?;
                if path_is_within_worktree(&latest_session.cwd, context.worktree_root) {
                    refreshed_sessions.push(latest_session);
                }
            }
            let refreshed_session_refs = refreshed_sessions.iter().collect::<Vec<&Session>>();
            validate_worktree_sessions(
                &refreshed_session_refs,
                &context,
                refuse_current_session,
                &mut validate_session,
            )?;
            close_worktree_sessions(
                &refreshed_sessions,
                context.args.force,
                context.caller_pane_id,
                &mut close_session,
            )
        },
        remove_worktree,
    )
    .await
}

fn validate_worktree_sessions(
    sessions: &[&Session],
    context: &WorktreeCloseContext<'_>,
    refuse_current_session: bool,
    validate_session: &mut impl FnMut(&Session, bool, Option<&str>) -> Result<()>,
) -> Result<()> {
    child::ensure_target_sessions(sessions, context.child_parent_session_id)?;
    if refuse_current_session {
        ensure_not_closing_current_session(
            sessions,
            context.caller_session_id,
            context.caller_pane_id,
        )?;
    }
    for session in sessions {
        if should_gracefully_close_worktree_session(session, context.caller_pane_id) {
            validate_session(session, context.args.force, context.caller_pane_id)?;
        }
    }
    Ok(())
}

fn close_worktree_sessions(
    sessions: &[Session],
    force: bool,
    current_pane_id: Option<&str>,
    close_session: &mut impl FnMut(&Session, bool, Option<&str>) -> Result<()>,
) -> Result<()> {
    let mut closed_session_ids = Vec::new();
    for session in sessions {
        if !should_gracefully_close_worktree_session(session, current_pane_id) {
            continue;
        }
        if let Err(error) = close_session(session, force, current_pane_id) {
            if closed_session_ids.is_empty() {
                return Err(error);
            }
            return Err(error).with_context(|| {
                format!(
                    "Failed to close agent session `{}`; already closed sessions: {}",
                    session.session_id,
                    closed_session_ids.join(", ")
                )
            });
        }
        closed_session_ids.push(session.session_id.clone());
    }
    Ok(())
}

fn ensure_not_closing_current_session(
    sessions: &[&Session],
    caller_session_id: Option<&str>,
    current_pane_id: Option<&str>,
) -> Result<()> {
    if sessions.iter().any(|session| {
        caller_session_id == Some(session.session_id.as_str())
            || is_caller_pane_session(session, current_pane_id)
    }) {
        bail!("The target worktree contains the caller's current session; skipping close");
    }
    Ok(())
}

pub(super) async fn close_with_worktree_plan<P, F>(
    prepare: impl Future<Output = Result<P>>,
    close_session: impl FnOnce() -> Result<()>,
    remove_worktree: impl FnOnce(P) -> F,
) -> Result<()>
where
    F: Future<Output = Result<()>>,
{
    let plan = prepare.await?;
    close_session()?;
    remove_worktree(plan).await
}

pub(super) fn close_worktree_session(
    session: &Session,
    force: bool,
    current_pane_id: Option<&str>,
) -> Result<()> {
    if !should_gracefully_close_worktree_session(session, current_pane_id) {
        return Ok(());
    }
    close_tracked_session(session, force)
}

fn validate_worktree_session(
    session: &Session,
    force: bool,
    current_pane_id: Option<&str>,
    runtime: &LiveCloseRuntime,
) -> Result<()> {
    if !should_gracefully_close_worktree_session(session, current_pane_id) {
        return Ok(());
    }
    let mut session = session.clone();
    super::super::session_status::include_pending_status(&mut session);
    prepare_close_session(&session, force, runtime).map(|_| ())
}

fn should_gracefully_close_worktree_session(
    session: &Session,
    current_pane_id: Option<&str>,
) -> bool {
    session.tmux_info.is_some() && !is_caller_pane_session(session, current_pane_id)
}

fn is_caller_pane_session(session: &Session, current_pane_id: Option<&str>) -> bool {
    session
        .tmux_info
        .as_ref()
        .is_some_and(|info| current_pane_id == Some(info.pane_id.as_str()))
}

pub(super) fn linked_worktree_root(path: &Path) -> Option<PathBuf> {
    let repo = GitRepo::open_at(path).ok()?;
    repo.is_worktree().then(|| repo.workdir().to_path_buf())
}

pub(super) fn path_is_within_worktree(path: &Path, worktree_root: &Path) -> bool {
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let worktree_root = worktree_root
        .canonicalize()
        .unwrap_or_else(|_| worktree_root.to_path_buf());
    path.starts_with(worktree_root)
}

fn select_worktree_sessions<'a>(sessions: &'a [Session], worktree_root: &Path) -> Vec<&'a Session> {
    sessions
        .iter()
        .filter(|session| path_is_within_worktree(&session.cwd, worktree_root))
        .collect()
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::path::PathBuf;

    use rstest::rstest;

    use super::super::test_support::session;
    use super::*;

    fn load_candidate_session(sessions: &[&Session], session_id: &str) -> Result<Option<Session>> {
        Ok(sessions
            .iter()
            .find(|session| session.session_id == session_id)
            .map(|session| (**session).clone()))
    }

    #[rstest]
    #[case::root(vec!["/tmp/worktrees/feature"], vec!["session-0"])]
    #[case::nested(vec!["/tmp/worktrees/feature/src"], vec!["session-0"])]
    #[case::sibling(vec!["/tmp/worktrees/feature-extra"], vec![])]
    #[case::multiple(
        vec!["/tmp/worktrees/feature", "/tmp/worktrees/feature/src"],
        vec!["session-0", "session-1"]
    )]
    fn selects_all_sessions_for_the_target_worktree(
        #[case] session_cwds: Vec<&str>,
        #[case] expected: Vec<&str>,
    ) {
        let sessions = session_cwds
            .into_iter()
            .enumerate()
            .map(|(index, cwd)| {
                let mut session = session();
                session.session_id = format!("session-{index}");
                session.cwd = PathBuf::from(cwd);
                session
            })
            .collect::<Vec<_>>();
        let actual = select_worktree_sessions(&sessions, Path::new("/tmp/worktrees/feature"))
            .into_iter()
            .map(|session| session.session_id.as_str())
            .collect::<Vec<_>>();

        assert_eq!(actual, expected);
    }

    #[rstest]
    #[case::same_pane(Some("%42"), true, false)]
    #[case::other_pane(Some("%99"), true, true)]
    #[case::no_current_pane(None, true, true)]
    #[case::missing_tmux_info(Some("%42"), false, false)]
    fn skips_graceful_close_for_the_current_or_untracked_pane(
        mut session: Session,
        #[case] current_pane_id: Option<&str>,
        #[case] has_tmux_info: bool,
        #[case] expected: bool,
    ) {
        if !has_tmux_info {
            session.tmux_info = None;
        }
        assert_eq!(
            should_gracefully_close_worktree_session(&session, current_pane_id),
            expected
        );
    }

    #[rstest]
    #[case::same_pane(None, Some("%42"), false, Err("The target worktree contains the caller's current session; skipping close".to_string()))]
    #[case::same_session_id_without_pane(Some("session-1"), None, true, Err("The target worktree contains the caller's current session; skipping close".to_string()))]
    #[case::different_session_and_pane(Some("session-2"), Some("%99"), false, Ok(()))]
    #[case::no_caller_context(None, None, false, Ok(()))]
    #[case::untracked_session(None, None, true, Ok(()))]
    fn refuses_to_close_the_callers_current_session(
        mut session: Session,
        #[case] caller_session_id: Option<&str>,
        #[case] current_pane_id: Option<&str>,
        #[case] without_tmux_info: bool,
        #[case] expected: std::result::Result<(), String>,
    ) {
        if without_tmux_info {
            session.tmux_info = None;
        }
        assert_eq!(
            ensure_not_closing_current_session(&[&session], caller_session_id, current_pane_id,)
                .map_err(|error| error.to_string()),
            expected,
        );
    }

    #[tokio::test]
    async fn pr_merge_path_refuses_the_callers_later_session_before_prepare() {
        let first_session = session();
        let mut caller_session = session();
        caller_session.session_id = "session-2".to_string();
        let sessions = [&first_session, &caller_session];
        let args = CloseArgs::default();
        let calls = RefCell::new(Vec::new());
        let context = WorktreeCloseContext {
            args: &args,
            sessions: &sessions,
            worktree_root: Path::new("/tmp/test-repo"),
            child_parent_session_id: None,
            caller_session_id: Some("session-2"),
            caller_pane_id: None,
        };
        let result = close_worktree_with(
            context,
            true,
            async {
                calls.borrow_mut().push("prepare");
                Ok::<(), anyhow::Error>(())
            },
            |_, _, _| {
                calls.borrow_mut().push("validate session");
                Ok(())
            },
            |_| Ok(None),
            |_, _, _| {
                calls.borrow_mut().push("close session");
                Ok(())
            },
            |_| async {
                calls.borrow_mut().push("remove worktree");
                Ok(())
            },
        )
        .await;
        assert_eq!(
            (
                result.map_err(|error| error.to_string()),
                calls.into_inner(),
            ),
            (
                Err(
                    "The target worktree contains the caller's current session; skipping close"
                        .to_string()
                ),
                vec![],
            ),
        );
    }

    #[tokio::test]
    async fn child_close_rejects_any_non_child_before_prepare_even_with_force() {
        let mut target_session = session();
        target_session.ancestor_session_ids = vec!["other-parent".to_string()];
        let mut child_session = session();
        child_session.session_id = "child-session".to_string();
        child_session.ancestor_session_ids = vec!["parent-session".to_string()];
        let sessions = [&child_session, &target_session];
        let args = CloseArgs {
            target: Some("session-1".to_string()),
            child: true,
            force: true,
            skip_hooks: false,
        };
        let calls = RefCell::new(Vec::new());
        let context = WorktreeCloseContext {
            args: &args,
            sessions: &sessions,
            worktree_root: Path::new("/tmp/test-repo"),
            child_parent_session_id: Some("parent-session"),
            caller_session_id: None,
            caller_pane_id: None,
        };

        let result = close_worktree_with(
            context,
            false,
            async {
                calls.borrow_mut().push("prepare".to_string());
                Ok::<(), anyhow::Error>(())
            },
            |_, _, _| {
                calls.borrow_mut().push("validate session".to_string());
                Ok(())
            },
            |_| Ok(None),
            |_, _, _| {
                calls.borrow_mut().push("close session".to_string());
                Ok(())
            },
            |_| async {
                calls.borrow_mut().push("remove worktree".to_string());
                Ok(())
            },
        )
        .await;

        assert_eq!(
            (
                result.map_err(|error| error.to_string()),
                calls.into_inner()
            ),
            (
                Err(
                    "Agent session `session-1` is not a direct child of the calling session"
                        .to_string()
                ),
                vec![],
            ),
        );
    }

    #[tokio::test]
    async fn child_worktree_close_without_a_tracked_session_refuses_before_prepare() {
        let args = CloseArgs {
            target: Some("target-worktree".to_string()),
            child: true,
            force: false,
            skip_hooks: false,
        };
        let calls = RefCell::new(Vec::new());
        let context = WorktreeCloseContext {
            args: &args,
            sessions: &[],
            worktree_root: Path::new("/tmp/test-repo"),
            child_parent_session_id: Some("parent-session"),
            caller_session_id: None,
            caller_pane_id: None,
        };

        let result = close_worktree_with(
            context,
            false,
            async {
                calls.borrow_mut().push("prepare");
                Ok::<(), anyhow::Error>(())
            },
            |_, _, _| {
                calls.borrow_mut().push("validate session");
                Ok(())
            },
            |_| Ok(None),
            |_, _, _| {
                calls.borrow_mut().push("close session");
                Ok(())
            },
            |_| async {
                calls.borrow_mut().push("remove worktree");
                Ok(())
            },
        )
        .await;

        assert_eq!(
            (
                result.map_err(|error| error.to_string()),
                calls.into_inner()
            ),
            (
                Err(
                    "The target worktree has no tracked agent session; cannot verify --child"
                        .to_string()
                ),
                vec![],
            ),
        );
    }

    #[tokio::test]
    async fn child_close_allows_a_direct_child_with_force() {
        let mut target_session = session();
        target_session.ancestor_session_ids = vec!["parent-session".to_string()];
        let sessions = [&target_session];
        let args = CloseArgs {
            target: Some("session-1".to_string()),
            child: true,
            force: true,
            skip_hooks: false,
        };
        let calls = RefCell::new(Vec::new());
        let context = WorktreeCloseContext {
            args: &args,
            sessions: &sessions,
            worktree_root: Path::new("/tmp/test-repo"),
            child_parent_session_id: Some("parent-session"),
            caller_session_id: None,
            caller_pane_id: None,
        };

        let result = close_worktree_with(
            context,
            false,
            async {
                calls.borrow_mut().push("prepare");
                Ok::<(), anyhow::Error>(())
            },
            |_, _, _| {
                calls.borrow_mut().push("validate session");
                Ok(())
            },
            |session_id| {
                calls.borrow_mut().push("load session-1");
                load_candidate_session(&sessions, session_id)
            },
            |_, force, _| {
                calls.borrow_mut().push(if force {
                    "close session with force"
                } else {
                    "close session safely"
                });
                Ok(())
            },
            |_| async {
                calls.borrow_mut().push("remove worktree");
                Ok(())
            },
        )
        .await;

        assert_eq!(
            (result.is_ok(), calls.into_inner()),
            (
                true,
                vec![
                    "validate session",
                    "prepare",
                    "load session-1",
                    "validate session",
                    "close session with force",
                    "remove worktree",
                ],
            ),
        );
    }

    #[tokio::test]
    async fn worktree_close_reloads_activity_after_confirmation() {
        let mut listed_session = session();
        listed_session.updated_at = chrono::Utc::now() - chrono::TimeDelta::hours(1);
        let latest_session = RefCell::new(listed_session.clone());
        let sessions = [&listed_session];
        let args = CloseArgs::default();
        let calls = RefCell::new(Vec::<String>::new());
        let context = WorktreeCloseContext {
            args: &args,
            sessions: &sessions,
            worktree_root: Path::new("/tmp/test-repo"),
            child_parent_session_id: None,
            caller_session_id: None,
            caller_pane_id: None,
        };

        let result = close_worktree_with(
            context,
            false,
            async {
                calls.borrow_mut().push("prepare".to_string());
                latest_session.borrow_mut().updated_at = chrono::Utc::now();
                Ok::<(), anyhow::Error>(())
            },
            |session, _, _| {
                calls
                    .borrow_mut()
                    .push(format!("validate {}", session.session_id));
                if session.updated_at > listed_session.updated_at {
                    anyhow::bail!("recent activity prevents closing the session");
                }
                Ok(())
            },
            |session_id| {
                calls.borrow_mut().push(format!("load {session_id}"));
                Ok(Some(latest_session.borrow().clone()))
            },
            |session, _, _| {
                calls
                    .borrow_mut()
                    .push(format!("close {}", session.session_id));
                Ok(())
            },
            |_| async {
                calls.borrow_mut().push("remove worktree".to_string());
                Ok(())
            },
        )
        .await;

        assert_eq!(
            (
                result.map_err(|error| error.to_string()),
                calls.into_inner(),
            ),
            (
                Err("recent activity prevents closing the session".to_string()),
                vec![
                    "validate session-1".to_string(),
                    "prepare".to_string(),
                    "load session-1".to_string(),
                    "validate session-1".to_string(),
                ],
            ),
        );
    }

    #[tokio::test]
    async fn worktree_close_skips_a_session_that_moved_outside_the_worktree() {
        let listed_session = session();
        let mut latest_session = listed_session.clone();
        latest_session.cwd = PathBuf::from("/tmp/other-repo");
        let sessions = [&listed_session];
        let args = CloseArgs::default();
        let calls = RefCell::new(Vec::<String>::new());
        let context = WorktreeCloseContext {
            args: &args,
            sessions: &sessions,
            worktree_root: Path::new("/tmp/test-repo"),
            child_parent_session_id: None,
            caller_session_id: None,
            caller_pane_id: None,
        };

        let result = close_worktree_with(
            context,
            false,
            async {
                calls.borrow_mut().push("prepare".to_string());
                Ok::<(), anyhow::Error>(())
            },
            |session, _, _| {
                calls
                    .borrow_mut()
                    .push(format!("validate {}", session.session_id));
                Ok(())
            },
            |session_id| {
                calls.borrow_mut().push(format!("load {session_id}"));
                Ok(Some(latest_session.clone()))
            },
            |session, _, _| {
                calls
                    .borrow_mut()
                    .push(format!("close {}", session.session_id));
                Ok(())
            },
            |_| async {
                calls.borrow_mut().push("remove worktree".to_string());
                Ok(())
            },
        )
        .await;

        assert_eq!(
            (result.is_ok(), calls.into_inner()),
            (
                true,
                vec![
                    "validate session-1".to_string(),
                    "prepare".to_string(),
                    "load session-1".to_string(),
                    "remove worktree".to_string(),
                ],
            ),
        );
    }

    #[tokio::test]
    async fn worktree_close_aborts_when_a_session_disappears_after_confirmation() {
        let target_session = session();
        let sessions = [&target_session];
        let args = CloseArgs::default();
        let calls = RefCell::new(Vec::<String>::new());
        let context = WorktreeCloseContext {
            args: &args,
            sessions: &sessions,
            worktree_root: Path::new("/tmp/test-repo"),
            child_parent_session_id: None,
            caller_session_id: None,
            caller_pane_id: None,
        };

        let result = close_worktree_with(
            context,
            false,
            async {
                calls.borrow_mut().push("prepare".to_string());
                Ok::<(), anyhow::Error>(())
            },
            |_, _, _| {
                calls.borrow_mut().push("validate session".to_string());
                Ok(())
            },
            |session_id| {
                calls.borrow_mut().push(format!("load {session_id}"));
                Ok(None)
            },
            |_, _, _| {
                calls.borrow_mut().push("close session".to_string());
                Ok(())
            },
            |_| async {
                calls.borrow_mut().push("remove worktree".to_string());
                Ok(())
            },
        )
        .await;

        assert_eq!(
            (
                result.map_err(|error| error.to_string()),
                calls.into_inner(),
            ),
            (
                Err(
                    "Could not reload agent session `session-1` after worktree confirmation"
                        .to_string(),
                ),
                vec![
                    "validate session".to_string(),
                    "prepare".to_string(),
                    "load session-1".to_string(),
                ],
            ),
        );
    }

    #[tokio::test]
    async fn worktree_close_closes_all_matching_sessions_before_removing_worktree() {
        let mut first_session = session();
        first_session.cwd = PathBuf::from("/tmp/worktrees/feature");
        let mut second_session = session();
        second_session.session_id = "session-2".to_string();
        second_session.cwd = PathBuf::from("/tmp/worktrees/feature/src");
        let sessions = [&first_session, &second_session];
        let args = CloseArgs::default();
        let calls = RefCell::new(Vec::<String>::new());
        let context = WorktreeCloseContext {
            args: &args,
            sessions: &sessions,
            worktree_root: Path::new("/tmp/worktrees/feature"),
            child_parent_session_id: None,
            caller_session_id: None,
            caller_pane_id: None,
        };

        let result = close_worktree_with(
            context,
            false,
            async {
                calls.borrow_mut().push("prepare".to_string());
                Ok::<(), anyhow::Error>(())
            },
            |session, _, _| {
                calls
                    .borrow_mut()
                    .push(format!("validate {}", session.session_id));
                Ok(())
            },
            |session_id| {
                calls.borrow_mut().push(format!("load {}", session_id));
                load_candidate_session(&sessions, session_id)
            },
            |session, _, _| {
                calls.borrow_mut().push(session.session_id.clone());
                Ok(())
            },
            |_| async {
                calls.borrow_mut().push("remove worktree".to_string());
                Ok(())
            },
        )
        .await;

        assert_eq!(
            (
                result.map_err(|error| error.to_string()),
                calls.into_inner()
            ),
            (
                Ok(()),
                vec![
                    "validate session-1".to_string(),
                    "validate session-2".to_string(),
                    "prepare".to_string(),
                    "load session-1".to_string(),
                    "load session-2".to_string(),
                    "validate session-1".to_string(),
                    "validate session-2".to_string(),
                    "session-1".to_string(),
                    "session-2".to_string(),
                    "remove worktree".to_string(),
                ],
            ),
        );
    }

    #[tokio::test]
    async fn worktree_close_preflights_every_session_before_closing_any() {
        let first_session = session();
        let mut second_session = session();
        second_session.session_id = "session-2".to_string();
        let sessions = [&first_session, &second_session];
        let args = CloseArgs::default();
        let calls = RefCell::new(Vec::<String>::new());
        let context = WorktreeCloseContext {
            args: &args,
            sessions: &sessions,
            worktree_root: Path::new("/tmp/test-repo"),
            child_parent_session_id: None,
            caller_session_id: None,
            caller_pane_id: None,
        };
        let result = close_worktree_with(
            context,
            false,
            async {
                calls.borrow_mut().push("prepare".to_string());
                Ok::<(), anyhow::Error>(())
            },
            |session, _, _| {
                calls
                    .borrow_mut()
                    .push(format!("validate {}", session.session_id));
                if session.session_id == "session-2" {
                    anyhow::bail!("running session refused to close");
                }
                Ok(())
            },
            |_| Ok(None),
            |session, _, _| {
                calls
                    .borrow_mut()
                    .push(format!("close {}", session.session_id));
                Ok(())
            },
            |_| async {
                calls.borrow_mut().push("remove worktree".to_string());
                Ok(())
            },
        )
        .await;

        assert_eq!(
            (
                result.map_err(|error| error.to_string()),
                calls.into_inner(),
            ),
            (
                Err("running session refused to close".to_string()),
                vec![
                    "validate session-1".to_string(),
                    "validate session-2".to_string(),
                ],
            ),
        );
    }

    #[tokio::test]
    async fn worktree_close_reports_sessions_closed_before_a_shutdown_failure() {
        let first_session = session();
        let mut second_session = session();
        second_session.session_id = "session-2".to_string();
        let sessions = [&first_session, &second_session];
        let args = CloseArgs::default();
        let calls = RefCell::new(Vec::<String>::new());
        let context = WorktreeCloseContext {
            args: &args,
            sessions: &sessions,
            worktree_root: Path::new("/tmp/test-repo"),
            child_parent_session_id: None,
            caller_session_id: None,
            caller_pane_id: None,
        };

        let result = close_worktree_with(
            context,
            false,
            async {
                calls.borrow_mut().push("prepare".to_string());
                Ok::<(), anyhow::Error>(())
            },
            |session, _, _| {
                calls
                    .borrow_mut()
                    .push(format!("validate {}", session.session_id));
                Ok(())
            },
            |session_id| {
                calls.borrow_mut().push(format!("load {}", session_id));
                load_candidate_session(&sessions, session_id)
            },
            |session, _, _| {
                calls
                    .borrow_mut()
                    .push(format!("close {}", session.session_id));
                if session.session_id == "session-2" {
                    anyhow::bail!("shutdown failed");
                }
                Ok(())
            },
            |_| async {
                calls.borrow_mut().push("remove worktree".to_string());
                Ok(())
            },
        )
        .await;

        assert_eq!(
            (
                result.map_err(|error| error.to_string()),
                calls.into_inner()
            ),
            (
                Err(
                    "Failed to close agent session `session-2`; already closed sessions: session-1"
                        .to_string()
                ),
                vec![
                    "validate session-1".to_string(),
                    "validate session-2".to_string(),
                    "prepare".to_string(),
                    "load session-1".to_string(),
                    "load session-2".to_string(),
                    "validate session-1".to_string(),
                    "validate session-2".to_string(),
                    "close session-1".to_string(),
                    "close session-2".to_string(),
                ],
            ),
        );
    }

    #[tokio::test]
    async fn cancelled_worktree_confirmation_leaves_the_session_open() {
        let calls = RefCell::new(Vec::new());

        let result = close_with_worktree_plan(
            async {
                calls.borrow_mut().push("prepare");
                Err::<(), _>(anyhow::anyhow!("Cancelled."))
            },
            || {
                calls.borrow_mut().push("close session");
                Ok(())
            },
            |_| async {
                calls.borrow_mut().push("remove worktree");
                Ok(())
            },
        )
        .await;

        assert_eq!(
            (result.unwrap_err().to_string(), calls.into_inner()),
            ("Cancelled.".to_string(), vec!["prepare"]),
        );
    }

    #[tokio::test]
    async fn worktree_close_removes_after_closing_the_selected_session() {
        let calls = RefCell::new(Vec::new());

        let result = close_with_worktree_plan(
            async {
                calls.borrow_mut().push("prepare");
                Ok(())
            },
            || {
                calls.borrow_mut().push("close session");
                Ok(())
            },
            |_| async {
                calls.borrow_mut().push("remove worktree");
                Ok(())
            },
        )
        .await;

        assert_eq!(
            (result.is_ok(), calls.into_inner()),
            (true, vec!["prepare", "close session", "remove worktree"]),
        );
    }
}
