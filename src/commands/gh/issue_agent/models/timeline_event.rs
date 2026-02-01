//! Timeline event types for GitHub Issue timeline.
//!
//! These types represent events in an issue's timeline (label changes,
//! cross-references, assignments, etc.) as returned by the GitHub GraphQL API.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::Author;

/// Union of all supported timeline event types.
///
/// Uses `#[serde(tag = "__typename")]` to deserialize based on the GraphQL
/// `__typename` field. Unknown event types are captured as `Unknown` and
/// can be safely ignored during display.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "__typename")]
pub enum TimelineItem {
    /// Reference from another issue or PR.
    CrossReferencedEvent(CrossReferencedEvent),
    /// Label added to the issue.
    LabeledEvent(LabeledEvent),
    /// Label removed from the issue.
    UnlabeledEvent(UnlabeledEvent),
    /// User assigned to the issue.
    AssignedEvent(AssignedEvent),
    /// User unassigned from the issue.
    UnassignedEvent(UnassignedEvent),
    /// Issue was closed.
    ClosedEvent(ClosedEvent),
    /// Issue was reopened.
    ReopenedEvent(ReopenedEvent),
    /// Unknown or unsupported event type (ignored during display).
    #[serde(other)]
    Unknown,
}

impl TimelineItem {
    /// Get the timestamp when this event occurred.
    /// Returns `None` for unknown event types.
    pub fn created_at(&self) -> Option<DateTime<Utc>> {
        match self {
            TimelineItem::CrossReferencedEvent(e) => Some(e.created_at),
            TimelineItem::LabeledEvent(e) => Some(e.created_at),
            TimelineItem::UnlabeledEvent(e) => Some(e.created_at),
            TimelineItem::AssignedEvent(e) => Some(e.created_at),
            TimelineItem::UnassignedEvent(e) => Some(e.created_at),
            TimelineItem::ClosedEvent(e) => Some(e.created_at),
            TimelineItem::ReopenedEvent(e) => Some(e.created_at),
            TimelineItem::Unknown => None,
        }
    }

    /// Get the actor (user) who triggered this event.
    /// Returns `None` for unknown event types or events without an actor.
    #[cfg(test)]
    pub fn actor(&self) -> Option<&Author> {
        match self {
            TimelineItem::CrossReferencedEvent(e) => e.actor.as_ref(),
            TimelineItem::LabeledEvent(e) => e.actor.as_ref(),
            TimelineItem::UnlabeledEvent(e) => e.actor.as_ref(),
            TimelineItem::AssignedEvent(e) => e.actor.as_ref(),
            TimelineItem::UnassignedEvent(e) => e.actor.as_ref(),
            TimelineItem::ClosedEvent(e) => e.actor.as_ref(),
            TimelineItem::ReopenedEvent(e) => e.actor.as_ref(),
            TimelineItem::Unknown => None,
        }
    }

    /// Returns true if this is an unknown/unsupported event type.
    pub fn is_unknown(&self) -> bool {
        matches!(self, TimelineItem::Unknown)
    }
}

/// Event: Another issue or PR referenced this issue.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CrossReferencedEvent {
    pub created_at: DateTime<Utc>,
    pub actor: Option<Author>,
    /// The issue or PR that referenced this issue.
    pub source: CrossReferenceSource,
    /// Whether the reference will close this issue when merged.
    #[serde(default)]
    pub will_close_target: bool,
}

/// Source of a cross-reference (either an Issue or PullRequest).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "__typename")]
pub enum CrossReferenceSource {
    Issue(IssueReference),
    PullRequest(PullRequestReference),
}

impl CrossReferenceSource {
    /// Get the repository (owner/name format) of the source.
    pub fn repository(&self) -> String {
        match self {
            CrossReferenceSource::Issue(i) => {
                format!("{}/{}", i.repository.owner.login, i.repository.name)
            }
            CrossReferenceSource::PullRequest(pr) => {
                format!("{}/{}", pr.repository.owner.login, pr.repository.name)
            }
        }
    }

    /// Get the issue/PR number.
    pub fn number(&self) -> i64 {
        match self {
            CrossReferenceSource::Issue(i) => i.number,
            CrossReferenceSource::PullRequest(pr) => pr.number,
        }
    }

    /// Get the title.
    pub fn title(&self) -> &str {
        match self {
            CrossReferenceSource::Issue(i) => &i.title,
            CrossReferenceSource::PullRequest(pr) => &pr.title,
        }
    }

    /// Check if the source is a PR.
    pub fn is_pull_request(&self) -> bool {
        matches!(self, CrossReferenceSource::PullRequest(_))
    }
}

/// Reference to an issue.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IssueReference {
    pub number: i64,
    pub title: String,
    pub repository: RepositoryReference,
}

/// Reference to a pull request.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PullRequestReference {
    pub number: i64,
    pub title: String,
    pub repository: RepositoryReference,
}

/// Reference to a repository (owner and name).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RepositoryReference {
    pub name: String,
    pub owner: RepositoryOwner,
}

/// Repository owner.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RepositoryOwner {
    pub login: String,
}

/// Event: A label was added to the issue.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LabeledEvent {
    pub created_at: DateTime<Utc>,
    pub actor: Option<Author>,
    pub label: LabelInfo,
}

/// Event: A label was removed from the issue.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UnlabeledEvent {
    pub created_at: DateTime<Utc>,
    pub actor: Option<Author>,
    pub label: LabelInfo,
}

/// Label information for labeled/unlabeled events.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LabelInfo {
    pub name: String,
}

/// Event: A user was assigned to the issue.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AssignedEvent {
    pub created_at: DateTime<Utc>,
    pub actor: Option<Author>,
    pub assignee: Option<Author>,
}

/// Event: A user was unassigned from the issue.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UnassignedEvent {
    pub created_at: DateTime<Utc>,
    pub actor: Option<Author>,
    pub assignee: Option<Author>,
}

/// Event: The issue was closed.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ClosedEvent {
    pub created_at: DateTime<Utc>,
    pub actor: Option<Author>,
}

/// Event: The issue was reopened.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReopenedEvent {
    pub created_at: DateTime<Utc>,
    pub actor: Option<Author>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    fn make_author(login: &str) -> Author {
        Author {
            login: login.to_string(),
        }
    }

    fn make_timestamp() -> DateTime<Utc> {
        "2024-01-15T10:00:00Z".parse().unwrap()
    }

    #[rstest]
    #[case::cross_referenced(
        TimelineItem::CrossReferencedEvent(CrossReferencedEvent {
            created_at: make_timestamp(),
            actor: Some(make_author("user1")),
            source: CrossReferenceSource::Issue(IssueReference {
                number: 1,
                title: "Test".to_string(),
                repository: RepositoryReference {
                    name: "repo".to_string(),
                    owner: RepositoryOwner { login: "owner".to_string() },
                },
            }),
            will_close_target: false,
        }),
        Some(make_timestamp()),
        Some("user1"),
        false
    )]
    #[case::labeled(
        TimelineItem::LabeledEvent(LabeledEvent {
            created_at: make_timestamp(),
            actor: Some(make_author("user2")),
            label: LabelInfo { name: "bug".to_string() },
        }),
        Some(make_timestamp()),
        Some("user2"),
        false
    )]
    #[case::unknown(TimelineItem::Unknown, None, None, true)]
    fn test_timeline_item_accessors(
        #[case] item: TimelineItem,
        #[case] expected_created_at: Option<DateTime<Utc>>,
        #[case] expected_actor_login: Option<&str>,
        #[case] expected_is_unknown: bool,
    ) {
        assert_eq!(item.created_at(), expected_created_at);
        assert_eq!(item.actor().map(|a| a.login.as_str()), expected_actor_login);
        assert_eq!(item.is_unknown(), expected_is_unknown);
    }

    #[test]
    fn test_cross_reference_source_issue() {
        let source = CrossReferenceSource::Issue(IssueReference {
            number: 42,
            title: "Issue Title".to_string(),
            repository: RepositoryReference {
                name: "repo".to_string(),
                owner: RepositoryOwner {
                    login: "owner".to_string(),
                },
            },
        });

        assert_eq!(source.repository(), "owner/repo");
        assert_eq!(source.number(), 42);
        assert_eq!(source.title(), "Issue Title");
        assert!(!source.is_pull_request());
    }

    #[test]
    fn test_cross_reference_source_pull_request() {
        let source = CrossReferenceSource::PullRequest(PullRequestReference {
            number: 99,
            title: "PR Title".to_string(),
            repository: RepositoryReference {
                name: "other-repo".to_string(),
                owner: RepositoryOwner {
                    login: "other-owner".to_string(),
                },
            },
        });

        assert_eq!(source.repository(), "other-owner/other-repo");
        assert_eq!(source.number(), 99);
        assert_eq!(source.title(), "PR Title");
        assert!(source.is_pull_request());
    }

    #[test]
    fn test_deserialize_timeline_item_labeled() {
        let json = r#"{
            "__typename": "LabeledEvent",
            "createdAt": "2024-01-15T10:00:00Z",
            "actor": {"login": "testuser"},
            "label": {"name": "bug"}
        }"#;

        let item: TimelineItem = serde_json::from_str(json).unwrap();
        assert!(matches!(item, TimelineItem::LabeledEvent(_)));

        if let TimelineItem::LabeledEvent(e) = item {
            assert_eq!(e.label.name, "bug");
            assert_eq!(e.actor.unwrap().login, "testuser");
        }
    }

    #[test]
    fn test_deserialize_timeline_item_cross_referenced_pr() {
        let json = r#"{
            "__typename": "CrossReferencedEvent",
            "createdAt": "2024-01-15T10:00:00Z",
            "actor": {"login": "dev1"},
            "source": {
                "__typename": "PullRequest",
                "number": 123,
                "title": "Add feature",
                "repository": {
                    "name": "myrepo",
                    "owner": {"login": "myowner"}
                }
            },
            "willCloseTarget": true
        }"#;

        let item: TimelineItem = serde_json::from_str(json).unwrap();

        if let TimelineItem::CrossReferencedEvent(e) = item {
            assert!(e.will_close_target);
            assert!(e.source.is_pull_request());
            assert_eq!(e.source.repository(), "myowner/myrepo");
            assert_eq!(e.source.number(), 123);
        } else {
            panic!("Expected CrossReferencedEvent");
        }
    }

    #[test]
    fn test_deserialize_unknown_event_type() {
        let json = r#"{
            "__typename": "SomeNewEventType",
            "createdAt": "2024-01-15T10:00:00Z"
        }"#;

        let item: TimelineItem = serde_json::from_str(json).unwrap();
        assert!(item.is_unknown());
    }
}
