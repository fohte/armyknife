//! tq API client module using reqwest.
//!
//! Provides TqClient for the user's personal task manager, tq
//! (<https://tq.fohte.net>).

mod client;
mod error;
#[cfg(test)]
pub(crate) mod mock;

pub use client::{TaskSessionLink, TqClient, TqTask};
pub use error::{Result, TqError};
