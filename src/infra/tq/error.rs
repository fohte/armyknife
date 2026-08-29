//! tq API error types.

use thiserror::Error;

#[derive(Error, Debug)]
pub enum TqError {
    #[error("tq API error: HTTP {0}")]
    ApiError(u16),

    #[error("HTTP request failed: {0}")]
    HttpError(#[from] reqwest::Error),
}

pub type Result<T> = anyhow::Result<T>;
