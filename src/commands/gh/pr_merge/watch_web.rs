use std::{future::Future, io::Write};

use super::{GhCli, MergeOutcome, PrMergeClient, PrSnapshot, close_merged_head_worktree};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "result", content = "message", rename_all = "camelCase")]
pub(crate) enum WatchWebPrMergeResult {
    Merged,
    AlreadyMerged,
    Enqueued(String),
}

pub(super) async fn run_merge_and_close<C, F>(
    client: &impl PrMergeClient,
    hook_runner: &mut impl FnMut(&[(&str, &str)]) -> anyhow::Result<()>,
    owner: &str,
    repo: &str,
    pr_number: u64,
    close_worktree: C,
) -> anyhow::Result<()>
where
    C: FnOnce(PrSnapshot) -> F,
    F: Future<Output = anyhow::Result<()>>,
{
    run_merge_and_close_with_expected_head(
        client,
        hook_runner,
        owner,
        repo,
        pr_number,
        None,
        close_worktree,
    )
    .await
    .map(|_| ())
}

pub(super) async fn run_merge_and_close_with_expected_head<C, F>(
    client: &impl PrMergeClient,
    hook_runner: &mut impl FnMut(&[(&str, &str)]) -> anyhow::Result<()>,
    owner: &str,
    repo: &str,
    pr_number: u64,
    expected_head_sha: Option<&str>,
    close_worktree: C,
) -> anyhow::Result<MergeOutcome>
where
    C: FnOnce(PrSnapshot) -> F,
    F: Future<Output = anyhow::Result<()>>,
{
    let outcome = super::merge_flow::run_merge_with_expected_head(
        client,
        hook_runner,
        owner,
        repo,
        pr_number,
        expected_head_sha,
    )
    .await?;
    match &outcome {
        MergeOutcome::Merged(merged) => {
            println!("PR #{pr_number} merged.");
            let _ = std::io::stdout().flush();
            if let Err(error) = close_worktree(merged.clone()).await {
                eprintln!(
                    "Could not close worktree for merged PR #{pr_number} (branch `{}`): {error:#}",
                    merged.head_ref_name,
                );
            }
        }
        MergeOutcome::AlreadyMerged(_) => {
            println!("PR #{pr_number} was already merged.");
        }
        MergeOutcome::Enqueued(message) => {
            println!("PR #{pr_number} was added to the merge queue and has not merged: {message}");
        }
    }
    Ok(outcome)
}

pub(crate) async fn run_watch_web_pr_merge(
    owner: &str,
    repo: &str,
    pr_number: u64,
    expected_head_sha: &str,
) -> anyhow::Result<WatchWebPrMergeResult> {
    let client = GhCli;
    let mut hook_runner = |env: &[(&str, &str)]| super::hooks::run_hook("pre-pr-merge", env);
    match run_merge_and_close_with_expected_head(
        &client,
        &mut hook_runner,
        owner,
        repo,
        pr_number,
        Some(expected_head_sha),
        |merged_pr| async move { close_merged_head_worktree(&merged_pr).await },
    )
    .await?
    {
        MergeOutcome::Merged(_) => Ok(WatchWebPrMergeResult::Merged),
        MergeOutcome::AlreadyMerged(_) => Ok(WatchWebPrMergeResult::AlreadyMerged),
        MergeOutcome::Enqueued(message) => Ok(WatchWebPrMergeResult::Enqueued(message)),
    }
}
