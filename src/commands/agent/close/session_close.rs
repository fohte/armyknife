use std::future::Future;
use std::io::{self, Write};
use std::path::Path;

use anyhow::Result;

use super::{CloseArgs, Session, TmuxInfo};
use crate::infra::tmux;

pub(super) async fn close_session_value(
    session: &Session,
    args: &CloseArgs,
    child_parent_session_id: Option<&str>,
) -> Result<()> {
    let Some(worktree_root) = super::worktree::linked_worktree_root(&session.cwd) else {
        return close_session_value_without_worktree(
            session,
            args,
            child_parent_session_id,
            super::close_tracked_session,
        );
    };

    close_session_value_with_worktree(
        SessionWorktreeContext {
            session,
            force: args.force,
            parent_session_id: child_parent_session_id,
            worktree_root: &worktree_root,
        },
        find_shared_worktree_session_ids,
        crate::shared::worktree_delete::prepare(&worktree_root, args.force),
        super::close_tracked_session,
        || {
            let current_pane_id = crate::shared::caller_pane::resolve_caller_pane_id();
            super::worktree::close_worktree_session(session, args.force, current_pane_id.as_deref())
        },
        |plan| crate::shared::worktree_delete::execute(plan, args.skip_hooks),
        &mut io::stderr(),
    )
    .await
}

struct SessionWorktreeContext<'a> {
    session: &'a Session,
    force: bool,
    parent_session_id: Option<&'a str>,
    worktree_root: &'a Path,
}

async fn close_session_value_with_worktree<P, F>(
    context: SessionWorktreeContext<'_>,
    find_shared_sessions: impl FnOnce(&SessionWorktreeContext<'_>) -> Result<Vec<String>>,
    prepare_worktree: impl Future<Output = Result<P>>,
    close_shared_session: impl FnOnce(&Session, bool) -> Result<()>,
    close_worktree_session: impl FnOnce() -> Result<()>,
    remove_worktree: impl FnOnce(P) -> F,
    warning_writer: &mut impl Write,
) -> Result<()>
where
    F: Future<Output = Result<()>>,
{
    super::child::ensure_target_session(Some(context.session), context.parent_session_id)?;

    let shared_session_ids = find_shared_sessions(&context)?;
    if !shared_session_ids.is_empty() {
        close_shared_session(context.session, context.force)?;
        if let Err(error) = write_worktree_kept_warning(warning_writer, &shared_session_ids) {
            tracing::warn!(
                event = "agent.close.worktree_kept_warning_failed",
                error = %error,
            );
        }
        return Ok(());
    }

    super::worktree::close_with_worktree_plan(
        prepare_worktree,
        close_worktree_session,
        remove_worktree,
    )
    .await
}

fn find_shared_worktree_session_ids(context: &SessionWorktreeContext<'_>) -> Result<Vec<String>> {
    let sessions = super::super::store::list_sessions()?;
    let mut pane_liveness = tmux::PaneLiveness::default();
    collect_shared_worktree_session_ids(
        context,
        &sessions,
        super::super::store::load_session,
        |info| {
            pane_liveness.cache_socket(
                info.socket_path.as_deref(),
                tmux::list_all_pane_ids_for_socket,
            );
            pane_liveness.is_alive(info.socket_path.as_deref(), &info.pane_id)
        },
    )
}

fn collect_shared_worktree_session_ids(
    context: &SessionWorktreeContext<'_>,
    sessions: &[Session],
    load_session: impl FnOnce(&str) -> Result<Option<Session>>,
    mut is_pane_alive: impl FnMut(&TmuxInfo) -> Option<bool>,
) -> Result<Vec<String>> {
    let mut session_ids = sessions
        .iter()
        .filter(|session| {
            session.session_id != context.session.session_id
                && super::worktree::path_is_within_worktree(&session.cwd, context.worktree_root)
                && !session
                    .tmux_info
                    .as_ref()
                    .is_some_and(|info| is_pane_alive(info) == Some(false))
        })
        .map(|session| session.session_id.clone())
        .collect::<Vec<_>>();

    // A missing caller record cannot establish that its cwd is outside this worktree.
    if let Some(parent_session_id) = context.parent_session_id
        && !sessions
            .iter()
            .any(|session| session.session_id == parent_session_id)
        && load_session(parent_session_id)?.is_none()
    {
        session_ids.push(parent_session_id.to_string());
    }

    session_ids.sort();
    session_ids.dedup();
    Ok(session_ids)
}

fn write_worktree_kept_warning(writer: &mut impl Write, session_ids: &[String]) -> io::Result<()> {
    writeln!(
        writer,
        "Keeping worktree because these agent sessions still use it: {}",
        session_ids.join(", ")
    )
}

fn close_session_value_without_worktree(
    session: &Session,
    args: &CloseArgs,
    child_parent_session_id: Option<&str>,
    close_session: impl FnOnce(&Session, bool) -> Result<()>,
) -> Result<()> {
    super::child::ensure_target_session(Some(session), child_parent_session_id)?;
    close_session(session, args.force)
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::path::{Path, PathBuf};

    use rstest::{fixture, rstest};

    use super::super::test_support::session;
    use super::*;

    fn linked_session(session_id: &str, cwd: &str, pane_id: Option<&str>) -> Session {
        let mut session = session();
        session.session_id = session_id.to_string();
        session.cwd = PathBuf::from(cwd);
        if let Some(pane_id) = pane_id {
            if let Some(info) = &mut session.tmux_info {
                info.pane_id = pane_id.to_string();
            }
        } else {
            session.tmux_info = None;
        }
        session
    }

    fn context<'a>(
        session: &'a Session,
        force: bool,
        parent_session_id: Option<&'a str>,
    ) -> SessionWorktreeContext<'a> {
        SessionWorktreeContext {
            session,
            force,
            parent_session_id,
            worktree_root: Path::new("/tmp/worktrees/feature"),
        }
    }

    #[fixture]
    fn shared_worktree_sessions() -> (Session, Vec<Session>) {
        let mut target_session =
            linked_session("target", "/tmp/worktrees/feature", Some("%target"));
        target_session.ancestor_session_ids = vec!["parent-session".to_string()];
        let sessions = vec![
            target_session.clone(),
            linked_session(
                "parent-session",
                "/tmp/worktrees/feature/src",
                Some("%parent"),
            ),
            linked_session("live-session", "/tmp/worktrees/feature", Some("%live")),
            linked_session(
                "unknown-session",
                "/tmp/worktrees/feature/src",
                Some("%unknown"),
            ),
            linked_session("untracked-session", "/tmp/worktrees/feature", None),
            linked_session("dead-session", "/tmp/worktrees/feature", Some("%dead")),
            linked_session("outside-session", "/tmp/other-repo", Some("%outside")),
        ];
        (target_session, sessions)
    }

    fn pane_liveness(info: &TmuxInfo) -> Option<bool> {
        match info.pane_id.as_str() {
            "%live" | "%parent" => Some(true),
            "%dead" => Some(false),
            _ => None,
        }
    }

    #[rstest]
    #[case::direct_child(
        vec!["parent-session"],
        Ok(()),
        vec!["close session safely"],
    )]
    #[case::non_child(
        vec!["other-session"],
        Err("Agent session `session-1` is not a direct child of the calling session"),
        vec![],
    )]
    fn validates_before_closing_a_tracked_session_without_a_linked_worktree(
        mut session: Session,
        #[case] ancestors: Vec<&str>,
        #[case] expected_result: std::result::Result<(), &str>,
        #[case] expected_calls: Vec<&str>,
    ) {
        session.ancestor_session_ids = ancestors.into_iter().map(str::to_string).collect();
        let args = CloseArgs {
            target: Some(session.session_id.clone()),
            child: true,
            ..CloseArgs::default()
        };
        let calls = RefCell::new(Vec::new());

        let result = close_session_value_without_worktree(
            &session,
            &args,
            Some("parent-session"),
            |_, force| {
                calls.borrow_mut().push(if force {
                    "close session with force"
                } else {
                    "close session safely"
                });
                Ok(())
            },
        );

        assert_eq!(
            (
                result.map_err(|error| error.to_string()),
                calls.into_inner(),
            ),
            (expected_result.map_err(str::to_string), expected_calls,),
        );
    }

    #[rstest]
    #[tokio::test]
    async fn forced_close_keeps_worktree_for_active_sessions_and_reports_their_ids(
        shared_worktree_sessions: (Session, Vec<Session>),
    ) {
        let (target_session, sessions) = shared_worktree_sessions;
        let calls = RefCell::new(Vec::new());
        let mut warning = Vec::new();

        let result = close_session_value_with_worktree(
            context(&target_session, true, Some("parent-session")),
            |context| {
                calls.borrow_mut().push("find shared sessions");
                collect_shared_worktree_session_ids(
                    context,
                    &sessions,
                    |_| {
                        Err(anyhow::anyhow!(
                            "the registered caller should not be loaded"
                        ))
                    },
                    pane_liveness,
                )
            },
            async {
                calls.borrow_mut().push("prepare worktree");
                Ok::<(), anyhow::Error>(())
            },
            |session, force| {
                calls
                    .borrow_mut()
                    .push(if force && session.session_id == "target" {
                        "close target with force"
                    } else {
                        "close target safely"
                    });
                Ok(())
            },
            || {
                calls.borrow_mut().push("close worktree session");
                Ok(())
            },
            |_| async {
                calls.borrow_mut().push("remove worktree");
                Ok(())
            },
            &mut warning,
        )
        .await;

        assert_eq!(
            (
                result.map_err(|error| error.to_string()),
                calls.into_inner(),
                String::from_utf8(warning).unwrap(),
            ),
            (
                Ok(()),
                vec!["find shared sessions", "close target with force"],
                "Keeping worktree because these agent sessions still use it: live-session, parent-session, unknown-session, untracked-session\n".to_string(),
            ),
        );
    }

    #[rstest]
    fn collects_only_other_non_stale_sessions_inside_the_worktree(
        shared_worktree_sessions: (Session, Vec<Session>),
    ) {
        let (target_session, sessions) = shared_worktree_sessions;
        let actual = collect_shared_worktree_session_ids(
            &context(&target_session, false, Some("parent-session")),
            &sessions,
            |_| {
                Err(anyhow::anyhow!(
                    "the registered caller should not be loaded"
                ))
            },
            pane_liveness,
        )
        .map_err(|error| error.to_string());

        assert_eq!(
            actual,
            Ok(vec![
                "live-session".to_string(),
                "parent-session".to_string(),
                "unknown-session".to_string(),
                "untracked-session".to_string(),
            ]),
        );
    }

    #[test]
    fn treats_an_unresolved_child_caller_as_using_the_worktree() {
        let mut target_session = linked_session("child", "/tmp/worktrees/feature", Some("%child"));
        target_session.ancestor_session_ids = vec!["parent-session".to_string()];

        let actual = collect_shared_worktree_session_ids(
            &context(&target_session, false, Some("parent-session")),
            &[target_session.clone()],
            |_| Ok(None),
            |_| None,
        )
        .map_err(|error| error.to_string());

        assert_eq!(actual, Ok(vec!["parent-session".to_string()]));
    }

    #[tokio::test]
    async fn removes_worktree_when_only_stale_or_unrelated_sessions_remain() {
        let target_session = linked_session("target", "/tmp/worktrees/feature", Some("%target"));
        let sessions = vec![
            target_session.clone(),
            linked_session("dead-session", "/tmp/worktrees/feature", Some("%dead")),
            linked_session("outside-session", "/tmp/other-repo", Some("%outside")),
        ];
        let calls = RefCell::new(Vec::new());
        let mut warning = Vec::new();

        let result = close_session_value_with_worktree(
            context(&target_session, false, None),
            |context| {
                calls.borrow_mut().push("find shared sessions");
                collect_shared_worktree_session_ids(
                    context,
                    &sessions,
                    |_| Ok(None),
                    |info| Some(info.pane_id != "%dead"),
                )
            },
            async {
                calls.borrow_mut().push("prepare worktree");
                Ok::<(), anyhow::Error>(())
            },
            |_, _| {
                calls.borrow_mut().push("close shared session");
                Ok(())
            },
            || {
                calls.borrow_mut().push("close worktree session");
                Ok(())
            },
            |_| async {
                calls.borrow_mut().push("remove worktree");
                Ok(())
            },
            &mut warning,
        )
        .await;

        assert_eq!(
            (
                result.map_err(|error| error.to_string()),
                calls.into_inner(),
                String::from_utf8(warning).unwrap(),
            ),
            (
                Ok(()),
                vec![
                    "find shared sessions",
                    "prepare worktree",
                    "close worktree session",
                    "remove worktree",
                ],
                String::new(),
            ),
        );
    }

    #[tokio::test]
    async fn session_id_child_close_rejects_non_children_before_worktree_prepare_even_with_force() {
        let mut target_session = session();
        target_session.ancestor_session_ids = vec!["other-parent".to_string()];
        let context = SessionWorktreeContext {
            session: &target_session,
            force: true,
            parent_session_id: Some("parent-session"),
            worktree_root: Path::new("/tmp/worktrees/feature"),
        };
        let calls = RefCell::new(Vec::new());
        let mut warning = Vec::new();

        let result = close_session_value_with_worktree(
            context,
            |_| {
                calls.borrow_mut().push("find shared sessions");
                Ok(vec![])
            },
            async {
                calls.borrow_mut().push("prepare worktree");
                Ok::<(), anyhow::Error>(())
            },
            |_, _| {
                calls.borrow_mut().push("close shared session");
                Ok(())
            },
            || {
                calls.borrow_mut().push("close worktree session");
                Ok(())
            },
            |_| async {
                calls.borrow_mut().push("remove worktree");
                Ok(())
            },
            &mut warning,
        )
        .await;

        assert_eq!(
            (
                result.map_err(|error| error.to_string()),
                calls.into_inner(),
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
}
