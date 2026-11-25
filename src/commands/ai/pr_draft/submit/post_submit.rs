use std::path::Path;

use super::PrTarget;
use crate::commands::agent;
use crate::commands::ai::pr_series::{
    PrSeriesPlan, load_plan_if_present, preserve_managed_section,
};
use crate::commands::ai::pr_series_aftercare::{
    SubmitSyncFailure, SubmitSyncOutcome, SubmitSyncStage, link_and_sync_after_submit,
};
use crate::infra::git::cmd::run_git_optional;
use crate::infra::github::{GitHubClient, PrBodyClient};
use crate::infra::tq::TqClient;
use crate::shared::env_var::EnvVars;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct TqBranchMetadata {
    pub(super) task_id: String,
    pub(super) checklist_item_id: String,
}

#[derive(Clone)]
pub(super) struct SavedPrSeriesPlan {
    pub(super) task_id: String,
    pub(super) plan: PrSeriesPlan,
}

pub(super) trait PostSubmitProcessor {
    fn metadata_for_branch(&self, branch: &str) -> Option<TqBranchMetadata>;

    async fn saved_plan(&self, metadata: &TqBranchMetadata) -> Option<SavedPrSeriesPlan>;

    async fn link_and_sync(
        &self,
        metadata: &TqBranchMetadata,
        target: &PrTarget,
        pr_url: &str,
        saved_plan: Option<SavedPrSeriesPlan>,
    ) -> std::result::Result<SubmitSyncOutcome, SubmitSyncFailure>;
}

pub(super) struct SubmitEffects<'a, P, R> {
    pub(super) post_submit_processor: &'a P,
    pub(super) record_created_pr_url: R,
    pub(super) capture_created_pr_url: Option<&'a (dyn Fn(&str) + Send + Sync)>,
}

pub(super) struct PrSeriesPostSubmitProcessor<'a> {
    pub(super) github: &'a GitHubClient,
}

impl PostSubmitProcessor for PrSeriesPostSubmitProcessor<'_> {
    fn metadata_for_branch(&self, branch: &str) -> Option<TqBranchMetadata> {
        read_current_branch_tq_metadata(branch)
    }

    async fn saved_plan(&self, metadata: &TqBranchMetadata) -> Option<SavedPrSeriesPlan> {
        match load_plan_if_present(&metadata.task_id) {
            Ok(Some(plan)) => Some(SavedPrSeriesPlan {
                task_id: metadata.task_id.clone(),
                plan,
            }),
            Ok(None) if metadata.task_id.parse::<u32>().is_ok() => {
                let Some(tq_client) = TqClient::detect() else {
                    tracing::warn!(
                        event = "pr_draft.submit.plan_task_resolution_failed",
                        task_id = %metadata.task_id,
                        "tq CLI is unavailable for resolving the canonical task ID"
                    );
                    return None;
                };
                let task = match tq_client.task_details(&metadata.task_id).await {
                    Ok(task) => task,
                    Err(error) => {
                        tracing::warn!(
                            event = "pr_draft.submit.plan_task_resolution_failed",
                            task_id = %metadata.task_id,
                            error = %error,
                            "failed to resolve the canonical tq task ID"
                        );
                        return None;
                    }
                };
                match load_plan_if_present(&task.id) {
                    Ok(Some(plan)) => Some(SavedPrSeriesPlan {
                        task_id: task.id,
                        plan,
                    }),
                    Ok(None) => None,
                    Err(error) => {
                        warn_plan_lookup_failure(metadata, &error);
                        None
                    }
                }
            }
            Ok(None) => None,
            Err(error) => {
                warn_plan_lookup_failure(metadata, &error);
                None
            }
        }
    }

    async fn link_and_sync(
        &self,
        metadata: &TqBranchMetadata,
        target: &PrTarget,
        pr_url: &str,
        saved_plan: Option<SavedPrSeriesPlan>,
    ) -> std::result::Result<SubmitSyncOutcome, SubmitSyncFailure> {
        let Some(saved_plan) = saved_plan else {
            return Ok(SubmitSyncOutcome::NoPlan);
        };
        link_and_sync_after_submit(
            self.github,
            &saved_plan.task_id,
            &metadata.checklist_item_id,
            &target.owner,
            &target.repo,
            pr_url,
            saved_plan.plan,
        )
        .await
    }
}

fn warn_plan_lookup_failure(metadata: &TqBranchMetadata, error: &anyhow::Error) {
    tracing::warn!(
        event = "pr_draft.submit.plan_check_failed",
        task_id = %metadata.task_id,
        error = %error,
        "failed to check for a PR series plan"
    );
}

pub(super) fn record_created_pr_url(pr_url: &str) {
    let Some(session_id) = EnvVars::load().own_session_id() else {
        tracing::warn!(
            event = "pr_draft.submit.session_pr_record_failed",
            "calling agent session is unavailable"
        );
        return;
    };
    let result = agent::store::sessions_dir()
        .and_then(|sessions_dir| record_created_pr_url_in(&sessions_dir, &session_id, pr_url));
    if let Err(error) = result {
        tracing::warn!(
            event = "pr_draft.submit.session_pr_record_failed",
            session = %session_id,
            error = %error,
            "failed to record pull request on the calling session"
        );
    }
}

pub(super) fn record_created_pr_url_in(
    sessions_dir: &Path,
    session_id: &str,
    pr_url: &str,
) -> anyhow::Result<()> {
    let lock = agent::store::lock_session_for_update(sessions_dir, session_id)?;
    let Some(mut session) = lock.load()? else {
        anyhow::bail!("agent session not found: {session_id}");
    };
    if !session.created_pr_urls.iter().any(|url| url == pr_url) {
        session.created_pr_urls.push(pr_url.to_owned());
        lock.save(&session)?;
    }
    Ok(())
}

pub(super) async fn body_for_existing_pr_update(
    gh_client: &impl PrBodyClient,
    owner: &str,
    repo: &str,
    number: u64,
    submitted_body: &str,
    has_saved_plan: bool,
    metadata: Option<&TqBranchMetadata>,
) -> String {
    if !has_saved_plan {
        return submitted_body.to_string();
    }
    let Some(metadata) = metadata else {
        return submitted_body.to_string();
    };

    let existing_body = match gh_client.get_pull_request_body(owner, repo, number).await {
        Ok(body) => body,
        Err(error) => {
            warn_pr_series_body_fallback(
                "pr_draft.submit.read_body_failed",
                owner,
                repo,
                number,
                metadata,
                &error,
            );
            return submitted_body.to_string();
        }
    };

    match preserve_managed_section(&existing_body, submitted_body) {
        Ok(body) => body,
        Err(error) => {
            warn_pr_series_body_fallback(
                "pr_draft.submit.preserve_section_failed",
                owner,
                repo,
                number,
                metadata,
                &error,
            );
            submitted_body.to_string()
        }
    }
}

fn warn_pr_series_body_fallback(
    event: &'static str,
    owner: &str,
    repo: &str,
    number: u64,
    metadata: &TqBranchMetadata,
    error: &anyhow::Error,
) {
    tracing::warn!(
        event = event,
        task_id = %metadata.task_id,
        owner,
        repo,
        number,
        error = %error,
        "failed to preserve the PR series section while updating the PR body"
    );
    eprintln!(
        "Warning: could not preserve the PR series section for {owner}/{repo} pull request {number}: {error}. Continuing with the draft body. If synchronization does not restore the section, run `a ai pr-series sync {}`.",
        metadata.task_id,
    );
}

fn read_current_branch_tq_metadata(branch: &str) -> Option<TqBranchMetadata> {
    tq_branch_metadata(
        read_branch_config_value(branch, "x-tq-task-id"),
        read_branch_config_value(branch, "x-tq-checklist-item-id"),
    )
}

fn tq_branch_metadata(
    task_id: Option<String>,
    checklist_item_id: Option<String>,
) -> Option<TqBranchMetadata> {
    let task_id = task_id.filter(|value| !value.trim().is_empty())?;
    let checklist_item_id = checklist_item_id.filter(|value| !value.trim().is_empty())?;
    Some(TqBranchMetadata {
        task_id,
        checklist_item_id,
    })
}

fn read_branch_config_value(branch: &str, name: &str) -> Option<String> {
    let key = format!("branch.{branch}.{name}");
    run_git_optional(Path::new("."), ["config", "--local", "--get", &key])
        .filter(|value| !value.trim().is_empty())
}

pub(super) fn format_post_submit_failure(
    failure: &SubmitSyncFailure,
    metadata: &TqBranchMetadata,
    pr_url: &str,
) -> String {
    match failure.stage {
        SubmitSyncStage::Prepare => format!(
            "PR submit completed, but PR series setup failed: {}\nAfter resolving the error, run `tq checklist item update {} --github {}` and then `a ai pr-series sync {}`.",
            failure.error, metadata.checklist_item_id, pr_url, metadata.task_id
        ),
        SubmitSyncStage::Link => format!(
            "PR submit completed, but its tq checklist link failed: {}\nAfter resolving the error, run `tq checklist item update {} --github {}` and then `a ai pr-series sync {}`.",
            failure.error, metadata.checklist_item_id, pr_url, metadata.task_id
        ),
        SubmitSyncStage::Sync => format!(
            "PR submit completed and linked, but PR series synchronization failed: {}\nAfter resolving the listed errors, run `a ai pr-series sync {}`.",
            failure.error, metadata.task_id
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::agent::types::SessionPrContext;
    use crate::commands::ai::pr_series_aftercare::{SubmitSyncFailure, SubmitSyncStage};
    use indoc::indoc;
    use rstest::{fixture, rstest};

    #[fixture]
    fn sessions_dir_with_cwd_pr_context() -> tempfile::TempDir {
        let sessions_dir = tempfile::tempdir().expect("temporary session directory should exist");
        let session: crate::commands::agent::types::Session =
            serde_json::from_value(serde_json::json!({
                "session_id": "sample-session",
                "pr_context": {
                    "owner": "sample-owner",
                    "repo": "sample-repository",
                    "branch": "cwd-branch",
                },
                "cwd": "/workspace/sample",
                "transcript_path": null,
                "tmux_info": null,
                "status": "stopped",
                "created_at": "2026-01-01T00:00:00Z",
                "updated_at": "2026-01-01T00:00:00Z",
                "last_message": null,
            }))
            .expect("fixture session should deserialize");
        crate::commands::agent::store::save_session_to(sessions_dir.path(), &session)
            .expect("fixture session should save");
        sessions_dir
    }

    #[rstest]
    fn created_pr_url_is_saved_once_without_replacing_the_cwd_context(
        sessions_dir_with_cwd_pr_context: tempfile::TempDir,
    ) {
        let sessions_dir = sessions_dir_with_cwd_pr_context;
        let pr_url = "https://github.com/sample-owner/sample-repository/pull/17";

        record_created_pr_url_in(sessions_dir.path(), "sample-session", pr_url)
            .expect("pull request should be recorded");
        record_created_pr_url_in(sessions_dir.path(), "sample-session", pr_url)
            .expect("repeated recording should be idempotent");

        let session =
            crate::commands::agent::store::load_session_from(sessions_dir.path(), "sample-session")
                .expect("session store should be readable")
                .expect("session should remain stored");
        assert_eq!(
            (session.pr_context, session.created_pr_urls),
            (
                Some(SessionPrContext {
                    owner: "sample-owner".to_string(),
                    repo: "sample-repository".to_string(),
                    branch: "cwd-branch".to_string(),
                    checklist_item_id: None,
                }),
                vec![pr_url.to_string()],
            ),
        );
    }

    #[rstest]
    #[case::prepare(
        SubmitSyncStage::Prepare,
        indoc! {"
            PR submit completed, but PR series setup failed: sample synchronization failure
            After resolving the error, run `tq checklist item update sample-leaf-id --github https://github.com/acme/notify-api/pull/17` and then `a ai pr-series sync sample-task-id`.
        "}.trim_end()
    )]
    #[case::link(
        SubmitSyncStage::Link,
        indoc! {"
            PR submit completed, but its tq checklist link failed: sample synchronization failure
            After resolving the error, run `tq checklist item update sample-leaf-id --github https://github.com/acme/notify-api/pull/17` and then `a ai pr-series sync sample-task-id`.
        "}.trim_end()
    )]
    #[case::sync(
        SubmitSyncStage::Sync,
        indoc! {"
            PR submit completed and linked, but PR series synchronization failed: sample synchronization failure
            After resolving the listed errors, run `a ai pr-series sync sample-task-id`.
        "}.trim_end()
    )]
    fn formats_post_submit_recovery_instructions(
        #[case] stage: SubmitSyncStage,
        #[case] expected: &str,
    ) {
        let failure = SubmitSyncFailure {
            stage,
            error: anyhow::anyhow!("sample synchronization failure"),
        };
        let metadata = TqBranchMetadata {
            task_id: "sample-task-id".to_string(),
            checklist_item_id: "sample-leaf-id".to_string(),
        };

        assert_eq!(
            format_post_submit_failure(
                &failure,
                &metadata,
                "https://github.com/acme/notify-api/pull/17",
            ),
            expected,
        );
    }

    #[rstest]
    #[case::both_ids(
        Some("sample-task-id".to_string()),
        Some("sample-leaf-id".to_string()),
        Some(TqBranchMetadata {
            task_id: "sample-task-id".to_string(),
            checklist_item_id: "sample-leaf-id".to_string(),
        })
    )]
    #[case::missing_task_id(None, Some("sample-leaf-id".to_string()), None)]
    #[case::missing_leaf_id(Some("sample-task-id".to_string()), None, None)]
    fn requires_both_branch_tq_ids(
        #[case] task_id: Option<String>,
        #[case] checklist_item_id: Option<String>,
        #[case] expected: Option<TqBranchMetadata>,
    ) {
        assert_eq!(tq_branch_metadata(task_id, checklist_item_id), expected);
    }
}
