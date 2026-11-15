use std::{future::Future, io::Write};

use anyhow::Context;

use super::{GhCli, MergeOutcome, PrMergeClient, PrSnapshot, close_merged_head_worktree};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "result", content = "message", rename_all = "camelCase")]
pub(crate) enum WatchWebPrMergeResult {
    Merged,
    AlreadyMerged,
    Enqueued(String),
}

pub(super) struct MergeOptions<'a> {
    pub expected_head_sha: Option<&'a str>,
    pub allow_failing_base: bool,
}

#[cfg(test)]
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
    run_merge_and_close_with_options(
        client,
        hook_runner,
        owner,
        repo,
        pr_number,
        MergeOptions {
            expected_head_sha: None,
            allow_failing_base: false,
        },
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
    run_merge_and_close_with_options(
        client,
        hook_runner,
        owner,
        repo,
        pr_number,
        MergeOptions {
            expected_head_sha,
            allow_failing_base: false,
        },
        close_worktree,
    )
    .await
}

pub(super) async fn run_merge_and_close_with_options<C, F>(
    client: &impl PrMergeClient,
    hook_runner: &mut impl FnMut(&[(&str, &str)]) -> anyhow::Result<()>,
    owner: &str,
    repo: &str,
    pr_number: u64,
    options: MergeOptions<'_>,
    close_worktree: C,
) -> anyhow::Result<MergeOutcome>
where
    C: FnOnce(PrSnapshot) -> F,
    F: Future<Output = anyhow::Result<()>>,
{
    let outcome = super::merge_flow::run_merge_with_options(
        client,
        hook_runner,
        owner,
        repo,
        pr_number,
        options.expected_head_sha,
        options.allow_failing_base,
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
            client
                .wait_for_merge_commit_checks(
                    owner,
                    repo,
                    &merged.base_ref_name,
                    merged.merge_commit_sha.as_deref(),
                )
                .with_context(|| {
                    format!(
                        "PR #{pr_number} was merged, but required checks on the merge commit failed"
                    )
                })?;
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
