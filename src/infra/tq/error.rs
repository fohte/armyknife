//! tq CLI error types.

use std::fmt;

use thiserror::Error;

#[derive(Error, Debug)]
pub enum TqError {
    #[error("{}", .0)]
    CommandFailed(CommandFailedError),

    #[error("failed to parse tq output as JSON: {0}")]
    InvalidOutput(#[from] serde_json::Error),
}

#[derive(Debug)]
pub struct CommandFailedError {
    pub args: Vec<String>,
    pub message: String,
    pub stderr: Option<String>,
}

impl fmt::Display for CommandFailedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "tq {} failed: {}", self.args.join(" "), self.message)?;
        if let Some(stderr) = &self.stderr
            && !stderr.is_empty()
        {
            writeln!(f)?;
            writeln!(f)?;
            writeln!(f, "-- stderr --")?;
            write!(f, "{stderr}")?;
        }
        Ok(())
    }
}

impl TqError {
    pub(crate) fn command_failed<S: AsRef<str>>(
        args: &[S],
        message: impl Into<String>,
        stderr: Option<String>,
    ) -> Self {
        Self::CommandFailed(CommandFailedError {
            args: args.iter().map(|s| s.as_ref().to_string()).collect(),
            message: message.into(),
            stderr,
        })
    }
}

pub type Result<T> = std::result::Result<T, TqError>;
