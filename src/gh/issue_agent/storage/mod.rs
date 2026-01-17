// These types will be used in future phases of gh-issue-agent
#![allow(dead_code)]
#![allow(unused_imports)]

mod error;
mod issue_storage;
mod paths;
mod read;

pub use error::{Result, StorageError};
pub use issue_storage::{IssueStorage, LocalChanges};
pub use read::{CommentFileMetadata, LocalComment};
