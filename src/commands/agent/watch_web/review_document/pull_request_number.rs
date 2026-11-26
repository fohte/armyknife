use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use futures_util::stream::{self, StreamExt};

use super::metadata::ReviewDocumentSummary;
use crate::infra::github::{GitHubClient, PrClient, PrState};

pub(super) type PullRequestNumberFuture =
    Pin<Box<dyn Future<Output = anyhow::Result<Option<u64>>> + Send>>;
pub(super) type PullRequestNumberLookup =
    Arc<dyn Fn(String, String, String) -> PullRequestNumberFuture + Send + Sync>;

const LOOKUP_TIMEOUT: Duration = Duration::from_secs(2);

pub(super) fn github_open_pull_request_number_lookup() -> PullRequestNumberLookup {
    Arc::new(|owner, repository, branch| {
        Box::pin(async move {
            let client = tokio::task::spawn_blocking(GitHubClient::get).await??;
            Ok(client
                .get_pr_for_branch(&owner, &repository, &branch)
                .await?
                .filter(|pull_request| pull_request.state == PrState::Open)
                .map(|pull_request| pull_request.number))
        })
    })
}

pub(super) async fn lookup_open_pull_request_numbers(
    summaries: Vec<ReviewDocumentSummary>,
    lookup: PullRequestNumberLookup,
) -> Vec<ReviewDocumentSummary> {
    let deadline = tokio::time::Instant::now() + LOOKUP_TIMEOUT;
    stream::iter(summaries)
        .map(|mut summary| {
            let lookup = Arc::clone(&lookup);
            async move {
                if summary.kind != super::metadata::ReviewDocumentKind::PrDraft {
                    return summary;
                }

                let Some(repository) = summary.target.repository.as_deref() else {
                    return summary;
                };
                let Some(branch) = summary.target.branch.as_deref() else {
                    return summary;
                };
                let Some((owner, repository)) = repository.split_once('/') else {
                    return summary;
                };
                if owner.is_empty() || repository.is_empty() || repository.contains('/') {
                    return summary;
                }

                let result = tokio::time::timeout_at(
                    deadline,
                    lookup(owner.to_owned(), repository.to_owned(), branch.to_owned()),
                )
                .await;
                match result {
                    Ok(Ok(Some(number))) => summary.target.pull_request_number = Some(number),
                    Ok(Ok(None)) => {}
                    Ok(Err(error)) => {
                        summary.target.pull_request_lookup_unavailable = Some(true);
                        tracing::warn!(%error, "failed to look up PR draft target");
                    }
                    Err(_) => {
                        summary.target.pull_request_lookup_unavailable = Some(true);
                        tracing::warn!("timed out looking up PR draft target");
                    }
                }
                summary
            }
        })
        .buffered(4)
        .collect()
        .await
}

#[cfg(test)]
pub(super) fn no_pull_request_number_lookup() -> PullRequestNumberLookup {
    Arc::new(|_, _, _| Box::pin(async { Ok(None) }))
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
    use crate::commands::agent::watch_web::review_document::metadata::{
        ReviewDocumentKind, ReviewDocumentTarget,
    };

    fn summary(
        kind: ReviewDocumentKind,
        repository: Option<&str>,
        branch: Option<&str>,
    ) -> ReviewDocumentSummary {
        ReviewDocumentSummary {
            id: "fixture-review".to_owned(),
            kind,
            target: ReviewDocumentTarget {
                repository: repository.map(str::to_owned),
                branch: branch.map(str::to_owned),
                ..ReviewDocumentTarget::default()
            },
            started_at: None,
            is_editor: true,
            can_save: true,
            can_finish: true,
        }
    }

    #[tokio::test]
    async fn adds_open_pull_request_numbers_only_for_pull_request_drafts() {
        let calls = Arc::new(AtomicUsize::new(0));
        let lookup_calls = Arc::clone(&calls);
        let lookup: PullRequestNumberLookup = Arc::new(move |owner, repository, branch| {
            let lookup_calls = Arc::clone(&lookup_calls);
            Box::pin(async move {
                lookup_calls.fetch_add(1, Ordering::Relaxed);
                Ok((owner.as_str(), repository.as_str(), branch.as_str())
                    .eq(&("sample-owner", "sample-repo", "feature/example"))
                    .then_some(42))
            })
        });

        let summaries = lookup_open_pull_request_numbers(
            vec![
                summary(
                    ReviewDocumentKind::PrDraft,
                    Some("sample-owner/sample-repo"),
                    Some("feature/example"),
                ),
                summary(
                    ReviewDocumentKind::Draft,
                    Some("sample-owner/sample-repo"),
                    Some("feature/example"),
                ),
            ],
            lookup,
        )
        .await;

        let mut expected_pr_draft = summary(
            ReviewDocumentKind::PrDraft,
            Some("sample-owner/sample-repo"),
            Some("feature/example"),
        );
        expected_pr_draft.target.pull_request_number = Some(42);
        assert_eq!(
            (summaries, calls.load(Ordering::Relaxed)),
            (
                vec![
                    expected_pr_draft,
                    summary(
                        ReviewDocumentKind::Draft,
                        Some("sample-owner/sample-repo"),
                        Some("feature/example"),
                    ),
                ],
                1,
            ),
        );
    }

    #[tokio::test]
    async fn marks_the_pr_number_unavailable_when_lookup_times_out() {
        let lookup: PullRequestNumberLookup = Arc::new(|_, _, _| Box::pin(std::future::pending()));
        let summaries = lookup_open_pull_request_numbers(
            vec![summary(
                ReviewDocumentKind::PrDraft,
                Some("sample-owner/sample-repo"),
                Some("feature/example"),
            )],
            lookup,
        )
        .await;
        let mut expected = summary(
            ReviewDocumentKind::PrDraft,
            Some("sample-owner/sample-repo"),
            Some("feature/example"),
        );
        expected.target.pull_request_lookup_unavailable = Some(true);

        assert_eq!(summaries, vec![expected]);
    }
}
