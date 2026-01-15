// These functions will be used in future phases of gh-issue-agent
#![allow(dead_code)]
#![allow(unused_imports)]

mod diff;
mod error;
mod paths;
mod read;
mod write;

pub use diff::{
    LocalChanges, detect_local_changes, detect_local_changes_from_dir, has_local_changes,
};
pub use error::{Result, StorageError};
pub use paths::{get_cache_dir, get_issue_dir};
pub use read::{
    CommentFileMetadata, LocalComment, read_comments, read_comments_from_dir, read_issue_body,
    read_issue_body_from_dir, read_metadata, read_metadata_from_dir,
};
pub use write::{
    save_comments, save_comments_to_dir, save_issue_body, save_issue_body_to_dir, save_metadata,
    save_metadata_to_dir,
};
