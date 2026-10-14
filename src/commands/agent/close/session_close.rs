use std::future::Future;
use std::path::Path;

use anyhow::Result;

use super::{CloseArgs, Session};

pub(super) async fn close_session_value(
    session: &Session,
    args: &CloseArgs,
    child_parent_session_id: Option<&str>,
) -> Result<()> {
    let Some(worktree_root) = super::linked_worktree_root(&session.cwd) else {
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
        crate::shared::worktree_delete::prepare(&worktree_root, args.force),
        super::super::store::load_session,
        super::close_tracked_session,
        || {
            let current_pane_id = crate::shared::caller_pane::resolve_caller_pane_id();
            super::close_worktree_session(session, args.force, current_pane_id.as_deref())
        },
        |plan| crate::shared::worktree_delete::execute(plan, args.skip_hooks),
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
    prepare_worktree: impl Future<Output = Result<P>>,
    load_session: impl FnOnce(&str) -> Result<Option<Session>>,
    close_shared_session: impl FnOnce(&Session, bool) -> Result<()>,
    close_worktree_session: impl FnOnce() -> Result<()>,
    remove_worktree: impl FnOnce(P) -> F,
) -> Result<()>
where
    F: Future<Output = Result<()>>,
{
    super::child::ensure_target_session(Some(context.session), context.parent_session_id)?;

    if close_child_session_if_sharing_worktree(
        context.session,
        context.force,
        context.parent_session_id,
        context.worktree_root,
        load_session,
        close_shared_session,
    )? {
        return Ok(());
    }

    super::close_with_worktree_plan(prepare_worktree, close_worktree_session, remove_worktree).await
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

fn close_child_session_if_sharing_worktree(
    session: &Session,
    force: bool,
    child_parent_session_id: Option<&str>,
    worktree_root: &Path,
    load_session: impl FnOnce(&str) -> Result<Option<Session>>,
    close_session: impl FnOnce(&Session, bool) -> Result<()>,
) -> Result<bool> {
    let Some(parent_session_id) = child_parent_session_id else {
        return Ok(false);
    };
    if !caller_session_is_in_worktree(parent_session_id, worktree_root, load_session)? {
        return Ok(false);
    }
    close_session(session, force)?;
    Ok(true)
}

fn caller_session_is_in_worktree(
    caller_session_id: &str,
    worktree_root: &Path,
    load_session: impl FnOnce(&str) -> Result<Option<Session>>,
) -> Result<bool> {
    Ok(match load_session(caller_session_id)? {
        Some(session) => super::path_is_within_worktree(&session.cwd, worktree_root),
        None => true,
    })
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::path::{Path, PathBuf};

    use rstest::rstest;

    use super::super::test_support::session;
    use super::*;

    #[rstest]
    #[case::caller_in_worktree(Some("/tmp/worktrees/feature/src"), Ok(true))]
    #[case::caller_outside_worktree(Some("/tmp/other-repo"), Ok(false))]
    #[case::caller_record_missing(None, Ok(true))]
    fn detects_when_child_worktree_is_shared_with_the_caller(
        mut session: Session,
        #[case] caller_cwd: Option<&str>,
        #[case] expected: std::result::Result<bool, String>,
    ) {
        if let Some(caller_cwd) = caller_cwd {
            session.cwd = PathBuf::from(caller_cwd);
        }
        let caller_session = caller_cwd.map(|_| session.clone());

        let actual = caller_session_is_in_worktree(
            "parent-session",
            Path::new("/tmp/worktrees/feature"),
            |_| Ok(caller_session),
        )
        .map_err(|error| error.to_string());

        assert_eq!(actual, expected);
    }

    #[rstest]
    #[case::shared_worktree(true, vec!["close child session"])]
    #[case::separate_worktree(false, Vec::new())]
    fn closes_shared_worktree_child_without_removing_its_worktree(
        mut session: Session,
        #[case] caller_shares_worktree: bool,
        #[case] expected_calls: Vec<&str>,
    ) {
        session.ancestor_session_ids = vec!["parent-session".to_string()];
        let mut parent = session.clone();
        parent.cwd = PathBuf::from(if caller_shares_worktree {
            "/tmp/worktrees/feature/src"
        } else {
            "/tmp/other-repo"
        });
        let args = CloseArgs {
            target: Some(session.session_id.clone()),
            child: true,
            ..CloseArgs::default()
        };
        let calls = RefCell::new(Vec::new());

        let result = close_child_session_if_sharing_worktree(
            &session,
            args.force,
            Some("parent-session"),
            Path::new("/tmp/worktrees/feature"),
            |_| Ok(Some(parent)),
            |_, _| {
                calls.borrow_mut().push("close child session");
                Ok(())
            },
        );

        assert_eq!(
            (
                result.map_err(|error| error.to_string()),
                calls.into_inner()
            ),
            (Ok(caller_shares_worktree), expected_calls),
        );
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

    #[tokio::test]
    async fn session_id_child_close_rejects_non_children_before_worktree_prepare_even_with_force() {
        let mut target_session = session();
        target_session.ancestor_session_ids = vec!["other-parent".to_string()];
        let caller_session = session();
        let context = SessionWorktreeContext {
            session: &target_session,
            force: true,
            parent_session_id: Some("parent-session"),
            worktree_root: Path::new("/tmp/worktrees/feature"),
        };
        let calls = RefCell::new(Vec::new());

        let result = close_session_value_with_worktree(
            context,
            async {
                calls.borrow_mut().push("prepare worktree");
                Ok::<(), anyhow::Error>(())
            },
            |_| {
                calls.borrow_mut().push("load caller session");
                Ok(Some(caller_session))
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
