use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::commands::ai::pr_draft::{draft_dir, parse_draft_path_under};
use crate::commands::gh::issue_agent::models::IssueMetadata;
use crate::commands::gh::issue_agent::storage::IssueStorage;
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
    pub(super) path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) repository: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) branch: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) issue_number: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) pull_request_number: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) pull_request_lookup_unavailable: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) thread_count: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ReviewDocumentSummary {
    pub(super) id: String,
    pub(super) kind: ReviewDocumentKind,
    pub(super) target: ReviewDocumentTarget,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) started_at: Option<u64>,
    pub(super) is_editor: bool,
    pub(super) can_save: bool,
    pub(super) can_finish: bool,
}

pub(super) fn summarize(
    roots: &ReviewDocumentRoots,
    marker: &ReviewDocumentMarker,
    session_cwd: &Path,
    is_editor: bool,
    can_save: bool,
    can_finish: bool,
) -> ReviewDocumentSummary {
    let (kind, target) = classify(&marker.path, roots, session_cwd);
    ReviewDocumentSummary {
        id: marker.id.clone(),
        kind,
        target,
        started_at: (marker.started_at / 1_000_000)
            .try_into()
            .ok()
            .filter(|started_at| *started_at > 0),
        is_editor,
        can_save,
        can_finish,
    }
}

pub(super) fn classify(
    path: &Path,
    roots: &ReviewDocumentRoots,
    session_cwd: &Path,
) -> (ReviewDocumentKind, ReviewDocumentTarget) {
    let path = canonicalize_or_original(path);
    let pr_draft_root = canonicalize_or_original(&roots.pr_draft);
    let issue_agent_root = canonicalize_or_original(&roots.issue_agent);
    let pr_review_root = canonicalize_or_original(&roots.pr_review);

    if let Some((owner, repo, branch)) = parse_draft_path_under(&path, &pr_draft_root) {
        let frontmatter = read_frontmatter(&path);
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

    classify_cached(&path, &issue_agent_root, &pr_review_root, session_cwd)
}

fn canonicalize_or_original(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

fn classify_cached(
    path: &Path,
    issue_agent_dir: &Path,
    pr_review_dir: &Path,
    session_cwd: &Path,
) -> (ReviewDocumentKind, ReviewDocumentTarget) {
    if let Ok(relative) = path.strip_prefix(issue_agent_dir) {
        let components = relative
            .components()
            .filter_map(|component| component.as_os_str().to_str())
            .collect::<Vec<_>>();
        if components.len() >= 4 {
            let repository = Some(format!("{}/{}", components[0], components[1]));
            if components.last() == Some(&"issue.md") {
                let frontmatter = read_frontmatter(path);
                let title = frontmatter
                    .as_ref()
                    .and_then(|value| value.get("title"))
                    .and_then(serde_yaml::Value::as_str)
                    .map(str::to_owned);
                let issue_number = frontmatter
                    .as_ref()
                    .and_then(|value| value.get("readonly"))
                    .and_then(|value| value.get("number"))
                    .and_then(serde_yaml::Value::as_i64);
                let issue_metadata =
                    if components[2] != "new" && (title.is_none() || issue_number.is_none()) {
                        path.parent().and_then(read_issue_metadata)
                    } else {
                        None
                    };
                return (
                    if components[2] == "new" {
                        ReviewDocumentKind::Draft
                    } else {
                        ReviewDocumentKind::IssueBody
                    },
                    ReviewDocumentTarget {
                        repository,
                        issue_number: issue_number
                            .or_else(|| issue_metadata.as_ref().map(|metadata| metadata.number)),
                        title: title.or_else(|| {
                            issue_metadata
                                .as_ref()
                                .map(|metadata| metadata.title.clone())
                        }),
                        ..ReviewDocumentTarget::default()
                    },
                );
            }

            if components.len() >= 5
                && components[components.len() - 2] == "comments"
                && components.last().is_some_and(|name| name.ends_with(".md"))
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

    (
        ReviewDocumentKind::Draft,
        ReviewDocumentTarget {
            path: relative_draft_path(path, session_cwd),
            ..ReviewDocumentTarget::default()
        },
    )
}

fn relative_draft_path(path: &Path, session_cwd: &Path) -> Option<String> {
    let session_cwd = canonicalize_or_original(session_cwd);
    path.strip_prefix(session_cwd)
        .ok()
        .filter(|relative| !relative.as_os_str().is_empty())
        .or_else(|| path.file_name().map(Path::new))
        .and_then(|relative| relative.to_str())
        .map(str::to_owned)
}

fn issue_title_for_comment(path: &Path) -> Option<String> {
    let issue_dir = path.parent()?.parent()?;
    let issue_title = issue_dir.join("issue.md");
    read_frontmatter(&issue_title)
        .and_then(|frontmatter| {
            frontmatter
                .get("title")
                .and_then(serde_yaml::Value::as_str)
                .map(str::to_owned)
        })
        .or_else(|| read_issue_metadata(issue_dir).map(|metadata| metadata.title))
}

fn read_issue_metadata(issue_dir: &Path) -> Option<IssueMetadata> {
    IssueStorage::from_dir(issue_dir).read_metadata().ok()
}

fn read_frontmatter(path: &Path) -> Option<serde_yaml::Value> {
    let content = std::fs::read_to_string(path).ok()?;
    parse_frontmatter(&content)
}

fn parse_frontmatter(content: &str) -> Option<serde_yaml::Value> {
    let (_, yaml, _) = crate::shared::yaml_frontmatter::split_frontmatter(content)?;
    serde_yaml::from_str(yaml).ok()
}
