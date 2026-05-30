// The library shares the binary's module tree so the in-workspace
// `config-schema-gen` crate can reach `config::generate_schema`. Everything
// outside the config code path is unused from the library's perspective —
// it's exercised by the `a` binary — so suppress the resulting dead-code
// noise here rather than in every leaf module.
#![allow(
    dead_code,
    unused_imports,
    clippy::allow_attributes,
    reason = "binary-only code reached through the shared module tree"
)]

mod cli;
mod commands;
mod infra;
mod shared;

pub use shared::config;
