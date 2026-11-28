//! tq CLI integration.
//!
//! Provides [`TqClient`] for the user's personal task manager, tq, via the
//! `tq` command-line tool rather than its HTTP API directly.

mod client;
mod error;
mod events;

pub use client::{
    SessionTasks, TqChecklist, TqChecklistItem, TqClient, TqGithubLink, TqProjectSummary, TqTask,
    TqTaskDetails, TqTaskStatus, TqTaskSummary,
};
pub use error::{Result, TqError};
pub use events::{TqChangeEvent, TqChangeResource, TqEvent, TqEventSubscription, subscribe_events};
