use std::path::PathBuf;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum StorageError {
    #[error("Issue directory not found: {0}")]
    IssueDirectoryNotFound(PathBuf),

    #[error("File not found: {0}")]
    FileNotFound(PathBuf),

    #[error("Failed to parse comment metadata in {path}: {message}")]
    CommentMetadataParseError { path: PathBuf, message: String },

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON parse error: {0}")]
    JsonParse(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, StorageError>;
