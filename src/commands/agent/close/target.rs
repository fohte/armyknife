use std::path::PathBuf;

use anyhow::{Result, bail};

use super::worktree::{close_worktree, linked_worktree_root};
use super::{CloseArgs, Session};

pub(super) async fn close_default_target(target: DefaultTarget, args: &CloseArgs) -> Result<()> {
    match target {
        DefaultTarget::Session(session) => {
            super::session_close::close_session_value(&session, args, None).await
        }
        DefaultTarget::Worktree(worktree_root) => {
            ensure_keep_worktree_session_target(args, None)?;
            close_worktree(&worktree_root, args, false, None).await
        }
    }
}

#[derive(Debug)]
pub(super) enum DefaultTarget {
    Session(Box<Session>),
    Worktree(PathBuf),
}

pub(super) fn resolve_default_target(
    pane_session_id: Result<String>,
    load_session: impl FnOnce(&str) -> Result<Option<Session>>,
    current_worktree_root: impl FnOnce() -> Result<Option<PathBuf>>,
) -> Result<DefaultTarget> {
    match pane_session_id {
        Ok(session_id) => match load_session(&session_id)? {
            Some(session) => Ok(DefaultTarget::Session(Box::new(session))),
            None => current_worktree_root()?
                .map(DefaultTarget::Worktree)
                .ok_or_else(|| anyhow::anyhow!("Agent session `{session_id}` was not found")),
        },
        Err(error) => current_worktree_root()?
            .map(DefaultTarget::Worktree)
            .ok_or(error),
    }
}

pub(super) async fn close_target(
    target: &str,
    args: &CloseArgs,
    child_parent_session_id: Option<&str>,
) -> Result<()> {
    if !target.contains('/')
        && !target.contains('\\')
        && !target.contains("..")
        && let Some(session) = super::super::store::load_session(target)?
    {
        return super::session_close::close_session_value(&session, args, child_parent_session_id)
            .await;
    }

    ensure_keep_worktree_session_target(args, Some(target))?;

    let config = crate::shared::config::load_config()?;
    let worktree_path = crate::shared::worktree_delete::resolve_worktree_path(
        target,
        &config.agent.worktree.dir,
        &config.agent.worktree.branch_prefix,
    )?;
    let worktree_path = PathBuf::from(worktree_path);
    let worktree_root = linked_worktree_root(&worktree_path).unwrap_or(worktree_path);
    close_worktree(&worktree_root, args, false, child_parent_session_id).await
}

fn ensure_keep_worktree_session_target(args: &CloseArgs, target: Option<&str>) -> Result<()> {
    if args.keep_worktree {
        if let Some(target) = target {
            bail!(
                "Target `{target}` is not a tracked agent session; --keep-worktree requires a session target"
            );
        }
        bail!("--keep-worktree requires a session target");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use rstest::rstest;

    use super::super::CloseArgs;
    use super::super::test_support::session;
    use super::{DefaultTarget, close_default_target, close_target, resolve_default_target};

    #[rstest]
    #[case::stored_pane_session_wins(
        Some("session-1"),
        true,
        Some("/tmp/worktrees/feature"),
        Ok(ResolvedTarget::SessionId("session-1".to_string()))
    )]
    #[case::stale_pane_session_falls_back_to_worktree(
        Some("session-1"),
        false,
        Some("/tmp/worktrees/feature"),
        Ok(ResolvedTarget::Worktree(PathBuf::from("/tmp/worktrees/feature")))
    )]
    #[case::stale_pane_session_preserves_not_found_without_worktree(
        Some("session-1"),
        false,
        None,
        Err("Agent session `session-1` was not found".to_string())
    )]
    #[case::unavailable_pane_session_falls_back_to_worktree(
        None,
        false,
        Some("/tmp/worktrees/feature"),
        Ok(ResolvedTarget::Worktree(PathBuf::from("/tmp/worktrees/feature")))
    )]
    #[case::preserves_pane_error_without_worktree(
        None,
        false,
        None,
        Err("no agent session for pane".to_string())
    )]
    fn resolves_default_target_from_pane_or_current_worktree(
        #[case] pane_session_id: Option<&str>,
        #[case] session_is_stored: bool,
        #[case] worktree_root: Option<&str>,
        #[case] expected: std::result::Result<ResolvedTarget, String>,
    ) {
        let pane_session_id = pane_session_id
            .map(|session_id| Ok(session_id.to_string()))
            .unwrap_or_else(|| Err(anyhow::anyhow!("no agent session for pane")));
        let worktree_root = worktree_root.map(PathBuf::from);
        let actual = resolve_default_target(
            pane_session_id,
            |_| Ok(session_is_stored.then(session)),
            || Ok(worktree_root),
        )
        .map(|target| match target {
            DefaultTarget::Session(session) => ResolvedTarget::SessionId(session.session_id),
            DefaultTarget::Worktree(path) => ResolvedTarget::Worktree(path),
        })
        .map_err(|error| error.to_string());

        assert_eq!(actual, expected);
    }

    #[tokio::test]
    async fn keep_worktree_rejects_a_worktree_target() {
        let args = CloseArgs {
            target: Some("feature/worktree".to_string()),
            keep_worktree: true,
            ..CloseArgs::default()
        };

        let result = close_target("feature/worktree", &args, None).await;

        assert_eq!(
            result.unwrap_err().to_string(),
            "Target `feature/worktree` is not a tracked agent session; --keep-worktree requires a session target"
        );
    }

    #[tokio::test]
    async fn keep_worktree_rejects_the_default_worktree_fallback() {
        let args = CloseArgs {
            keep_worktree: true,
            ..CloseArgs::default()
        };

        let result = close_default_target(
            DefaultTarget::Worktree(PathBuf::from("/tmp/worktrees/current")),
            &args,
        )
        .await;

        assert_eq!(
            result.unwrap_err().to_string(),
            "--keep-worktree requires a session target"
        );
    }

    #[derive(Debug, PartialEq, Eq)]
    enum ResolvedTarget {
        SessionId(String),
        Worktree(PathBuf),
    }
}
