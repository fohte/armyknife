use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::commands::ai::pr_draft::{draft_dir, parse_draft_path_under};
use crate::commands::gh::pr_review::count_review_threads;
use crate::shared::cache;
use crate::shared::human_in_the_loop::ReviewDocumentMarker;

#[derive(Debug, Clone)]
pub(super) struct ReviewDocumentRoots {
    pub(super) pr_draft: PathBuf,
    pub(super) issue_agent: PathBuf,
    pub(super) pr_review: PathBuf,
}

impl Default for ReviewDocumentRoots {
    fn default() -> Self {
        Self {
            pr_draft: draft_dir(),
            issue_agent: cache::issue_agent_dir(),
            pr_review: cache::pr_review_dir(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum ReviewDocumentKind {
    PrDraft,
    IssueComment,
    IssueBody,
    ReviewReplies,
    Draft,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ReviewDocumentTarget {
    #[serde(skip_serializing_if = "Option::is_none")]
    repository: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    branch: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    issue_number: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pull_request_number: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    thread_count: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ReviewDocumentSummary {
    pub(super) id: String,
    pub(super) kind: ReviewDocumentKind,
    pub(super) target: ReviewDocumentTarget,
    pub(super) is_editor: bool,
    pub(super) can_save: bool,
    pub(super) can_finish: bool,
}

pub(super) fn summarize(
    roots: &ReviewDocumentRoots,
    marker: &ReviewDocumentMarker,
    is_editor: bool,
    can_save: bool,
    can_finish: bool,
) -> ReviewDocumentSummary {
    let (kind, target) = classify(&marker.path, roots);
    ReviewDocumentSummary {
        id: marker.id.clone(),
        kind,
        target,
        is_editor,
        can_save,
        can_finish,
    }
}

fn classify(
    path: &Path,
    roots: &ReviewDocumentRoots,
) -> (ReviewDocumentKind, ReviewDocumentTarget) {
    if let Some((owner, repo, branch)) = parse_draft_path_under(path, &roots.pr_draft) {
        let frontmatter = read_frontmatter(path);
        return (
            ReviewDocumentKind::PrDraft,
            ReviewDocumentTarget {
                repository: Some(format!("{owner}/{repo}")),
                branch: Some(branch),
                title: frontmatter
                    .as_ref()
                    .and_then(|value| value.get("title"))
                    .and_then(serde_yaml::Value::as_str)
                    .map(str::to_owned),
                ..ReviewDocumentTarget::default()
            },
        );
    }

    classify_cached(path, &roots.issue_agent, &roots.pr_review)
}

fn classify_cached(
    path: &Path,
    issue_agent_dir: &Path,
    pr_review_dir: &Path,
) -> (ReviewDocumentKind, ReviewDocumentTarget) {
    if let Ok(relative) = path.strip_prefix(issue_agent_dir) {
        let components = relative
            .components()
            .filter_map(|component| component.as_os_str().to_str())
            .collect::<Vec<_>>();
        if components.len() >= 4 {
            let repository = Some(format!("{}/{}", components[0], components[1]));
            let frontmatter = read_frontmatter(path);
            let title = frontmatter
                .as_ref()
                .and_then(|value| value.get("title"))
                .and_then(serde_yaml::Value::as_str)
                .map(str::to_owned);

            if components.last() == Some(&"issue.md") {
                let issue_number = frontmatter
                    .as_ref()
                    .and_then(|value| value.get("readonly"))
                    .and_then(|value| value.get("number"))
                    .and_then(serde_yaml::Value::as_i64);
                return (
                    if components[2] == "new" {
                        ReviewDocumentKind::Draft
                    } else {
                        ReviewDocumentKind::IssueBody
                    },
                    ReviewDocumentTarget {
                        repository,
                        issue_number,
                        title,
                        ..ReviewDocumentTarget::default()
                    },
                );
            }

            if components.len() >= 5
                && components[components.len() - 2] == "comments"
                && components
                    .last()
                    .is_some_and(|name| name.starts_with("new_"))
            {
                return (
                    ReviewDocumentKind::IssueComment,
                    ReviewDocumentTarget {
                        repository,
                        issue_number: components[2].parse().ok(),
                        title: issue_title_for_comment(path),
                        ..ReviewDocumentTarget::default()
                    },
                );
            }
        }
    }

    if let Ok(relative) = path.strip_prefix(pr_review_dir) {
        let components = relative
            .components()
            .filter_map(|component| component.as_os_str().to_str())
            .collect::<Vec<_>>();
        if components.len() == 4 && components[3] == "threads.md" {
            let content = std::fs::read_to_string(path).ok();
            let frontmatter = content.as_deref().and_then(parse_frontmatter);
            let repository = frontmatter
                .as_ref()
                .and_then(|value| value.get("repo"))
                .and_then(serde_yaml::Value::as_str)
                .map(str::to_owned)
                .or_else(|| Some(format!("{}/{}", components[0], components[1])));
            let pull_request_number = frontmatter
                .as_ref()
                .and_then(|value| value.get("pr"))
                .and_then(serde_yaml::Value::as_u64)
                .or_else(|| components[2].parse().ok());
            let thread_count = content.as_deref().and_then(count_review_threads);
            return (
                ReviewDocumentKind::ReviewReplies,
                ReviewDocumentTarget {
                    repository,
                    pull_request_number,
                    thread_count,
                    ..ReviewDocumentTarget::default()
                },
            );
        }
    }

    (ReviewDocumentKind::Draft, ReviewDocumentTarget::default())
}

fn issue_title_for_comment(path: &Path) -> Option<String> {
    let issue_path = path.parent()?.parent()?.join("issue.md");
    read_frontmatter(&issue_path)?
        .get("title")
        .cloned()
        .and_then(|value| value.as_str().map(str::to_owned))
}

fn read_frontmatter(path: &Path) -> Option<serde_yaml::Value> {
    let content = std::fs::read_to_string(path).ok()?;
    parse_frontmatter(&content)
}

fn parse_frontmatter(content: &str) -> Option<serde_yaml::Value> {
    let (_, yaml, _) = crate::shared::yaml_frontmatter::split_frontmatter(content)?;
    serde_yaml::from_str(yaml).ok()
}
