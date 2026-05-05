use crate::commands::gh::pr_review::error::PrReviewError;
use crate::commands::gh::pr_review::markdown::ParsedThreadsFile;
use crate::commands::gh::pr_review::models::{PrData, ReviewThread};
use crate::infra::github::GitHubClient;

/// An action to apply to GitHub during push.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplyAction {
    PostReply {
        thread_id: String,
        in_reply_to: i64,
        body: String,
    },
    ResolveThread {
        thread_id: String,
        thread_node_id: String,
    },
}

/// A conflict detected between local and remote state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadConflict {
    pub thread_id: String,
    pub path: String,
    pub reason: String,
}

/// Represents the set of changes to apply from local edits to GitHub.
pub struct ReplyChangeSet {
    pub actions: Vec<ReplyAction>,
    pub conflicts: Vec<ThreadConflict>,
}

impl ReplyChangeSet {
    /// Detect changes between local parsed data and remote state.
    pub fn detect(local: &ParsedThreadsFile, remote: &PrData, pulled_at: &str) -> Self {
        let mut actions = Vec::new();
        let mut conflicts = Vec::new();

        for local_thread in &local.threads {
            let remote_thread = remote
                .threads
                .iter()
                .find(|t| t.id.as_deref() == Some(&local_thread.thread_id));

            let remote_thread = match remote_thread {
                Some(t) => t,
                None => continue,
            };

            // Check for conflicts: new comments since pull
            if has_new_comments_since(remote_thread, pulled_at) {
                conflicts.push(ThreadConflict {
                    thread_id: local_thread.thread_id.clone(),
                    path: local_thread.path.clone(),
                    reason: "New comments added since last pull".to_string(),
                });
            }

            // Detect draft reply
            if let Some(draft) = &local_thread.draft_reply
                && let Some(root) = remote_thread.root_comment()
            {
                actions.push(ReplyAction::PostReply {
                    thread_id: local_thread.thread_id.clone(),
                    in_reply_to: root.database_id,
                    body: draft.clone(),
                });
            }

            // Detect resolve change (only unresolved -> resolved)
            if local_thread.resolve
                && !remote_thread.is_resolved
                && let Some(node_id) = &remote_thread.id
            {
                actions.push(ReplyAction::ResolveThread {
                    thread_id: local_thread.thread_id.clone(),
                    thread_node_id: node_id.clone(),
                });
            }
        }

        Self { actions, conflicts }
    }

    /// Check whether there are any changes to apply.
    pub fn has_changes(&self) -> bool {
        !self.actions.is_empty()
    }

    /// Check whether there are any conflicts.
    pub fn has_conflicts(&self) -> bool {
        !self.conflicts.is_empty()
    }

    /// Display the change summary to stdout.
    pub fn display(&self) {
        if self.actions.is_empty() {
            println!("No changes to push.");
            return;
        }

        let reply_count = self
            .actions
            .iter()
            .filter(|a| matches!(a, ReplyAction::PostReply { .. }))
            .count();
        let resolve_count = self
            .actions
            .iter()
            .filter(|a| matches!(a, ReplyAction::ResolveThread { .. }))
            .count();

        println!("Changes to push:");
        if reply_count > 0 {
            println!("  - {reply_count} reply(ies) to post");
        }
        if resolve_count > 0 {
            println!("  - {resolve_count} thread(s) to resolve");
        }

        if !self.conflicts.is_empty() {
            println!("\nConflicts detected:");
            for conflict in &self.conflicts {
                println!(
                    "  - Thread {} ({}): {}",
                    conflict.thread_id, conflict.path, conflict.reason
                );
            }
        }
    }

    /// Format conflict details as a string (for error reporting).
    pub fn format_conflicts(&self) -> String {
        self.conflicts
            .iter()
            .map(|c| format!("  - {} ({}): {}", c.thread_id, c.path, c.reason))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Apply all actions to GitHub.
    /// Actions are executed in order: replies first, then resolves.
    pub async fn apply(
        &self,
        client: &GitHubClient,
        owner: &str,
        repo: &str,
        pr_number: u64,
    ) -> Result<(), PrReviewError> {
        // Post replies first
        for action in &self.actions {
            if let ReplyAction::PostReply {
                thread_id,
                in_reply_to,
                body,
            } = action
            {
                client
                    .reply_to_pr_review_comment(owner, repo, pr_number, *in_reply_to, body)
                    .await
                    .map_err(|e| PrReviewError::ReplyPostFailed {
                        thread_id: thread_id.clone(),
                        details: e.to_string(),
                    })?;
            }
        }

        // Then resolve threads
        for action in &self.actions {
            if let ReplyAction::ResolveThread {
                thread_id,
                thread_node_id,
            } = action
            {
                client
                    .resolve_review_thread(thread_node_id)
                    .await
                    .map_err(|e| PrReviewError::ResolveFailed {
                        thread_id: thread_id.clone(),
                        details: e.to_string(),
                    })?;
            }
        }

        Ok(())
    }
}

/// Check if a thread has comments created after the given timestamp.
///
/// Uses `chrono::DateTime` parsing to handle format differences (e.g.,
/// fractional seconds) that would break lexicographic string comparison.
fn has_new_comments_since(thread: &ReviewThread, pulled_at: &str) -> bool {
    let Ok(pulled_at_dt) = chrono::DateTime::parse_from_rfc3339(pulled_at) else {
        return false;
    };
    thread.comments.nodes.iter().any(|c| {
        chrono::DateTime::parse_from_rfc3339(&c.created_at).is_ok_and(|dt| dt > pulled_at_dt)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::gh::pr_review::markdown::ParsedThread;
    use crate::commands::gh::pr_review::markdown::serializer::ThreadsFrontmatter;
    use crate::commands::gh::pr_review::models::{
        Comment,
        comment::{Author, PullRequestReview},
        thread::CommentsNode,
    };
    use rstest::rstest;

    fn make_comment(id: i64, author: &str, created_at: &str) -> Comment {
        Comment {
            database_id: id,
            author: Some(Author {
                login: author.to_string(),
            }),
            body: "comment".to_string(),
            created_at: created_at.to_string(),
            path: Some("src/main.rs".to_string()),
            line: Some(10),
            original_line: None,
            diff_hunk: None,
            reply_to: None,
            pull_request_review: Some(PullRequestReview { database_id: 100 }),
        }
    }

    fn make_remote_thread(
        node_id: &str,
        comments: Vec<Comment>,
        is_resolved: bool,
    ) -> ReviewThread {
        ReviewThread {
            id: Some(node_id.to_string()),
            is_resolved,
            comments: CommentsNode { nodes: comments },
        }
    }

    fn make_local_thread(
        thread_id: &str,
        resolve: bool,
        draft_reply: Option<&str>,
    ) -> ParsedThread {
        ParsedThread {
            thread_id: thread_id.to_string(),
            path: "src/main.rs".to_string(),
            line: Some(10),
            resolve,
            draft_reply: draft_reply.map(|s| s.to_string()),
        }
    }

    fn make_parsed_file(threads: Vec<ParsedThread>) -> ParsedThreadsFile {
        ParsedThreadsFile {
            frontmatter: ThreadsFrontmatter {
                pr: 42,
                repo: "fohte/armyknife".to_string(),
                pulled_at: "2024-01-15T10:00:00Z".to_string(),
                submit: false,
            },
            reviews: vec![],
            threads,
        }
    }

    #[rstest]
    fn test_detect_no_changes() {
        let local = make_parsed_file(vec![make_local_thread("RT_abc", false, None)]);
        let remote = PrData {
            reviews: vec![],
            threads: vec![make_remote_thread(
                "RT_abc",
                vec![make_comment(1, "user", "2024-01-15T09:00:00Z")],
                false,
            )],
        };

        let changeset = ReplyChangeSet::detect(&local, &remote, "2024-01-15T10:00:00Z");
        assert!(!changeset.has_changes());
        assert!(!changeset.has_conflicts());
    }

    #[rstest]
    fn test_detect_draft_reply() {
        let local = make_parsed_file(vec![make_local_thread("RT_abc", false, Some("My reply"))]);
        let remote = PrData {
            reviews: vec![],
            threads: vec![make_remote_thread(
                "RT_abc",
                vec![make_comment(1, "user", "2024-01-15T09:00:00Z")],
                false,
            )],
        };

        let changeset = ReplyChangeSet::detect(&local, &remote, "2024-01-15T10:00:00Z");
        assert!(changeset.has_changes());
        assert_eq!(changeset.actions.len(), 1);
        assert!(matches!(
            &changeset.actions[0],
            ReplyAction::PostReply { body, in_reply_to: 1, .. } if body == "My reply"
        ));
    }

    #[rstest]
    fn test_detect_resolve() {
        let local = make_parsed_file(vec![make_local_thread("RT_abc", true, None)]);
        let remote = PrData {
            reviews: vec![],
            threads: vec![make_remote_thread(
                "RT_abc",
                vec![make_comment(1, "user", "2024-01-15T09:00:00Z")],
                false,
            )],
        };

        let changeset = ReplyChangeSet::detect(&local, &remote, "2024-01-15T10:00:00Z");
        assert!(changeset.has_changes());
        assert_eq!(changeset.actions.len(), 1);
        assert!(matches!(
            &changeset.actions[0],
            ReplyAction::ResolveThread { thread_node_id, .. } if thread_node_id == "RT_abc"
        ));
    }

    #[rstest]
    fn test_detect_already_resolved_skipped() {
        let local = make_parsed_file(vec![make_local_thread("RT_abc", true, None)]);
        let remote = PrData {
            reviews: vec![],
            threads: vec![make_remote_thread(
                "RT_abc",
                vec![make_comment(1, "user", "2024-01-15T09:00:00Z")],
                true, // already resolved
            )],
        };

        let changeset = ReplyChangeSet::detect(&local, &remote, "2024-01-15T10:00:00Z");
        assert!(!changeset.has_changes());
    }

    #[rstest]
    fn test_detect_reply_and_resolve() {
        let local = make_parsed_file(vec![make_local_thread("RT_abc", true, Some("Fixed!"))]);
        let remote = PrData {
            reviews: vec![],
            threads: vec![make_remote_thread(
                "RT_abc",
                vec![make_comment(1, "user", "2024-01-15T09:00:00Z")],
                false,
            )],
        };

        let changeset = ReplyChangeSet::detect(&local, &remote, "2024-01-15T10:00:00Z");
        assert_eq!(changeset.actions.len(), 2);
        assert!(matches!(
            &changeset.actions[0],
            ReplyAction::PostReply { .. }
        ));
        assert!(matches!(
            &changeset.actions[1],
            ReplyAction::ResolveThread { .. }
        ));
    }

    #[rstest]
    fn test_detect_conflict() {
        let local = make_parsed_file(vec![make_local_thread("RT_abc", false, Some("My reply"))]);
        let remote = PrData {
            reviews: vec![],
            threads: vec![make_remote_thread(
                "RT_abc",
                vec![
                    make_comment(1, "user", "2024-01-15T09:00:00Z"),
                    // New comment after pull
                    make_comment(2, "other", "2024-01-15T11:00:00Z"),
                ],
                false,
            )],
        };

        let changeset = ReplyChangeSet::detect(&local, &remote, "2024-01-15T10:00:00Z");
        assert!(changeset.has_conflicts());
        assert_eq!(changeset.conflicts.len(), 1);
        // Still detects the action even with conflicts
        assert!(changeset.has_changes());
    }

    #[rstest]
    fn test_detect_thread_not_in_remote() {
        let local = make_parsed_file(vec![make_local_thread("RT_missing", false, Some("Reply"))]);
        let remote = PrData {
            reviews: vec![],
            threads: vec![],
        };

        let changeset = ReplyChangeSet::detect(&local, &remote, "2024-01-15T10:00:00Z");
        assert!(!changeset.has_changes());
    }

    #[rstest]
    fn test_detect_conflict_with_fractional_seconds() {
        let local = make_parsed_file(vec![make_local_thread("RT_abc", false, Some("My reply"))]);
        let remote = PrData {
            reviews: vec![],
            threads: vec![make_remote_thread(
                "RT_abc",
                vec![
                    make_comment(1, "user", "2024-01-15T09:00:00Z"),
                    // New comment with fractional seconds (must still be detected)
                    make_comment(2, "other", "2024-01-15T11:00:00.123Z"),
                ],
                false,
            )],
        };

        let changeset = ReplyChangeSet::detect(&local, &remote, "2024-01-15T10:00:00Z");
        assert!(changeset.has_conflicts());
    }
}
