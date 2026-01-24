mod author;
mod comment;
mod issue;
mod metadata;

#[expect(unused_imports, reason = "WithAuthor used via re-export in submodules")]
pub use author::{Author, WithAuthor};
pub use comment::Comment;
pub use issue::{Issue, Label, Milestone};
pub use metadata::IssueMetadata;
