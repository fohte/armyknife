use std::collections::HashSet;

use crate::commands::agent::tui::progress::marks::stage_marks;
use crate::commands::agent::tui::progress::{FetchedDelegate, ProgressMark};
use crate::commands::agent::tui::progress_fetch::PullRequestListItem;
use crate::infra::github::PrState;
use crate::shared::config::AgentConfig;

use super::ImplementationPlan;

pub(in crate::commands::agent::tui) fn pull_request_list_items(
    delegates: &[FetchedDelegate],
    plan: Option<&ImplementationPlan>,
    agent_config: &AgentConfig,
) -> Vec<PullRequestListItem> {
    let mut items = Vec::new();
    let mut matched_delegate_session_ids = HashSet::new();

    if let Some(plan) = plan {
        for item in plan.batches.iter().flat_map(|batch| &batch.items) {
            let delegate = item.delegate_session_id.as_deref().and_then(|id| {
                delegates
                    .iter()
                    .find(|delegate| delegate.session.session_id == id)
            });
            if let Some(delegate) = delegate {
                matched_delegate_session_ids.insert(delegate.session.session_id.clone());
            }

            let progress = item.pull_request.as_ref();
            let link = item.github_link.as_ref();
            let number = progress
                .map(|progress| progress.pull_request.number)
                .or_else(|| link.and_then(|link| link.number));
            let url = progress
                .map(|progress| progress.pull_request.url.clone())
                .or_else(|| link.and_then(|link| link.url.clone()));
            let (Some(number), Some(url)) = (number, url) else {
                continue;
            };
            let repository = pull_request_repository(
                &url,
                number,
                link.and_then(|link| link.owner.as_deref().zip(link.repo.as_deref())),
            );

            items.push(PullRequestListItem {
                number,
                owner: repository.as_ref().map(|(owner, _)| owner.clone()),
                repo: repository.map(|(_, repo)| repo),
                title: progress
                    .map_or_else(|| item.title.clone(), |pr| pr.pull_request.title.clone()),
                url,
                head_sha: progress.and_then(|progress| progress.head_sha.clone()),
                state: progress
                    .map(|progress| progress.pull_request.state.clone())
                    .or_else(|| {
                        link.and_then(|link| link.state.as_deref())
                            .and_then(parse_pr_state)
                    }),
                ci_state: progress.map(|progress| progress.ci_state),
                review_status: delegate
                    .map_or(ProgressMark::NotStarted, |delegate| {
                        stage_marks(&delegate.session, progress, agent_config).diff_review
                    })
                    .label()
                    .to_string(),
            });
        }
    }

    items.extend(
        delegates
            .iter()
            .filter(|delegate| !matched_delegate_session_ids.contains(&delegate.session.session_id))
            .filter_map(|delegate| {
                let progress = delegate.pull_request.as_ref()?;
                let repository = pull_request_repository(
                    &progress.pull_request.url,
                    progress.pull_request.number,
                    None,
                );
                Some(PullRequestListItem {
                    number: progress.pull_request.number,
                    owner: repository.as_ref().map(|(owner, _)| owner.clone()),
                    repo: repository.map(|(_, repo)| repo),
                    title: progress.pull_request.title.clone(),
                    url: progress.pull_request.url.clone(),
                    head_sha: progress.head_sha.clone(),
                    state: Some(progress.pull_request.state.clone()),
                    ci_state: Some(progress.ci_state),
                    review_status: stage_marks(&delegate.session, Some(progress), agent_config)
                        .diff_review
                        .label()
                        .to_string(),
                })
            }),
    );

    let mut seen_urls = HashSet::new();
    items.retain(|item| seen_urls.insert(item.url.clone()));
    items.sort_by_key(|item| match item.state {
        Some(PrState::Open) => 0,
        Some(PrState::Closed | PrState::Merged) => 1,
        None => 2,
    });
    items
}

fn parse_pr_state(state: &str) -> Option<PrState> {
    if state.eq_ignore_ascii_case("open") {
        Some(PrState::Open)
    } else if state.eq_ignore_ascii_case("closed") {
        Some(PrState::Closed)
    } else if state.eq_ignore_ascii_case("merged") {
        Some(PrState::Merged)
    } else {
        None
    }
}

fn pull_request_repository(
    url: &str,
    number: u64,
    linked_repository: Option<(&str, &str)>,
) -> Option<(String, String)> {
    let parsed_url = reqwest::Url::parse(url).ok()?;
    if parsed_url.scheme() != "https" {
        return None;
    }
    let segments = parsed_url
        .path_segments()?
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    let pull_index = segments.iter().rposition(|segment| *segment == "pull")?;
    if pull_index < 2 || segments.get(pull_index + 1)?.parse::<u64>().ok()? != number {
        return None;
    }
    let repository = (
        segments[pull_index - 2].to_string(),
        segments[pull_index - 1].to_string(),
    );
    if linked_repository.is_some_and(|(owner, repo)| owner != repository.0 || repo != repository.1)
    {
        return None;
    }
    Some(repository)
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use crate::commands::agent::tui::progress::FetchedDelegate;
    use crate::commands::agent::tui::progress_batches::{
        ImplementationBatch, ImplementationItem, ImplementationPlan,
    };
    use crate::commands::agent::tui::ui::test_support::create_test_session;
    use crate::infra::github::{PrCiState, PrInfo, PrProgressInfo, PrState};
    use crate::infra::tq::TqGithubLink;
    use crate::shared::config::AgentConfig;

    use crate::commands::agent::tui::progress_fetch::PullRequestListItem;

    use super::pull_request_list_items;

    #[rstest]
    #[case::github_url(
        "https://github.com/example/project/pull/42",
        42,
        Some(("example", "project")),
        Some(("example", "project")),
    )]
    #[case::enterprise_url(
        "https://git.example/ghe/team/project/pull/42",
        42,
        None,
        Some(("team", "project")),
    )]
    #[case::linked_repository_mismatch(
        "https://github.com/example/project/pull/42",
        42,
        Some(("different", "project")),
        None,
    )]
    #[case::url_number_mismatch("https://github.com/example/project/pull/43", 42, None, None)]
    fn resolves_repository_from_the_listed_pull_request(
        #[case] url: &str,
        #[case] number: u64,
        #[case] linked_repository: Option<(&str, &str)>,
        #[case] expected: Option<(&str, &str)>,
    ) {
        let expected = expected.map(|(owner, repo)| (owner.to_string(), repo.to_string()));

        assert_eq!(
            super::pull_request_repository(url, number, linked_repository),
            expected,
        );
    }

    #[test]
    fn lists_open_pull_requests_before_merged_pull_requests() {
        let delegates = vec![
            FetchedDelegate {
                session: create_test_session("merged-session"),
                checklist_item_id: None,
                pull_request: Some(PrProgressInfo {
                    pull_request: PrInfo {
                        number: 2046,
                        title: "Update workspace navigation".to_string(),
                        state: PrState::Merged,
                        url: "https://github.com/example/project/pull/2046".to_string(),
                    },
                    ci_state: PrCiState::Successful,
                    head_sha: None,
                }),
                lookup_unavailable: false,
            },
            FetchedDelegate {
                session: create_test_session("open-session"),
                checklist_item_id: None,
                pull_request: Some(PrProgressInfo {
                    pull_request: PrInfo {
                        number: 2048,
                        title: "Add session pull request links".to_string(),
                        state: PrState::Open,
                        url: "https://github.com/example/project/pull/2048".to_string(),
                    },
                    ci_state: PrCiState::InProgress,
                    head_sha: Some("confirmed-head".to_string()),
                }),
                lookup_unavailable: false,
            },
        ];

        assert_eq!(
            pull_request_list_items(&delegates, None, &AgentConfig::default()),
            vec![
                PullRequestListItem {
                    number: 2048,
                    owner: Some("example".to_string()),
                    repo: Some("project".to_string()),
                    title: "Add session pull request links".to_string(),
                    url: "https://github.com/example/project/pull/2048".to_string(),
                    head_sha: Some("confirmed-head".to_string()),
                    state: Some(PrState::Open),
                    ci_state: Some(PrCiState::InProgress),
                    review_status: "done".to_string(),
                },
                PullRequestListItem {
                    number: 2046,
                    owner: Some("example".to_string()),
                    repo: Some("project".to_string()),
                    title: "Update workspace navigation".to_string(),
                    url: "https://github.com/example/project/pull/2046".to_string(),
                    head_sha: None,
                    state: Some(PrState::Merged),
                    ci_state: Some(PrCiState::Successful),
                    review_status: "done".to_string(),
                },
            ],
        );
    }

    #[test]
    fn uses_tq_link_fields_when_pull_request_progress_is_unavailable() {
        let plan = ImplementationPlan {
            batches: vec![ImplementationBatch {
                id: "batch-id".to_string(),
                title: "Synthetic implementation batch".to_string(),
                items: vec![ImplementationItem {
                    id: "checklist-item-id".to_string(),
                    title: "Add synthetic setting".to_string(),
                    delegate_session_id: None,
                    github_link: Some(TqGithubLink {
                        id: "github-link-id".to_string(),
                        kind: Some("pull_request".to_string()),
                        owner: Some("example".to_string()),
                        repo: Some("project".to_string()),
                        number: Some(2050),
                        title: Some("Add synthetic setting".to_string()),
                        url: Some("https://github.com/example/project/pull/2050".to_string()),
                        state: Some("open".to_string()),
                    }),
                    pull_request: None,
                    lookup_unavailable: true,
                }],
            }],
            unmatched_delegates: Vec::new(),
        };

        assert_eq!(
            pull_request_list_items(&[], Some(&plan), &AgentConfig::default()),
            vec![PullRequestListItem {
                number: 2050,
                owner: Some("example".to_string()),
                repo: Some("project".to_string()),
                title: "Add synthetic setting".to_string(),
                url: "https://github.com/example/project/pull/2050".to_string(),
                head_sha: None,
                state: Some(PrState::Open),
                ci_state: None,
                review_status: "not_started".to_string(),
            }],
        );
    }
}
