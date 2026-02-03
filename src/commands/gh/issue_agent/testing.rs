//! Test factories for gh/issue_agent types.
//!
//! This module provides factory functions for creating test objects specific to
//! the gh/issue_agent module. Use `*_with()` variants to customize specific fields.
//!
//! # Example
//! ```ignore
//! use crate::commands::gh::issue_agent::testing::factories;
//!
//! let issue = factories::issue_with(|i| {
//!     i.title = "Custom Title".to_string();
//!     i.number = 42;
//! });
//! ```

pub mod factories {
    use chrono::{DateTime, Duration, Utc};

    use crate::commands::gh::issue_agent::models::{
        AssignedEvent, Author, ClosedEvent, Comment, CrossReferenceSource, CrossReferencedEvent,
        Issue, IssueReference, Label, LabelInfo, LabeledEvent, PullRequestReference, ReopenedEvent,
        RepositoryOwner, RepositoryReference, TimelineItem, UnassignedEvent, UnlabeledEvent,
    };
    use crate::commands::gh::issue_agent::storage::{CommentFileMetadata, LocalComment};

    // =========================================================================
    // Issue factories
    // =========================================================================

    /// Create an Issue with default test values.
    pub fn issue() -> Issue {
        Issue {
            number: 1,
            title: "Test Issue".to_string(),
            body: Some("Test body".to_string()),
            state: "OPEN".to_string(),
            labels: vec![],
            assignees: vec![],
            milestone: None,
            author: Some(Author {
                login: "testuser".to_string(),
            }),
            // Use relative times for consistent relative time formatting in tests
            created_at: Utc::now() - Duration::hours(2),
            updated_at: Utc::now(),
        }
    }

    /// Create an Issue with customizations applied via closure.
    pub fn issue_with(f: impl FnOnce(&mut Issue)) -> Issue {
        let mut i = issue();
        f(&mut i);
        i
    }

    // =========================================================================
    // Comment factories
    // =========================================================================

    /// Create a Comment with default test values.
    pub fn comment() -> Comment {
        let created_at = Utc::now() - Duration::hours(1);
        Comment {
            id: "IC_123".to_string(),
            database_id: 123,
            author: Some(Author {
                login: "commenter".to_string(),
            }),
            created_at,
            updated_at: created_at,
            body: "Test comment".to_string(),
        }
    }

    /// Create a Comment with customizations applied via closure.
    pub fn comment_with(f: impl FnOnce(&mut Comment)) -> Comment {
        let mut c = comment();
        f(&mut c);
        c
    }

    // =========================================================================
    // LocalComment factories
    // =========================================================================

    /// Create a LocalComment with default test values.
    pub fn local_comment() -> LocalComment {
        LocalComment {
            filename: "001_comment_123.md".to_string(),
            body: "Test comment body".to_string(),
            metadata: CommentFileMetadata {
                author: Some("testuser".to_string()),
                created_at: Some("2024-01-01T00:00:00+00:00".to_string()),
                updated_at: Some("2024-01-01T00:00:00+00:00".to_string()),
                id: Some("IC_123".to_string()),
                database_id: Some(123),
            },
        }
    }

    /// Create a LocalComment with customizations applied via closure.
    pub fn local_comment_with(f: impl FnOnce(&mut LocalComment)) -> LocalComment {
        let mut c = local_comment();
        f(&mut c);
        c
    }

    // =========================================================================
    // Helper factories
    // =========================================================================

    /// Create an Author with the given login.
    pub fn author(login: &str) -> Author {
        Author {
            login: login.to_string(),
        }
    }

    /// Create a Label with the given name.
    pub fn label(name: &str) -> Label {
        Label {
            name: name.to_string(),
        }
    }

    /// Create multiple labels from a slice of names.
    pub fn labels(names: &[&str]) -> Vec<Label> {
        names.iter().map(|n| label(n)).collect()
    }

    /// Create multiple authors (assignees) from a slice of logins.
    pub fn assignees(logins: &[&str]) -> Vec<Author> {
        logins.iter().map(|l| author(l)).collect()
    }

    // =========================================================================
    // Timeline Event factories
    // =========================================================================

    /// Create a default timestamp for timeline events.
    fn default_event_time() -> DateTime<Utc> {
        Utc::now() - Duration::minutes(30)
    }

    /// Create a LabeledEvent with the given parameters.
    pub fn labeled_event(actor_login: &str, label_name: &str) -> TimelineItem {
        TimelineItem::LabeledEvent(LabeledEvent {
            created_at: default_event_time(),
            actor: Some(author(actor_login)),
            label: LabelInfo {
                name: label_name.to_string(),
            },
        })
    }

    /// Create an UnlabeledEvent with the given parameters.
    pub fn unlabeled_event(actor_login: &str, label_name: &str) -> TimelineItem {
        TimelineItem::UnlabeledEvent(UnlabeledEvent {
            created_at: default_event_time(),
            actor: Some(author(actor_login)),
            label: LabelInfo {
                name: label_name.to_string(),
            },
        })
    }

    /// Create an AssignedEvent with the given parameters.
    pub fn assigned_event(actor_login: &str, assignee_login: &str) -> TimelineItem {
        TimelineItem::AssignedEvent(AssignedEvent {
            created_at: default_event_time(),
            actor: Some(author(actor_login)),
            assignee: Some(author(assignee_login)),
        })
    }

    /// Create an UnassignedEvent with the given parameters.
    pub fn unassigned_event(actor_login: &str, assignee_login: &str) -> TimelineItem {
        TimelineItem::UnassignedEvent(UnassignedEvent {
            created_at: default_event_time(),
            actor: Some(author(actor_login)),
            assignee: Some(author(assignee_login)),
        })
    }

    /// Create a ClosedEvent with the given actor.
    pub fn closed_event(actor_login: &str) -> TimelineItem {
        TimelineItem::ClosedEvent(ClosedEvent {
            created_at: default_event_time(),
            actor: Some(author(actor_login)),
        })
    }

    /// Create a ReopenedEvent with the given actor.
    pub fn reopened_event(actor_login: &str) -> TimelineItem {
        TimelineItem::ReopenedEvent(ReopenedEvent {
            created_at: default_event_time(),
            actor: Some(author(actor_login)),
        })
    }

    /// Create a CrossReferencedEvent from a PR.
    pub fn cross_referenced_pr(
        actor_login: &str,
        pr_number: i64,
        pr_title: &str,
        repo_owner: &str,
        repo_name: &str,
        will_close_target: bool,
    ) -> TimelineItem {
        TimelineItem::CrossReferencedEvent(CrossReferencedEvent {
            created_at: default_event_time(),
            actor: Some(author(actor_login)),
            source: CrossReferenceSource::PullRequest(PullRequestReference {
                number: pr_number,
                title: pr_title.to_string(),
                repository: RepositoryReference {
                    name: repo_name.to_string(),
                    owner: RepositoryOwner {
                        login: repo_owner.to_string(),
                    },
                },
            }),
            will_close_target,
        })
    }

    /// Create a CrossReferencedEvent from an Issue.
    pub fn cross_referenced_issue(
        actor_login: &str,
        issue_number: i64,
        issue_title: &str,
        repo_owner: &str,
        repo_name: &str,
    ) -> TimelineItem {
        TimelineItem::CrossReferencedEvent(CrossReferencedEvent {
            created_at: default_event_time(),
            actor: Some(author(actor_login)),
            source: CrossReferenceSource::Issue(IssueReference {
                number: issue_number,
                title: issue_title.to_string(),
                repository: RepositoryReference {
                    name: repo_name.to_string(),
                    owner: RepositoryOwner {
                        login: repo_owner.to_string(),
                    },
                },
            }),
            will_close_target: false,
        })
    }
}
