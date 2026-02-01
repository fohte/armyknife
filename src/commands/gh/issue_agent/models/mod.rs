mod author;
mod comment;
mod issue;
mod metadata;
mod timeline_event;

pub use author::Author;
pub use comment::Comment;
pub use issue::{Issue, Label, Milestone};
pub use metadata::IssueMetadata;
pub use timeline_event::TimelineItem;

// Re-export for testing module
#[cfg(test)]
pub(crate) use timeline_event::{
    AssignedEvent, ClosedEvent, CrossReferenceSource, CrossReferencedEvent, IssueReference,
    LabelInfo, LabeledEvent, PullRequestReference, ReopenedEvent, RepositoryOwner,
    RepositoryReference, UnassignedEvent, UnlabeledEvent,
};
