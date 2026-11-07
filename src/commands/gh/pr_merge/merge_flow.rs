use anyhow::{Context, bail};

use crate::{
    infra::github::{AsyncMergeFallback, AsyncMergeResult, PrState},
    shared::env_var::EnvVars,
};

use super::{MergeOutcome, PrMergeClient, ensure_pr_is_open_and_safe, unmerged_lower_pr};

pub(super) async fn run_merge(
    client: &impl PrMergeClient,
    hook_runner: &mut impl FnMut(&[(&str, &str)]) -> anyhow::Result<()>,
    owner: &str,
    repo: &str,
    pr_number: u64,
) -> anyhow::Result<MergeOutcome> {
    let initial = client.view_pr(owner, repo, pr_number)?;
    ensure_pr_is_open_and_safe(&initial, pr_number, None)?;

    let mut is_stacked = false;
    if let Some(stack) = client
        .get_stack_for_pull_request(owner, repo, pr_number)
        .await
        .context("Failed to inspect stack membership before merge")?
    {
        is_stacked = true;
        let stack_pull_requests = client
            .get_pull_requests_in_stack(owner, repo, stack.number)
            .await
            .context("Failed to inspect pull request states in the stack")?;
        if let Some(lower_pr) = unmerged_lower_pr(&stack_pull_requests, pr_number)? {
            bail!("PR #{pr_number} cannot be merged because lower PR #{lower_pr} is not merged.");
        }
    }

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
    ensure_pr_is_open_and_safe(&current, pr_number, Some(&initial.head_sha))?;

    let merge_result = match client
        .merge_async(owner, repo, pr_number, &initial.head_sha)
        .await
    {
        Ok(result) => result,
        Err(merge_error) => {
            let Some(fallback) = merge_error.downcast_ref::<AsyncMergeFallback>() else {
                if let Ok(merged) = client.view_pr(owner, repo, pr_number)
                    && merged.state == PrState::Merged
                {
                    return Ok(MergeOutcome::Merged(merged));
                }
                return Err(merge_error);
            };
            let request_uuid = fallback.request_uuid();
            if is_stacked {
                if let Ok(merged) = client.view_pr(owner, repo, pr_number)
                    && merged.state == PrState::Merged
                {
                    return Ok(MergeOutcome::Merged(merged));
                }
                return Err(merge_error);
            }

            let before_sync = client.view_pr(owner, repo, pr_number).with_context(|| {
                format!(
                    "Could not verify PR #{pr_number} before synchronous fallback for async merge request {request_uuid}"
                )
            })?;
            if before_sync.state == PrState::Merged {
                return Ok(MergeOutcome::Merged(before_sync));
            }
            ensure_pr_is_open_and_safe(&before_sync, pr_number, Some(&initial.head_sha))
                .with_context(|| {
                    format!(
                        "Cannot fall back to synchronous merge for async merge request {request_uuid}"
                    )
                })?;
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
                    "Cannot fall back to synchronous merge for PR #{pr_number} because base branch `{}` requires a merge queue; async merge request {request_uuid} is still pending.",
                    before_sync.base_ref_name
                );
            }

            if let Err(sync_error) = client
                .merge_sync(owner, repo, pr_number, &initial.head_sha)
                .await
            {
                if let Ok(merged) = client.view_pr(owner, repo, pr_number)
                    && merged.state == PrState::Merged
                {
                    return Ok(MergeOutcome::Merged(merged));
                }
                return Err(sync_error).with_context(|| {
                    format!(
                        "Synchronous merge fallback failed for PR #{pr_number} after async merge request {request_uuid}"
                    )
                });
            }
            return match client.view_pr(owner, repo, pr_number) {
                Ok(merged) if merged.state == PrState::Merged => Ok(MergeOutcome::Merged(merged)),
                Ok(refreshed) => {
                    eprintln!(
                        "PR #{pr_number} was merged, but GitHub reports state {:?}; continuing with worktree close.",
                        refreshed.state,
                    );
                    Ok(MergeOutcome::Merged(refreshed))
                }
                Err(error) => {
                    eprintln!(
                        "PR #{pr_number} was merged, but its state could not be refreshed: {error:#}; continuing with worktree close."
                    );
                    Ok(MergeOutcome::Merged(before_sync))
                }
            };
        }
    };

    match merge_result {
        AsyncMergeResult::Merged { .. } => match client.view_pr(owner, repo, pr_number) {
            Ok(merged) if merged.state == PrState::Merged => Ok(MergeOutcome::Merged(merged)),
            Ok(refreshed) => {
                eprintln!(
                    "PR #{pr_number} was merged, but GitHub reports state {:?}; continuing with worktree close.",
                    refreshed.state,
                );
                Ok(MergeOutcome::Merged(refreshed))
            }
            Err(error) => {
                eprintln!(
                    "PR #{pr_number} was merged, but its state could not be refreshed: {error:#}; continuing with worktree close."
                );
                Ok(MergeOutcome::Merged(current))
            }
        },
        AsyncMergeResult::Enqueued { message } => {
            Ok(MergeOutcome::Enqueued(message.unwrap_or_else(|| {
                "GitHub added the PR to the merge queue.".to_string()
            })))
        }
    }
}
