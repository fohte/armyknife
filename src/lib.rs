// The binary (`a`) is a thin wrapper over this library. Modules below are
// `pub` only so the binary and the in-workspace `config-schema-gen` crate
// can reach them — they are not a stable external API, hence the trait
// shape lints below are suppressed crate-wide.
#![expect(
    async_fn_in_trait,
    reason = "library exposes the binary's module tree, not a stable external API"
)]

pub mod cli;
pub mod commands;
pub mod infra;
pub mod shared;
