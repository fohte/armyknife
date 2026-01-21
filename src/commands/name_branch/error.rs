use std::io;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error {
    #[error("Failed to generate branch name: {0}")]
    GenerationFailed(String),

    #[error("Invalid branch name: {0}")]
    InvalidBranchName(String),

    #[error("IO error: {0}")]
    Io(#[from] io::Error),
}

pub type Result<T> = std::result::Result<T, Error>;
