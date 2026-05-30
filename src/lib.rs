// The binary (`a`) is a thin wrapper over this library. The modules below
// are exposed publicly only so the binary and the in-workspace
// `config-schema-gen` crate can reach them — they are not a stable external
// API. Internal trait shapes (async fns in traits, missing `Default` impls)
// and items whose lints stop firing once they become reachable via a public
// path are suppressed here rather than in every leaf module.
#![allow(
    async_fn_in_trait,
    clippy::new_without_default,
    clippy::allow_attributes,
    reason = "library exposes the binary's module tree, not a stable external API"
)]

pub mod cli;
pub mod commands;
pub mod infra;
pub mod shared;
