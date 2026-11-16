use anyhow::{Context, bail};

use crate::{
    infra::github::{AsyncMergeFallback, AsyncMergeResult, PrState},
    shared::env_var::EnvVars,
};

use super::{
    MergeOutcome, MergedPr, PrMergeClient, PrSnapshot, ensure_pr_is_open_and_safe,
    unmerged_lower_pr,
};

#[derive(Clone, Copy)]
pub(super) struct MergeOptions<'a> {
    pub expected_head_sha: Option<&'a str>,
    pub allow_failing_base: bool,
    pub run_branch_commit_checks: bool,
}

#[derive(Clone, Copy)]
struct PostMergeCheckContext<'a> {
    owner: &'a str,
    repo: &'a str,
    pr_number: u64,
    discover_merge_checks: bool,
}

fn check_base_branch_before_merge(
    client: &impl PrMergeClient,
    owner: &str,
    repo: &str,
    pr_number: u64,
    branch: &str,
    allow_failing_base: bool,
) -> anyhow::Result<bool> {
    client
        .wait_for_base_branch_checks(owner, repo, branch, allow_failing_base)
        .with_context(|| {
            format!(
                "Cannot merge PR #{pr_number} while required checks on base branch `{branch}` are failing"
            )
        })
}

#[cfg(test)]
pub(super) async fn run_merge(
    client: &impl PrMergeClient,
    hook_runner: &mut impl FnMut(&[(&str, &str)]) -> anyhow::Result<()>,
    owner: &str,
    repo: &str,
    pr_number: u64,
) -> anyhow::Result<MergeOutcome> {
    run_merge_with_options(
        client,
        hook_runner,
        owner,
        repo,
        pr_number,
        MergeOptions {
            expected_head_sha: None,
            allow_failing_base: false,
            run_branch_commit_checks: true,
        },
    )
    .await
}

#[cfg(test)]
pub(super) async fn run_merge_with_expected_head(
    client: &impl PrMergeClient,
    hook_runner: &mut impl FnMut(&[(&str, &str)]) -> anyhow::Result<()>,
    owner: &str,
    repo: &str,
    pr_number: u64,
    expected_head_sha: Option<&str>,
) -> anyhow::Result<MergeOutcome> {
    run_merge_with_options(
        client,
        hook_runner,
        owner,
        repo,
        pr_number,
        MergeOptions {
            expected_head_sha,
            allow_failing_base: false,
            run_branch_commit_checks: false,
        },
    )
    .await
}

pub(super) async fn run_merge_with_options(
    client: &impl PrMergeClient,
    hook_runner: &mut impl FnMut(&[(&str, &str)]) -> anyhow::Result<()>,
    owner: &str,
    repo: &str,
    pr_number: u64,
    options: MergeOptions<'_>,
) -> anyhow::Result<MergeOutcome> {
    let initial = client.view_pr(owner, repo, pr_number)?;
    if initial.state == PrState::Merged {
        return Ok(MergeOutcome::AlreadyMerged(initial));
    }
    ensure_pr_is_open_and_safe(&initial, pr_number, None)?;
    if options
        .expected_head_sha
        .is_some_and(|expected| expected != initial.head_sha)
    {
        bail!("PR #{pr_number} head changed since confirmation.");
    }

    let mut discover_merge_checks = if options.run_branch_commit_checks {
        check_base_branch_before_merge(
            client,
            owner,
            repo,
            pr_number,
            &initial.base_ref_name,
            options.allow_failing_base,
        )?
    } else {
        false
    };

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
        return Ok(MergeOutcome::Merged(MergedPr {
            snapshot: current,
            discover_merge_checks: options.run_branch_commit_checks,
        }));
    }
    ensure_pr_is_open_and_safe(&current, pr_number, Some(&initial.head_sha))?;

    if options.run_branch_commit_checks {
        discover_merge_checks = check_base_branch_before_merge(
            client,
            owner,
            repo,
            pr_number,
            &current.base_ref_name,
            options.allow_failing_base,
        )?;
    }

    let post_merge_check_context = PostMergeCheckContext {
        owner,
        repo,
        pr_number,
        discover_merge_checks,
    };

    let merge_result = match client
        .merge_async(owner, repo, pr_number, &initial.head_sha)
        .await
    {
        Ok(result) => result,
        Err(error) => {
            return recover_async_merge_error(
                client,
                &initial,
                is_stacked,
                post_merge_check_context,
                error,
            )
            .await;
        }
    };

    match merge_result {
        AsyncMergeResult::Merged { sha } => Ok(merged_outcome_after_refresh(
            client,
            post_merge_check_context,
            current,
            sha,
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
    initial: &PrSnapshot,
    is_stacked: bool,
    context: PostMergeCheckContext<'_>,
    merge_error: anyhow::Error,
) -> anyhow::Result<MergeOutcome> {
    let Some(fallback) = merge_error.downcast_ref::<AsyncMergeFallback>().cloned() else {
        if let Some(merged) = already_merged(client, context.owner, context.repo, context.pr_number)
        {
            return Ok(MergeOutcome::Merged(MergedPr {
                snapshot: merged,
                discover_merge_checks: context.discover_merge_checks,
            }));
        }
        return Err(merge_error);
    };
    if is_stacked {
        if let Some(merged) = already_merged(client, context.owner, context.repo, context.pr_number)
        {
            return Ok(MergeOutcome::Merged(MergedPr {
                snapshot: merged,
                discover_merge_checks: context.discover_merge_checks,
            }));
        }
        return Err(merge_error).context(format!(
            "Cannot use synchronous fallback for stacked PR #{}. {}",
            context.pr_number,
            pending_request_retry_note(&fallback),
        ));
    }

    fallback_to_sync_merge(client, context, &initial.head_sha, &fallback).await
}

async fn fallback_to_sync_merge(
    client: &impl PrMergeClient,
    context: PostMergeCheckContext<'_>,
    confirmed_head_sha: &str,
    fallback: &AsyncMergeFallback,
) -> anyhow::Result<MergeOutcome> {
    let request_uuid = fallback.request_uuid();
    let before_sync = client
        .view_pr(context.owner, context.repo, context.pr_number)
        .with_context(|| {
        format!(
            "Could not verify PR #{} before synchronous fallback for async merge request {request_uuid}",
            context.pr_number
        )
    })?;
    if before_sync.state == PrState::Merged {
        return Ok(MergeOutcome::Merged(MergedPr {
            snapshot: before_sync,
            discover_merge_checks: context.discover_merge_checks,
        }));
    }
    ensure_pr_is_open_and_safe(&before_sync, context.pr_number, Some(confirmed_head_sha))
        .with_context(|| {
            format!("Cannot fall back to synchronous merge for async merge request {request_uuid}")
        })?;
    let merge_queue_enabled = client
        .branch_requires_merge_queue(context.owner, context.repo, &before_sync.base_ref_name)
        .await
        .with_context(|| {
            format!(
                "Could not determine whether PR #{} uses a merge queue before synchronous fallback for async merge request {request_uuid}",
                context.pr_number
            )
        })?;
    if merge_queue_enabled {
        bail!(
            "Cannot use synchronous fallback for PR #{} because base branch `{}` requires a merge queue. {}",
            context.pr_number,
            before_sync.base_ref_name,
            pending_request_retry_note(fallback),
        );
    }

    let merge_result = match client
        .merge_sync(
            context.owner,
            context.repo,
            context.pr_number,
            confirmed_head_sha,
        )
        .await
    {
        Ok(result) => result,
        Err(sync_error) => {
            if let Some(merged) =
                already_merged(client, context.owner, context.repo, context.pr_number)
            {
                return Ok(MergeOutcome::Merged(MergedPr {
                    snapshot: merged,
                    discover_merge_checks: context.discover_merge_checks,
                }));
            }
            return Err(sync_error).with_context(|| {
                format!(
                    "Synchronous merge fallback failed for PR #{} after async merge request {request_uuid}",
                    context.pr_number
                )
            });
        }
    };

    Ok(merged_outcome_after_refresh(
        client,
        context,
        before_sync,
        match merge_result {
            crate::infra::github::MergeResult::Merged { sha } => sha,
            crate::infra::github::MergeResult::Enqueued { .. } => None,
        },
    ))
}

fn pending_request_retry_note(fallback: &AsyncMergeFallback) -> String {
    format!(
        "Pending async merge request {} remains. {}",
        fallback.request_uuid(),
        fallback.retry_hint(),
    )
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
    context: PostMergeCheckContext<'_>,
    fallback_snapshot: PrSnapshot,
    merge_sha: Option<String>,
) -> MergeOutcome {
    match client.view_pr(context.owner, context.repo, context.pr_number) {
        Ok(mut merged) if merged.state == PrState::Merged => {
            merged.merge_commit_sha = merge_sha.or(merged.merge_commit_sha);
            MergeOutcome::Merged(MergedPr {
                snapshot: merged,
                discover_merge_checks: context.discover_merge_checks,
            })
        }
        Ok(refreshed) => {
            eprintln!(
                "PR #{} was merged, but GitHub reports state {:?}; continuing with worktree close.",
                context.pr_number, refreshed.state,
            );
            let mut refreshed = refreshed;
            refreshed.merge_commit_sha = merge_sha.or(refreshed.merge_commit_sha);
            MergeOutcome::Merged(MergedPr {
                snapshot: refreshed,
                discover_merge_checks: context.discover_merge_checks,
            })
        }
        Err(error) => {
            eprintln!(
                "PR #{} was merged, but its state could not be refreshed: {error:#}; continuing with worktree close.",
                context.pr_number
            );
            let mut fallback_snapshot = fallback_snapshot;
            fallback_snapshot.merge_commit_sha = merge_sha.or(fallback_snapshot.merge_commit_sha);
            MergeOutcome::Merged(MergedPr {
                snapshot: fallback_snapshot,
                discover_merge_checks: context.discover_merge_checks,
            })
        }
    }
}
