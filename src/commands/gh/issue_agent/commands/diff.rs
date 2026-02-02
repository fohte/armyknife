//! Diff command for gh-issue-agent.
//!
//! Shows the diff between local changes and remote GitHub issue state.

use clap::Args;

use super::common::IssueContext;
use super::push::changeset::{ChangeSet, DetectOptions, LocalState, RemoteState};
use crate::infra::github::OctocrabClient;

#[derive(Args, Clone, PartialEq, Eq, Debug)]
pub struct DiffArgs {
    #[command(flatten)]
    pub issue: super::IssueArgs,
}

pub async fn run(args: &DiffArgs) -> anyhow::Result<()> {
    let (ctx, client) = IssueContext::from_args(&args.issue).await?;
    run_with_context(&ctx, client).await
}

/// Internal implementation that accepts a client and storage for testability.
#[cfg(test)]
pub(super) async fn run_with_client_and_storage(
    args: &DiffArgs,
    client: &OctocrabClient,
    storage: &crate::commands::gh::issue_agent::storage::IssueStorage,
    current_user: &str,
) -> anyhow::Result<()> {
    use super::common::{get_repo_from_arg_or_git, parse_repo};

    let repo = get_repo_from_arg_or_git(&args.issue.repo)?;
    let (owner, repo_name) = parse_repo(&repo)?;

    let ctx = IssueContext {
        owner: owner.to_string(),
        repo_name: repo_name.to_string(),
        issue_number: args.issue.issue_number,
        storage: storage.clone(),
        current_user: current_user.to_string(),
    };
    run_with_context(&ctx, client).await
}

async fn run_with_context(ctx: &IssueContext, client: &OctocrabClient) -> anyhow::Result<()> {
    let remote = ctx.fetch_remote(client).await?;
    let local = ctx.load_local()?;

    // Warn if remote has changed since pull (but don't error, just show diff)
    // For diff command, we use simple timestamp comparison since we just want
    // to warn the user, not block the operation.
    let remote_updated_at = remote.issue.updated_at.to_rfc3339();
    if local.metadata.updated_at != remote_updated_at {
        eprintln!();
        eprintln!("Warning: Remote has been updated since last pull.");
        eprintln!("Consider running 'a gh issue-agent pull --force' to update local copy.");
    }

    // Detect all changes
    let local_state = LocalState {
        metadata: &local.metadata,
        body: &local.body,
        comments: &local.comments,
    };
    let remote_state = RemoteState {
        issue: &remote.issue,
        comments: &remote.comments,
    };
    let options = DetectOptions {
        current_user: &ctx.current_user,
        // For diff display, show all changes including other users' comments
        edit_others: true,
        allow_delete: true,
    };
    let changeset = ChangeSet::detect(&local_state, &remote_state, &options)?;

    // Display changes
    if changeset.has_changes() {
        changeset.display()?;
    } else {
        println!();
        println!("No changes detected.");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::gh::issue_agent::commands::IssueArgs;
    use crate::commands::gh::issue_agent::commands::test_helpers::{
        TestSetup, create_comment_file, setup_local_comment, test_dir,
    };
    use crate::infra::github::RemoteComment;
    use rstest::rstest;
    use tempfile::TempDir;

    fn make_args() -> DiffArgs {
        DiffArgs {
            issue: IssueArgs {
                issue_number: 123,
                repo: Some("owner/repo".to_string()),
            },
        }
    }

    fn make_remote_comment<'a>(
        author: &'a str,
        id: &'a str,
        database_id: i64,
        body: &'a str,
    ) -> RemoteComment<'a> {
        RemoteComment {
            id,
            database_id,
            author,
            body,
        }
    }

    #[rstest]
    #[tokio::test]
    async fn test_no_changes(test_dir: TempDir) {
        let (mock, storage) = TestSetup::new(test_dir.path()).build().await;

        let client = mock.client();
        let result = run_with_client_and_storage(&make_args(), &client, &storage, "testuser").await;
        assert!(result.is_ok());
    }

    #[rstest]
    #[tokio::test]
    async fn test_body_diff(test_dir: TempDir) {
        let (mock, storage) = TestSetup::new(test_dir.path())
            .local_body("Local body")
            .remote_body("Remote body")
            .build()
            .await;

        let client = mock.client();
        let result = run_with_client_and_storage(&make_args(), &client, &storage, "testuser").await;
        assert!(result.is_ok());
    }

    #[rstest]
    #[tokio::test]
    async fn test_title_diff(test_dir: TempDir) {
        let (mock, storage) = TestSetup::new(test_dir.path())
            .local_title("Local title")
            .remote_title("Remote title")
            .build()
            .await;

        let client = mock.client();
        let result = run_with_client_and_storage(&make_args(), &client, &storage, "testuser").await;
        assert!(result.is_ok());
    }

    #[rstest]
    #[tokio::test]
    async fn test_labels_diff(test_dir: TempDir) {
        let (mock, storage) = TestSetup::new(test_dir.path())
            .local_labels(vec!["enhancement", "help wanted"])
            .build()
            .await;

        let client = mock.client();
        let result = run_with_client_and_storage(&make_args(), &client, &storage, "testuser").await;
        assert!(result.is_ok());
    }

    #[rstest]
    #[tokio::test]
    async fn test_comment_diff(test_dir: TempDir) {
        let (mock, storage) = TestSetup::new(test_dir.path())
            .remote_comments(vec![make_remote_comment(
                "testuser",
                "IC_abc",
                12345,
                "Original comment",
            )])
            .build()
            .await;
        setup_local_comment(
            test_dir.path(),
            "001_comment_12345.md",
            &create_comment_file(
                "testuser",
                "2024-01-01T12:00:00+00:00",
                "IC_abc",
                12345,
                "Updated comment",
            ),
        );

        let client = mock.client();
        let result = run_with_client_and_storage(&make_args(), &client, &storage, "testuser").await;
        assert!(result.is_ok());
    }

    #[rstest]
    #[tokio::test]
    async fn test_new_comment_diff(test_dir: TempDir) {
        let (mock, storage) = TestSetup::new(test_dir.path()).build().await;
        setup_local_comment(test_dir.path(), "new_my_comment.md", "New comment body");

        let client = mock.client();
        let result = run_with_client_and_storage(&make_args(), &client, &storage, "testuser").await;
        assert!(result.is_ok());
    }

    #[rstest]
    #[tokio::test]
    async fn test_deleted_comment_diff(test_dir: TempDir) {
        let (mock, storage) = TestSetup::new(test_dir.path())
            .remote_comments(vec![make_remote_comment(
                "testuser",
                "IC_abc",
                12345,
                "Comment to delete",
            )])
            .build()
            .await;
        // Don't create local comment file - simulating deletion

        let client = mock.client();
        let result = run_with_client_and_storage(&make_args(), &client, &storage, "testuser").await;
        // diff should succeed even when showing deleted comments
        assert!(result.is_ok());
    }

    #[rstest]
    #[tokio::test]
    async fn test_other_user_comment_diff(test_dir: TempDir) {
        // diff should show changes to other users' comments without error
        // (unlike push which requires --edit-others flag)
        let (mock, storage) = TestSetup::new(test_dir.path())
            .remote_comments(vec![make_remote_comment(
                "otheruser",
                "IC_abc",
                12345,
                "Original",
            )])
            .build()
            .await;
        setup_local_comment(
            test_dir.path(),
            "001_comment_12345.md",
            &create_comment_file(
                "otheruser",
                "2024-01-01T12:00:00+00:00",
                "IC_abc",
                12345,
                "Modified by current user",
            ),
        );

        let client = mock.client();
        let result = run_with_client_and_storage(&make_args(), &client, &storage, "testuser").await;
        assert!(result.is_ok());
    }

    #[rstest]
    #[tokio::test]
    async fn test_remote_changed_shows_warning_but_succeeds(test_dir: TempDir) {
        // diff should succeed even when remote has changed (unlike push which fails)
        let (mock, storage) = TestSetup::new(test_dir.path())
            .local_ts("2024-01-01T00:00:00+00:00")
            .remote_ts("2024-01-02T00:00:00+00:00")
            .build()
            .await;

        let client = mock.client();
        let result = run_with_client_and_storage(&make_args(), &client, &storage, "testuser").await;
        assert!(result.is_ok());
    }

    #[rstest]
    #[tokio::test]
    async fn test_invalid_repo_format(test_dir: TempDir) {
        let (mock, storage) = TestSetup::new(test_dir.path()).build().await;
        let args = DiffArgs {
            issue: IssueArgs {
                issue_number: 123,
                repo: Some("invalid-format".to_string()),
            },
        };

        let client = mock.client();
        let result = run_with_client_and_storage(&args, &client, &storage, "testuser").await;
        assert_eq!(
            result.unwrap_err().to_string(),
            "Invalid input: Invalid repository format: invalid-format. Expected owner/repo"
        );
    }
}
