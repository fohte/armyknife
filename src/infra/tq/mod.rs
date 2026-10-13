//! tq CLI integration.
//!
//! Provides [`TqClient`] for the user's personal task manager, tq, via the
//! `tq` command-line tool rather than its HTTP API directly.

mod client;
mod error;

pub use client::{
    SessionTasks, TqChecklist, TqChecklistItem, TqClient, TqGithubLink, TqTask, TqTaskStatus,
};
pub use error::{Result, TqError};
