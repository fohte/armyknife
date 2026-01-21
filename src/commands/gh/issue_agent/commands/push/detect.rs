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

        if local_comment.body == remote_comment.body {
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

/// Check if remote has changed since the last pull.
/// Returns Ok(()) if no conflict, Err with message if changed.
pub(super) fn check_remote_unchanged(
    local_updated_at: &str,
    remote_updated_at: &str,
    force: bool,
) -> Result<(), String> {
    if force || local_updated_at == remote_updated_at {
        Ok(())
    } else {
        Err(format!(
            "Remote has changed since pull. Local: {}, Remote: {}",
            local_updated_at, remote_updated_at
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::gh::issue_agent::models::IssueMetadata;
    use crate::commands::gh::issue_agent::testing::factories;
    use rstest::rstest;

    mod check_remote_unchanged_tests {
        use super::*;

        #[rstest]
        #[case::same_timestamp("2024-01-01T00:00:00Z", "2024-01-01T00:00:00Z", false)]
        #[case::force_with_different("2024-01-01T00:00:00Z", "2024-01-02T00:00:00Z", true)]
        #[case::force_with_same("2024-01-01T00:00:00Z", "2024-01-01T00:00:00Z", true)]
        fn test_ok(#[case] local: &str, #[case] remote: &str, #[case] force: bool) {
            assert!(check_remote_unchanged(local, remote, force).is_ok());
        }

        #[rstest]
        #[case::different_timestamp("2024-01-01T00:00:00Z", "2024-01-02T00:00:00Z", false)]
        #[case::local_newer("2024-01-02T00:00:00Z", "2024-01-01T00:00:00Z", false)]
        fn test_err(#[case] local: &str, #[case] remote: &str, #[case] force: bool) {
            let result = check_remote_unchanged(local, remote, force);
            assert!(result.is_err());
            assert_eq!(
                result.unwrap_err(),
                format!(
                    "Remote has changed since pull. Local: {}, Remote: {}",
                    local, remote
                )
            );
        }
    }

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
    }
}
