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

    classify_cached(&path, &issue_agent_root, &pr_review_root)
}

fn canonicalize_or_original(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
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
            let issue_metadata = if components.last() == Some(&"issue.md") {
                path.parent().and_then(read_issue_metadata)
            } else {
                None
            };
            let title = frontmatter
                .as_ref()
                .and_then(|value| value.get("title"))
                .and_then(serde_yaml::Value::as_str)
                .map(str::to_owned)
                .or_else(|| {
                    issue_metadata
                        .as_ref()
                        .map(|metadata| metadata.title.clone())
                });

            if components.last() == Some(&"issue.md") {
                let issue_number = frontmatter
                    .as_ref()
                    .and_then(|value| value.get("readonly"))
                    .and_then(|value| value.get("number"))
                    .and_then(serde_yaml::Value::as_i64)
                    .or_else(|| issue_metadata.as_ref().map(|metadata| metadata.number));
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

    (ReviewDocumentKind::Draft, ReviewDocumentTarget::default())
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

#[cfg(test)]
mod tests {
    use std::path::Path;

    use indoc::indoc;
    use rstest::{fixture, rstest};
    use tempfile::TempDir;

    use super::{ReviewDocumentKind, ReviewDocumentRoots, ReviewDocumentTarget, classify};

    #[fixture]
    fn review_document_roots() -> (TempDir, ReviewDocumentRoots) {
        let temp_dir = TempDir::new().expect("fixture directory should be created");
        let roots = ReviewDocumentRoots {
            pr_draft: temp_dir.path().join("pr-body-draft"),
            issue_agent: temp_dir.path().join("gh-issue-agent"),
            pr_review: temp_dir.path().join("gh-pr-review"),
        };
        (temp_dir, roots)
    }

    fn write_document(path: &Path, content: &str) {
        std::fs::create_dir_all(path.parent().expect("fixture file should have a parent"))
            .expect("fixture directory should be created");
        std::fs::write(path, content).expect("fixture document should be written");
    }

    #[cfg(unix)]
    #[rstest]
    fn classifies_pr_drafts_when_the_root_is_a_symlink(
        review_document_roots: (TempDir, ReviewDocumentRoots),
    ) {
        use std::os::unix::fs::symlink;

        let (temp_dir, mut roots) = review_document_roots;
        let actual_root = temp_dir.path().join("actual-pr-drafts");
        std::fs::create_dir_all(&actual_root).expect("actual draft root should be created");
        let linked_root = temp_dir.path().join("pr-body-draft");
        symlink(&actual_root, &linked_root).expect("draft root symlink should be created");
        roots.pr_draft = linked_root;
        let path = actual_root.join("demo-owner/example-repo/feature/review-metadata.md");
        write_document(
            &path,
            indoc! {"
                ---
                title: Example change
                ---
                Body.
            "},
        );

        assert_eq!(
            classify(&path, &roots),
            (
                ReviewDocumentKind::PrDraft,
                ReviewDocumentTarget {
                    repository: Some("demo-owner/example-repo".to_owned()),
                    branch: Some("feature/review-metadata".to_owned()),
                    title: Some("Example change".to_owned()),
                    ..ReviewDocumentTarget::default()
                },
            ),
        );
    }

    #[rstest]
    fn classifies_existing_issue_comment_edits(
        review_document_roots: (TempDir, ReviewDocumentRoots),
    ) {
        let (_temp_dir, roots) = review_document_roots;
        let issue_dir = roots.issue_agent.join("demo-owner/example-repo/7319");
        let issue_path = issue_dir.join("issue.md");
        let comment_path = issue_dir.join("comments/001_comment_88600.md");
        write_document(
            &issue_path,
            indoc! {"
                ---
                title: Example issue
                readonly:
                  number: 7319
                ---
                Body.
            "},
        );
        write_document(&comment_path, "Existing comment body.");

        assert_eq!(
            classify(&comment_path, &roots),
            (
                ReviewDocumentKind::IssueComment,
                ReviewDocumentTarget {
                    repository: Some("demo-owner/example-repo".to_owned()),
                    issue_number: Some(7319),
                    title: Some("Example issue".to_owned()),
                    ..ReviewDocumentTarget::default()
                },
            ),
        );
    }

    #[rstest]
    fn reads_issue_targets_from_legacy_metadata_json(
        review_document_roots: (TempDir, ReviewDocumentRoots),
    ) {
        let (_temp_dir, roots) = review_document_roots;
        let issue_dir = roots.issue_agent.join("demo-owner/example-repo/7319");
        let issue_path = issue_dir.join("issue.md");
        write_document(&issue_path, "Legacy issue body.\n");
        write_document(
            &issue_dir.join("metadata.json"),
            indoc! {r#"
                {
                  "number": 7319,
                  "title": "Legacy example issue",
                  "state": "OPEN",
                  "labels": [],
                  "assignees": [],
                  "milestone": null,
                  "author": "example-author",
                  "createdAt": "2026-01-01T00:00:00Z",
                  "updatedAt": "2026-01-02T00:00:00Z"
                }
            "#},
        );

        assert_eq!(
            classify(&issue_path, &roots),
            (
                ReviewDocumentKind::IssueBody,
                ReviewDocumentTarget {
                    repository: Some("demo-owner/example-repo".to_owned()),
                    issue_number: Some(7319),
                    title: Some("Legacy example issue".to_owned()),
                    ..ReviewDocumentTarget::default()
                },
            ),
        );
    }
}
