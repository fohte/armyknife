use crate::commands::gh::issue_agent::body_compare::comment_bodies_equal;
use crate::commands::gh::issue_agent::models::Comment;
use crate::commands::gh::issue_agent::storage::LocalComment;
use chrono::{DateTime, Utc};

use super::super::changeset::CommentChange;

pub(super) fn new_comment_change<'a>(
    local_comment: &'a LocalComment,
    remote_comments: &[Comment],
    current_user: &str,
    retry_started_at: Option<DateTime<Utc>>,
) -> Option<CommentChange<'a>> {
    if was_posted_since_attempt(
        &local_comment.body,
        remote_comments,
        current_user,
        retry_started_at,
    ) {
        None
    } else {
        Some(CommentChange::New {
            filename: &local_comment.filename,
            body: &local_comment.body,
        })
    }
}

fn was_posted_since_attempt(
    local_body: &str,
    remote_comments: &[Comment],
    current_user: &str,
    retry_started_at: Option<DateTime<Utc>>,
) -> bool {
    retry_started_at.is_some_and(|started_at| {
        remote_comments.iter().any(|remote_comment| {
            remote_comment.created_at >= started_at
                && remote_comment
                    .author
                    .as_ref()
                    .is_some_and(|author| author.login == current_user)
                && comment_bodies_equal(local_body, &remote_comment.body)
        })
    })
}
