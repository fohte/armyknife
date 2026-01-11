use super::comment::Comment;
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewThread {
    pub is_resolved: bool,
    pub comments: CommentsNode,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CommentsNode {
    pub nodes: Vec<Comment>,
}

impl ReviewThread {
    /// Get the root comment (the one without replyTo)
    pub fn root_comment(&self) -> Option<&Comment> {
        self.comments.nodes.iter().find(|c| c.reply_to.is_none())
    }

    /// Get all replies (comments that are not the root comment).
    /// This includes nested replies (replies to replies) since GitHub's API
    /// allows reply chains where replyTo points to any previous comment.
    pub fn replies(&self) -> Vec<&Comment> {
        self.comments
            .nodes
            .iter()
            .filter(|c| c.reply_to.is_some())
            .collect()
    }

    /// Count unresolved threads in a list
    pub fn count_unresolved(threads: &[&ReviewThread]) -> usize {
        threads.iter().filter(|t| !t.is_resolved).count()
    }
}
