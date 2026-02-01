use std::path::PathBuf;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum StorageError {
    #[error("File not found: {0}")]
    FileNotFound(PathBuf),

    #[error("File already exists: {0}")]
    FileAlreadyExists(PathBuf),

    #[error("Failed to parse comment metadata in {path}: {message}")]
    CommentMetadataParseError { path: PathBuf, message: String },

    #[error("Parse error: {0}")]
    ParseError(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON parse error: {0}")]
    JsonParse(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, StorageError>;
