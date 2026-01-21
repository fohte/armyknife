use std::io;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error {
    #[error("Invalid branch name: {0}")]
    InvalidBranchName(String),

    #[error("IO error: {0}")]
    Io(#[from] io::Error),
}

pub type Result<T> = anyhow::Result<T>;
