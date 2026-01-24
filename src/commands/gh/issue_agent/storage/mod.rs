// These types will be used in future phases of gh-issue-agent
#![expect(dead_code, reason = "module reserved for future use")]
#![expect(unused_imports, reason = "imports reserved for future use")]

mod error;
mod issue_storage;
mod issue_storage_diff;
mod issue_storage_read;
mod issue_storage_write;
mod paths;
mod read;

pub use error::{Result, StorageError};
pub use issue_storage::IssueStorage;
pub use issue_storage_diff::LocalChanges;
pub use read::{CommentFileMetadata, LocalComment};
