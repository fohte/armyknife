//! Change detection functions for push command.

use std::collections::{HashMap, HashSet};

use crate::gh::issue_agent::models::{Comment, Issue, IssueMetadata};
use crate::gh::issue_agent::storage::LocalComment;

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
) -> Result<Vec<CommentChange<'a>>, Box<dyn std::error::Error>> {
    let remote_comments_map: HashMap<&str, &Comment> =
        remote_comments.iter().map(|c| (c.id.as_str(), c)).collect();

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
            .ok_or("Comment missing databaseId")?;

        changes.push(CommentChange::Updated {
            filename: &local_comment.filename,
            local_body: &local_comment.body,
            remote_body: &remote_comment.body,
            database_id,
            author,
            current_user,
        });
    }

    Ok(changes)
}

/// Check if the user can edit a comment.
/// Returns Ok(()) if allowed, Err with message if not.
pub(super) fn check_can_edit_comment(
    comment_author: &str,
    current_user: &str,
    edit_others: bool,
    filename: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    if comment_author == current_user || edit_others {
        Ok(())
    } else {
        Err(format!(
            "Cannot edit other user's comment: {} (author: {}). Use --edit-others to allow.",
            filename, comment_author
        )
        .into())
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
