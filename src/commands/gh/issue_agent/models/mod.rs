mod author;
mod comment;
mod issue;
mod issue_template;
mod metadata;
mod new_issue;
mod timeline_event;

pub use author::Author;
pub use comment::Comment;
pub use issue::Issue;
#[cfg(test)]
pub use issue::Label;
pub use issue_template::IssueTemplate;
#[cfg(test)]
pub use metadata::ReadonlyMetadata;
pub use metadata::{IssueFrontmatter, IssueMetadata};
pub use new_issue::NewIssue;
#[cfg(test)]
pub use new_issue::NewIssueFrontmatter;
pub use timeline_event::TimelineItem;

// Re-export for testing module
#[cfg(test)]
pub(crate) use timeline_event::{
    AssignedEvent, ClosedEvent, CrossReferenceSource, CrossReferencedEvent, IssueReference,
    LabelInfo, LabeledEvent, PullRequestReference, ReopenedEvent, RepositoryOwner,
    RepositoryReference, UnassignedEvent, UnlabeledEvent,
};
