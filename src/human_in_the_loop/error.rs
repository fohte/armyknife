use std::io;
use std::path::PathBuf;
use thiserror::Error;

#[derive(Error, Debug)]
#[allow(dead_code)]
pub enum HumanInTheLoopError {
    #[error("File not found: {0}")]
    FileNotFound(PathBuf),

    #[error("Document was not approved")]
    NotApproved,

    #[error("Document has been modified after approval")]
    ModifiedAfterApproval,

    #[error("IO error: {0}")]
    Io(#[from] io::Error),

    #[error("YAML parse error: {0}")]
    YamlParse(#[from] serde_yaml::Error),

    #[error("Command failed: {0}")]
    CommandFailed(String),
}

pub type Result<T> = std::result::Result<T, HumanInTheLoopError>;
