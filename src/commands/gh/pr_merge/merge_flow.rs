use anyhow::{Context, bail};

use crate::{
    infra::github::{AsyncMergeFallback, AsyncMergeResult, PrState},
    shared::env_var::EnvVars,
};

use super::{
    MergeOutcome, PrMergeClient, PrSnapshot, ensure_pr_is_open_and_safe, unmerged_lower_pr,
};

pub(super) async fn run_merge(
    client: &impl PrMergeClient,
    hook_runner: &mut impl FnMut(&[(&str, &str)]) -> anyhow::Result<()>,
    owner: &str,
    repo: &str,
    pr_number: u64,
) -> anyhow::Result<MergeOutcome> {
    let initial = client.view_pr(owner, repo, pr_number)?;
    if initial.state == PrState::Merged {
        return Ok(MergeOutcome::AlreadyMerged(initial));
    }
    ensure_pr_is_open_and_safe(&initial, pr_number, None)?;

    let is_stacked = if let Some(stack) = client
        .get_stack_for_pull_request(owner, repo, pr_number)
        .await
        .context("Failed to inspect stack membership before merge")?
    {
        let stack_pull_requests = client
            .get_pull_requests_in_stack(owner, repo, stack.number)
            .await
            .context("Failed to inspect pull request states in the stack")?;
        if let Some(lower_pr) = unmerged_lower_pr(&stack_pull_requests, pr_number)? {
            bail!("PR #{pr_number} cannot be merged because lower PR #{lower_pr} is not merged.");
        }
        true
    } else {
        false
    };

    let pr_number_string = pr_number.to_string();
    let hook_env = [
        (EnvVars::pr_number_name(), pr_number_string.as_str()),
        (EnvVars::pr_owner_name(), owner),
        (EnvVars::pr_repo_name(), repo),
        (EnvVars::pr_url_name(), initial.url.as_str()),
        (EnvVars::pr_head_sha_name(), initial.head_sha.as_str()),
    ];
    hook_runner(&hook_env)?;

    client.wait_for_required_checks(owner, repo, pr_number)?;

    let current = client.view_pr(owner, repo, pr_number)?;
    if current.state == PrState::Merged {
        return Ok(MergeOutcome::Merged(current));
    }
    ensure_pr_is_open_and_safe(&current, pr_number, Some(&initial.head_sha))?;

    let merge_result = match client
        .merge_async(owner, repo, pr_number, &initial.head_sha)
        .await
    {
        Ok(result) => result,
        Err(error) => {
            return recover_async_merge_error(
                client, owner, repo, pr_number, &initial, is_stacked, error,
            )
            .await;
        }
    };

    match merge_result {
        AsyncMergeResult::Merged { .. } => Ok(merged_outcome_after_refresh(
            client, owner, repo, pr_number, current,
        )),
        AsyncMergeResult::Enqueued { message } => {
            Ok(MergeOutcome::Enqueued(message.unwrap_or_else(|| {
                "GitHub added the PR to the merge queue.".to_string()
            })))
        }
    }
}

async fn recover_async_merge_error(
    client: &impl PrMergeClient,
    owner: &str,
    repo: &str,
    pr_number: u64,
    initial: &PrSnapshot,
    is_stacked: bool,
    merge_error: anyhow::Error,
) -> anyhow::Result<MergeOutcome> {
    let Some(fallback) = merge_error.downcast_ref::<AsyncMergeFallback>().cloned() else {
        if let Some(merged) = already_merged(client, owner, repo, pr_number) {
            return Ok(MergeOutcome::Merged(merged));
        }
        return Err(merge_error);
    };
    if is_stacked {
        if let Some(merged) = already_merged(client, owner, repo, pr_number) {
            return Ok(MergeOutcome::Merged(merged));
        }
        return Err(merge_error).context(format!(
            "Cannot use synchronous fallback for stacked PR #{pr_number}. {}",
            pending_request_retry_note(&fallback),
        ));
    }

    fallback_to_sync_merge(client, owner, repo, pr_number, &initial.head_sha, &fallback).await
}

async fn fallback_to_sync_merge(
    client: &impl PrMergeClient,
    owner: &str,
    repo: &str,
    pr_number: u64,
    confirmed_head_sha: &str,
    fallback: &AsyncMergeFallback,
) -> anyhow::Result<MergeOutcome> {
    let request_uuid = fallback.request_uuid();
    let before_sync = client.view_pr(owner, repo, pr_number).with_context(|| {
        format!(
            "Could not verify PR #{pr_number} before synchronous fallback for async merge request {request_uuid}"
        )
    })?;
    if before_sync.state == PrState::Merged {
        return Ok(MergeOutcome::Merged(before_sync));
    }
    ensure_pr_is_open_and_safe(&before_sync, pr_number, Some(confirmed_head_sha)).with_context(
        || format!("Cannot fall back to synchronous merge for async merge request {request_uuid}"),
    )?;
    let merge_queue_enabled = client
        .branch_requires_merge_queue(owner, repo, &before_sync.base_ref_name)
        .await
        .with_context(|| {
            format!(
                "Could not determine whether PR #{pr_number} uses a merge queue before synchronous fallback for async merge request {request_uuid}"
            )
        })?;
    if merge_queue_enabled {
        bail!(
            "Cannot use synchronous fallback for PR #{pr_number} because base branch `{}` requires a merge queue. {}",
            before_sync.base_ref_name,
            pending_request_retry_note(fallback),
        );
    }

    if let Err(sync_error) = client
        .merge_sync(owner, repo, pr_number, confirmed_head_sha)
        .await
    {
        if let Some(merged) = already_merged(client, owner, repo, pr_number) {
            return Ok(MergeOutcome::Merged(merged));
        }
        return Err(sync_error).with_context(|| {
            format!(
                "Synchronous merge fallback failed for PR #{pr_number} after async merge request {request_uuid}"
            )
        });
    }

    Ok(merged_outcome_after_refresh(
        client,
        owner,
        repo,
        pr_number,
        before_sync,
    ))
}

fn pending_request_retry_note(fallback: &AsyncMergeFallback) -> String {
    let request_uuid = fallback.request_uuid();
    match fallback {
        AsyncMergeFallback::TimedOut { .. } => format!(
            "Pending async merge request {request_uuid} remains. Retrying waits on the same request and will time out again while it remains pending."
        ),
        AsyncMergeFallback::HeadMismatch {
            pending_head_sha,
            current_head_sha,
            ..
        } => format!(
            "Pending async merge request {request_uuid} remains. Retrying fails immediately because it targets head SHA {pending_head_sha}, not {current_head_sha}."
        ),
        AsyncMergeFallback::HeadUnavailable { .. } => format!(
            "Pending async merge request {request_uuid} remains. Retrying fails immediately because GitHub did not report its expected head SHA."
        ),
        AsyncMergeFallback::RequestUuidMismatch { .. } => format!(
            "Pending async merge request {request_uuid} remains. Retrying fails immediately because GitHub returned a different request UUID."
        ),
    }
}

fn already_merged(
    client: &impl PrMergeClient,
    owner: &str,
    repo: &str,
    pr_number: u64,
) -> Option<PrSnapshot> {
    client
        .view_pr(owner, repo, pr_number)
        .ok()
        .filter(|pr| pr.state == PrState::Merged)
}

fn merged_outcome_after_refresh(
    client: &impl PrMergeClient,
    owner: &str,
    repo: &str,
    pr_number: u64,
    fallback_snapshot: PrSnapshot,
) -> MergeOutcome {
    match client.view_pr(owner, repo, pr_number) {
        Ok(merged) if merged.state == PrState::Merged => MergeOutcome::Merged(merged),
        Ok(refreshed) => {
            eprintln!(
                "PR #{pr_number} was merged, but GitHub reports state {:?}; continuing with worktree close.",
                refreshed.state,
            );
            MergeOutcome::Merged(refreshed)
        }
        Err(error) => {
            eprintln!(
                "PR #{pr_number} was merged, but its state could not be refreshed: {error:#}; continuing with worktree close."
            );
            MergeOutcome::Merged(fallback_snapshot)
        }
    }
}
