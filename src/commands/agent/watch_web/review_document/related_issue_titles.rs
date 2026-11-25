use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use futures_util::stream::{self, StreamExt};
use serde_json::Value;

use crate::infra::github::GitHubClient;

pub(super) type IssueTitleFuture = Pin<Box<dyn Future<Output = anyhow::Result<String>> + Send>>;
pub(super) type IssueTitleLookup =
    Arc<dyn Fn(String, String, u64) -> IssueTitleFuture + Send + Sync>;

const ISSUE_TITLE_LOOKUP_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, PartialEq, Eq)]
struct IssueReference {
    reference: String,
    owner: String,
    repository: String,
    number: u64,
}

pub(super) fn github_issue_title_lookup() -> IssueTitleLookup {
    Arc::new(|owner, repository, number| {
        Box::pin(async move {
            let client = tokio::task::spawn_blocking(GitHubClient::get).await??;
            Ok(client.get_issue(&owner, &repository, number).await?.title)
        })
    })
}

pub(super) async fn lookup_issue_titles_with_timeout(
    frontmatter: &Value,
    lookup: IssueTitleLookup,
) -> HashMap<String, String> {
    let deadline = tokio::time::Instant::now() + ISSUE_TITLE_LOOKUP_TIMEOUT;
    lookup_issue_titles_until(frontmatter, lookup, Some(deadline)).await
}

async fn lookup_issue_titles_until(
    frontmatter: &Value,
    lookup: IssueTitleLookup,
    deadline: Option<tokio::time::Instant>,
) -> HashMap<String, String> {
    let references = issue_references(frontmatter);
    stream::iter(references)
        .map(|issue| {
            let lookup = Arc::clone(&lookup);
            async move {
                let IssueReference {
                    reference,
                    owner,
                    repository,
                    number,
                } = issue;
                let title_result = match deadline {
                    Some(deadline) => {
                        tokio::time::timeout_at(deadline, lookup(owner, repository, number))
                            .await
                            .ok()
                    }
                    None => Some(lookup(owner, repository, number).await),
                };
                let title = match title_result {
                    Some(Ok(title)) => Some(title),
                    Some(Err(error)) => {
                        tracing::debug!(issue = %reference, %error, "failed to load related issue title");
                        None
                    }
                    None => {
                        tracing::debug!(issue = %reference, "timed out loading related issue title");
                        None
                    }
                };
                (reference, title)
            }
        })
        .buffer_unordered(4)
        .filter_map(|(reference, title)| async move { title.map(|title| (reference, title)) })
        .collect()
        .await
}

fn issue_references(frontmatter: &Value) -> Vec<IssueReference> {
    let Some(fields) = frontmatter.as_object() else {
        return Vec::new();
    };

    let parent = fields.get("parentIssue").and_then(Value::as_str);
    let sub_issues = fields.get("subIssues").and_then(Value::as_array);
    let references = parent
        .into_iter()
        .chain(sub_issues.into_iter().flatten().filter_map(Value::as_str));

    let mut unique = HashMap::new();
    for reference in references {
        if let Some(parsed) = parse_issue_reference(reference) {
            unique.entry(parsed.reference.clone()).or_insert(parsed);
        }
    }
    unique.into_values().collect()
}

fn parse_issue_reference(reference: &str) -> Option<IssueReference> {
    let (repository, number) = reference.split_once('#')?;
    let (owner, repository) = repository.split_once('/')?;
    if owner.is_empty()
        || repository.is_empty()
        || repository.contains('/')
        || number.is_empty()
        || number.contains('#')
    {
        return None;
    }
    let number = number.parse::<u64>().ok().filter(|number| *number > 0)?;

    Some(IssueReference {
        reference: reference.to_owned(),
        owner: owner.to_owned(),
        repository: repository.to_owned(),
        number,
    })
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use rstest::rstest;

    use super::*;

    #[tokio::test]
    async fn looks_up_valid_issue_references_and_omits_unavailable_titles() {
        let lookup_count = Arc::new(AtomicUsize::new(0));
        let lookup = Arc::clone(&lookup_count);
        let issue_title_lookup: IssueTitleLookup = Arc::new(move |owner, repository, number| {
            let lookup = Arc::clone(&lookup);
            Box::pin(async move {
                lookup.fetch_add(1, Ordering::Relaxed);
                match (owner.as_str(), repository.as_str(), number) {
                    ("sample-owner", "sample-repo", 42) => Ok("Sample parent".to_owned()),
                    ("sample-owner", "sample-repo", 43) => {
                        anyhow::bail!("issue title unavailable")
                    }
                    _ => anyhow::bail!("unexpected issue reference"),
                }
            })
        });

        let titles = lookup_issue_titles_until(
            &serde_json::json!({
                "parentIssue": "sample-owner/sample-repo#42",
                "subIssues": ["sample-owner/sample-repo#43", "malformed reference"],
            }),
            issue_title_lookup,
            None,
        )
        .await;

        assert_eq!(
            (titles, lookup_count.load(Ordering::Relaxed)),
            (
                HashMap::from([(
                    "sample-owner/sample-repo#42".to_owned(),
                    "Sample parent".to_owned(),
                )]),
                2,
            ),
        );
    }

    #[tokio::test]
    async fn keeps_titles_that_resolve_before_another_lookup_times_out() {
        let issue_title_lookup: IssueTitleLookup = Arc::new(|_, _, number| {
            Box::pin(async move {
                match number {
                    42 => Ok("Sample parent".to_owned()),
                    43 => std::future::pending::<anyhow::Result<String>>().await,
                    _ => anyhow::bail!("unexpected issue reference"),
                }
            })
        });

        let titles = lookup_issue_titles_with_timeout(
            &serde_json::json!({
                "parentIssue": "sample-owner/sample-repo#42",
                "subIssues": ["sample-owner/sample-repo#43"],
            }),
            issue_title_lookup,
        )
        .await;

        assert_eq!(
            titles,
            HashMap::from([(
                "sample-owner/sample-repo#42".to_owned(),
                "Sample parent".to_owned(),
            )]),
        );
    }

    #[rstest]
    #[case::valid(
        "sample-owner/sample-repo#42",
        Some(IssueReference {
            reference: "sample-owner/sample-repo#42".to_owned(),
            owner: "sample-owner".to_owned(),
            repository: "sample-repo".to_owned(),
            number: 42,
        }),
    )]
    #[case::zero("sample-owner/sample-repo#0", None)]
    #[case::non_numeric("sample-owner/sample-repo#nope", None)]
    #[case::missing_repository("sample-owner#42", None)]
    #[case::extra_path_segment("sample-owner/extra/sample-repo#42", None)]
    fn parses_issue_references(#[case] reference: &str, #[case] expected: Option<IssueReference>) {
        assert_eq!(parse_issue_reference(reference), expected,);
    }
}
