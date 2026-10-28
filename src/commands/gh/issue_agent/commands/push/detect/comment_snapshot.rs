use std::collections::HashMap;

use crate::commands::gh::issue_agent::body_compare::comment_bodies_equal;
use crate::commands::gh::issue_agent::storage::CommentSnapshot;

pub(in crate::commands::gh::issue_agent::commands::push) type BaseCommentMap<'a> =
    HashMap<&'a str, &'a CommentSnapshot>;

#[derive(Debug, PartialEq, Eq)]
pub(in crate::commands::gh::issue_agent::commands::push) enum CommentBodyComparison {
    Unchanged,
    LocalOnly,
    RemoteOnly,
    Converged,
    Divergent,
}

pub(in crate::commands::gh::issue_agent::commands::push) fn index_base_comments(
    base_comments: Option<&[CommentSnapshot]>,
) -> Option<BaseCommentMap<'_>> {
    base_comments.map(|comments| comments.iter().map(|c| (c.id.as_str(), c)).collect())
}

pub(in crate::commands::gh::issue_agent::commands::push) fn compare_comment_bodies(
    local: &str,
    base: &str,
    remote: &str,
) -> CommentBodyComparison {
    let local_changed = !comment_bodies_equal(local, base);
    let remote_changed = !comment_bodies_equal(remote, base);

    match (local_changed, remote_changed) {
        (false, false) => CommentBodyComparison::Unchanged,
        (true, false) => CommentBodyComparison::LocalOnly,
        (false, true) => CommentBodyComparison::RemoteOnly,
        (true, true) if comment_bodies_equal(local, remote) => CommentBodyComparison::Converged,
        (true, true) => CommentBodyComparison::Divergent,
    }
}
