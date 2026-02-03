//! Change detection functions for push command.

use std::collections::{HashMap, HashSet};

use crate::commands::gh::issue_agent::models::{Comment, Issue, IssueMetadata};
use crate::commands::gh::issue_agent::storage::LocalComment;

use super::changeset::{BodyChange, CommentChange, LabelChange, TitleChange};

pub(super) fn detect_body_change<'a>(
    local_body: &'a str,
    remote_issue: &'a Issue,
) -> Option<BodyChange<'a>> {
    let remote_body = remote_issue.body.as_deref().unwrap_or("");
    if local_body != remote_body {
        Some(BodyChange {
            local: local_body,
            remote: remote_body,
        })
    } else {
        None
    }
}

pub(super) fn detect_title_change<'a>(
    local_metadata: &'a IssueMetadata,
    remote_issue: &'a Issue,
) -> Option<TitleChange<'a>> {
    if local_metadata.title != remote_issue.title {
        Some(TitleChange {
            local: &local_metadata.title,
            remote: &remote_issue.title,
        })
    } else {
        None
    }
}

pub(super) fn detect_label_change(
    local_metadata: &IssueMetadata,
    remote_issue: &Issue,
) -> Option<LabelChange> {
    let remote_labels: HashSet<&str> = remote_issue
        .labels
        .iter()
        .map(|l| l.name.as_str())
        .collect();
    let local_labels: HashSet<&str> = local_metadata.labels.iter().map(|s| s.as_str()).collect();

    if remote_labels == local_labels {
        return None;
    }

    // Compute to_add and to_remove as owned Strings
    let to_remove: Vec<String> = remote_labels
        .difference(&local_labels)
        .map(|s| s.to_string())
        .collect();
    let to_add: Vec<String> = local_labels
        .difference(&remote_labels)
        .map(|s| s.to_string())
        .collect();

    let mut remote_sorted: Vec<String> = remote_labels.into_iter().map(|s| s.to_string()).collect();
    remote_sorted.sort();
    let mut local_sorted: Vec<String> = local_labels.into_iter().map(|s| s.to_string()).collect();
    local_sorted.sort();

    Some(LabelChange {
        to_add,
        to_remove,
        local_sorted,
        remote_sorted,
    })
}

pub(super) fn detect_comment_changes<'a>(
    local_comments: &'a [LocalComment],
    remote_comments: &'a [Comment],
    current_user: &'a str,
    edit_others: bool,
    allow_delete: bool,
) -> anyhow::Result<Vec<CommentChange<'a>>> {
    let remote_comments_map: HashMap<&str, &Comment> =
        remote_comments.iter().map(|c| (c.id.as_str(), c)).collect();

    // Build a set of local comment IDs (excluding new comments)
    let local_comment_ids: HashSet<&str> = local_comments
        .iter()
        .filter_map(|c| c.metadata.id.as_deref())
        .collect();

    let mut changes = Vec::new();

    for local_comment in local_comments {
        if local_comment.is_new() {
            changes.push(CommentChange::New {
                filename: &local_comment.filename,
                body: &local_comment.body,
            });
            continue;
        }

        let Some(comment_id) = &local_comment.metadata.id else {
            continue;
        };
        let Some(remote_comment) = remote_comments_map.get(comment_id.as_str()) else {
            continue;
        };

        // Compare with whitespace normalized to handle inconsistencies
        // between GitHub API responses and local file parsing
        if local_comment.body.trim() == remote_comment.body.trim() {
            continue;
        }

        let author = local_comment
            .metadata
            .author
            .as_deref()
            .unwrap_or("unknown");

        // Check if editing other user's comment
        check_can_edit_comment(author, current_user, edit_others, &local_comment.filename)?;

        let database_id = local_comment
            .metadata
            .database_id
            .ok_or_else(|| anyhow::anyhow!("Comment missing databaseId"))?;

        changes.push(CommentChange::Updated {
            filename: &local_comment.filename,
            local_body: &local_comment.body,
            remote_body: &remote_comment.body,
            database_id,
            author,
            current_user,
        });
    }

    // Detect deleted comments (remote comments that don't exist locally)
    for remote_comment in remote_comments {
        if !local_comment_ids.contains(remote_comment.id.as_str()) {
            let author = remote_comment
                .author
                .as_ref()
                .map(|a| a.login.as_str())
                .unwrap_or("unknown");

            // Check if deleting other user's comment requires --allow-delete
            check_can_delete_comment(
                author,
                current_user,
                allow_delete,
                remote_comment.database_id,
            )?;

            changes.push(CommentChange::Deleted {
                database_id: remote_comment.database_id,
                body: &remote_comment.body,
                author,
            });
        }
    }

    Ok(changes)
}

/// Check if the user can delete a comment.
/// Returns Ok(()) if allowed, Err with message if not.
pub(super) fn check_can_delete_comment(
    comment_author: &str,
    current_user: &str,
    allow_delete: bool,
    database_id: i64,
) -> anyhow::Result<()> {
    if allow_delete {
        Ok(())
    } else if comment_author == current_user {
        anyhow::bail!(
            "Cannot delete comment (database_id: {}). Use --allow-delete to allow.",
            database_id
        )
    } else {
        anyhow::bail!(
            "Cannot delete other user's comment (database_id: {}, author: {}). Use --allow-delete to allow.",
            database_id,
            comment_author
        )
    }
}

/// Check if the user can edit a comment.
/// Returns Ok(()) if allowed, Err with message if not.
pub(super) fn check_can_edit_comment(
    comment_author: &str,
    current_user: &str,
    edit_others: bool,
    filename: &str,
) -> anyhow::Result<()> {
    if comment_author == current_user || edit_others {
        Ok(())
    } else {
        anyhow::bail!(
            "Cannot edit other user's comment: {} (author: {}). Use --edit-others to allow.",
            filename,
            comment_author
        )
    }
}

/// Represents a field-level conflict between local and remote state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Conflict {
    /// Issue body was edited both locally and remotely.
    Body {
        local_timestamp: Option<String>,
        remote_timestamp: Option<String>,
    },
    /// Issue title was edited both locally and remotely.
    Title {
        local_timestamp: Option<String>,
        remote_timestamp: Option<String>,
    },
    /// Comment was edited both locally and remotely.
    Comment {
        database_id: i64,
        local_timestamp: String,
        remote_timestamp: String,
    },
}

impl std::fmt::Display for Conflict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Conflict::Body {
                local_timestamp,
                remote_timestamp,
            } => write!(
                f,
                "Issue body: local={}, remote={}",
                local_timestamp.as_deref().unwrap_or("(never edited)"),
                remote_timestamp.as_deref().unwrap_or("(never edited)")
            ),
            Conflict::Title {
                local_timestamp,
                remote_timestamp,
            } => write!(
                f,
                "Issue title: local={}, remote={}",
                local_timestamp.as_deref().unwrap_or("(never edited)"),
                remote_timestamp.as_deref().unwrap_or("(never edited)")
            ),
            Conflict::Comment {
                database_id,
                local_timestamp,
                remote_timestamp,
            } => write!(
                f,
                "Comment (database_id={}): local={}, remote={}",
                database_id, local_timestamp, remote_timestamp
            ),
        }
    }
}

/// Input data for conflict detection.
pub(crate) struct ConflictCheckInput<'a> {
    /// Local metadata from frontmatter.
    pub local_metadata: &'a IssueMetadata,
    /// Local issue body.
    pub local_body: &'a str,
    /// Local comments.
    pub local_comments: &'a [LocalComment],
    /// Remote issue state.
    pub remote_issue: &'a Issue,
    /// Remote comments.
    pub remote_comments: &'a [Comment],
}

/// Check for conflicts between local and remote state.
///
/// Returns a list of conflicts found. Empty list means no conflicts.
///
/// The logic is:
/// - For body/title: conflict if local differs from remote AND remote updatedAt
///   has changed since pull (indicating someone else edited the issue)
/// - For comments: conflict if local comment body differs from remote AND remote updatedAt
///   has changed since pull
pub(crate) fn check_conflicts(input: &ConflictCheckInput<'_>) -> Vec<Conflict> {
    let mut conflicts = Vec::new();

    // Check if the issue was updated remotely since we pulled
    let local_updated_at = &input.local_metadata.updated_at;
    let remote_updated_at = input.remote_issue.updated_at.to_rfc3339();
    let issue_updated_remotely = local_updated_at != &remote_updated_at;

    // Check body conflict
    let remote_body = input.remote_issue.body.as_deref().unwrap_or("");
    let local_body_changed = input.local_body != remote_body;

    if local_body_changed && issue_updated_remotely {
        conflicts.push(Conflict::Body {
            local_timestamp: Some(local_updated_at.clone()),
            remote_timestamp: Some(remote_updated_at.clone()),
        });
    }

    // Check title conflict
    let local_title_changed = input.local_metadata.title != input.remote_issue.title;

    if local_title_changed && issue_updated_remotely {
        conflicts.push(Conflict::Title {
            local_timestamp: Some(local_updated_at.clone()),
            remote_timestamp: Some(remote_updated_at.clone()),
        });
    }

    // Check comment conflicts
    let remote_comments_map: HashMap<&str, &Comment> = input
        .remote_comments
        .iter()
        .map(|c| (c.id.as_str(), c))
        .collect();

    for local_comment in input.local_comments {
        // Skip new comments - they can't have conflicts
        if local_comment.is_new() {
            continue;
        }

        let Some(comment_id) = &local_comment.metadata.id else {
            continue;
        };
        let Some(remote_comment) = remote_comments_map.get(comment_id.as_str()) else {
            continue;
        };

        // Check if local comment was modified
        if local_comment.body.trim() == remote_comment.body.trim() {
            continue;
        }

        // Local comment was modified - check if remote was also edited
        let local_updated_at = local_comment.metadata.updated_at.as_deref();
        let remote_updated_at = remote_comment.updated_at.to_rfc3339();

        // If local_updated_at is None, this is a legacy comment file without
        // timestamp tracking. We skip conflict detection for backward compatibility.
        if let Some(local_ts) = local_updated_at.filter(|ts| *ts != remote_updated_at) {
            conflicts.push(Conflict::Comment {
                database_id: remote_comment.database_id,
                local_timestamp: local_ts.to_string(),
                remote_timestamp: remote_updated_at,
            });
        }
    }

    conflicts
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::gh::issue_agent::models::IssueMetadata;
    use crate::commands::gh::issue_agent::testing::factories;
    use rstest::rstest;

    mod check_can_edit_comment_tests {
        use super::*;

        #[rstest]
        #[case::own_comment("alice", "alice", false)]
        #[case::other_with_flag("bob", "alice", true)]
        #[case::own_with_flag("alice", "alice", true)]
        fn test_allowed(
            #[case] author: &str,
            #[case] current_user: &str,
            #[case] edit_others: bool,
        ) {
            assert!(check_can_edit_comment(author, current_user, edit_others, "001.md").is_ok());
        }

        #[rstest]
        #[case::other_without_flag("bob", "alice", false)]
        #[case::unknown_author("unknown", "alice", false)]
        fn test_denied(
            #[case] author: &str,
            #[case] current_user: &str,
            #[case] edit_others: bool,
        ) {
            let result = check_can_edit_comment(author, current_user, edit_others, "001.md");
            assert!(result.is_err());
            assert_eq!(
                result.unwrap_err().to_string(),
                format!(
                    "Cannot edit other user's comment: 001.md (author: {}). Use --edit-others to allow.",
                    author
                )
            );
        }
    }

    mod detect_label_change_tests {
        use super::*;

        fn metadata_with_labels(labels: &[&str]) -> IssueMetadata {
            IssueMetadata {
                number: 1,
                title: "Test".to_string(),
                state: "OPEN".to_string(),
                labels: labels.iter().map(|s| s.to_string()).collect(),
                assignees: vec![],
                milestone: None,
                author: "testuser".to_string(),
                created_at: "2024-01-01T00:00:00Z".to_string(),
                updated_at: "2024-01-02T00:00:00Z".to_string(),
            }
        }

        fn issue_with_labels(labels: &[&str]) -> Issue {
            factories::issue_with(|i| {
                i.labels = factories::labels(labels);
            })
        }

        #[rstest]
        #[case::no_changes(vec!["bug"], vec!["bug"])]
        #[case::both_empty(vec![], vec![])]
        fn test_returns_none_when_no_changes(#[case] local: Vec<&str>, #[case] remote: Vec<&str>) {
            let metadata = metadata_with_labels(&local);
            let issue = issue_with_labels(&remote);
            assert!(detect_label_change(&metadata, &issue).is_none());
        }

        #[rstest]
        #[case::add_one(vec!["bug", "new"], vec!["bug"], vec![], vec!["new"])]
        #[case::remove_one(vec!["bug"], vec!["bug", "old"], vec!["old"], vec![])]
        #[case::add_and_remove(vec!["new"], vec!["old"], vec!["old"], vec!["new"])]
        #[case::empty_local(vec![], vec!["a", "b"], vec!["a", "b"], vec![])]
        #[case::empty_remote(vec!["a", "b"], vec![], vec![], vec!["a", "b"])]
        fn test_detects_label_changes(
            #[case] local: Vec<&str>,
            #[case] remote: Vec<&str>,
            #[case] expected_remove: Vec<&str>,
            #[case] expected_add: Vec<&str>,
        ) {
            let metadata = metadata_with_labels(&local);
            let issue = issue_with_labels(&remote);

            let change = detect_label_change(&metadata, &issue).expect("Expected label changes");

            let mut to_remove = change.to_remove.clone();
            to_remove.sort();
            let mut to_add = change.to_add.clone();
            to_add.sort();
            let mut expected_remove: Vec<String> =
                expected_remove.iter().map(|s| s.to_string()).collect();
            let mut expected_add: Vec<String> =
                expected_add.iter().map(|s| s.to_string()).collect();
            expected_remove.sort();
            expected_add.sort();

            assert_eq!(to_remove, expected_remove);
            assert_eq!(to_add, expected_add);
        }
    }

    mod check_can_delete_comment_tests {
        use super::*;

        #[rstest]
        #[case::allow_delete_own("alice", "alice", true)]
        #[case::allow_delete_other("bob", "alice", true)]
        fn test_allowed_with_flag(
            #[case] author: &str,
            #[case] current_user: &str,
            #[case] allow_delete: bool,
        ) {
            assert!(check_can_delete_comment(author, current_user, allow_delete, 12345).is_ok());
        }

        #[rstest]
        #[case::deny_own_without_flag("alice", "alice", false)]
        #[case::deny_other_without_flag("bob", "alice", false)]
        fn test_denied_without_flag(
            #[case] author: &str,
            #[case] current_user: &str,
            #[case] allow_delete: bool,
        ) {
            let result = check_can_delete_comment(author, current_user, allow_delete, 12345);
            assert!(result.is_err());
            assert!(result.unwrap_err().to_string().contains("--allow-delete"));
        }
    }

    mod detect_comment_changes_tests {
        use super::*;
        use crate::commands::gh::issue_agent::storage::LocalComment;

        fn make_local_comment(
            id: &str,
            database_id: i64,
            body: &str,
            author: &str,
        ) -> LocalComment {
            factories::local_comment_with(|c| {
                c.filename = format!("001_comment_{}.md", database_id);
                c.body = body.to_string();
                c.metadata.author = Some(author.to_string());
                c.metadata.id = Some(id.to_string());
                c.metadata.database_id = Some(database_id);
            })
        }

        fn make_remote_comment(id: &str, database_id: i64, body: &str, author: &str) -> Comment {
            factories::comment_with(|c| {
                c.id = id.to_string();
                c.database_id = database_id;
                c.body = body.to_string();
                c.author = Some(factories::author(author));
            })
        }

        #[rstest]
        #[case::delete_allowed(vec![], true, 1)]
        #[case::no_delete_when_local_exists(vec![make_local_comment("IC_123", 12345, "body", "alice")], false, 0)]
        fn test_delete_detection(
            #[case] local_comments: Vec<LocalComment>,
            #[case] allow_delete: bool,
            #[case] expected_changes: usize,
        ) {
            let remote_comments = vec![make_remote_comment("IC_123", 12345, "body", "alice")];

            let result = detect_comment_changes(
                &local_comments,
                &remote_comments,
                "alice",
                false,
                allow_delete,
            );

            let changes = result.expect("Expected Ok result");
            assert_eq!(changes.len(), expected_changes);
            if expected_changes > 0 {
                assert!(matches!(
                    &changes[0],
                    CommentChange::Deleted { database_id, .. } if *database_id == 12345
                ));
            }
        }

        #[rstest]
        fn test_delete_without_allow_delete_fails() {
            let local_comments: Vec<LocalComment> = vec![];
            let remote_comments = vec![make_remote_comment("IC_123", 12345, "body", "alice")];

            let result =
                detect_comment_changes(&local_comments, &remote_comments, "alice", false, false);

            assert!(result.unwrap_err().to_string().contains("--allow-delete"));
        }

        /// Test that comments with whitespace-only differences are not detected as changed.
        /// GitHub API may return body with leading/trailing newlines, but local file parsing
        /// normalizes them via lines().join("\n").
        #[rstest]
        #[case::trailing_newline("body", "body\n")]
        #[case::trailing_multiple_newlines("body", "body\n\n")]
        #[case::trailing_spaces("body", "body  ")]
        #[case::leading_newline("body", "\nbody")]
        #[case::leading_multiple_newlines("body", "\n\nbody")]
        fn test_no_change_with_whitespace_difference(
            #[case] local_body: &str,
            #[case] remote_body: &str,
        ) {
            let local_comments = vec![make_local_comment("IC_123", 12345, local_body, "alice")];
            let remote_comments = vec![make_remote_comment("IC_123", 12345, remote_body, "alice")];

            let result =
                detect_comment_changes(&local_comments, &remote_comments, "alice", false, false);

            let changes = result.expect("Expected Ok result");
            assert!(
                changes.is_empty(),
                "Expected no changes, but got: {:?}",
                changes
            );
        }

        /// Test that actual content changes are still detected.
        #[rstest]
        fn test_actual_change_is_detected() {
            let local_comments = vec![make_local_comment(
                "IC_123",
                12345,
                "modified body",
                "alice",
            )];
            let remote_comments = vec![make_remote_comment(
                "IC_123",
                12345,
                "original body",
                "alice",
            )];

            let result =
                detect_comment_changes(&local_comments, &remote_comments, "alice", false, false);

            let changes = result.expect("Expected Ok result");
            assert_eq!(changes.len(), 1);
            assert!(matches!(&changes[0], CommentChange::Updated { .. }));
        }
    }
}
