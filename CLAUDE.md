# armyknife

Rust CLI toolkit for AI-assisted development workflows.

## Tech Stack

- Rust (see rust-toolchain.toml)
- clap (CLI framework with derive macros)
- rstest, serial_test (testing)
- thiserror (error handling)
- serde, serde_yaml (serialization)

## Directory Structure

- Organized by subcommand (e.g., `ai/pr_draft/` for `a ai pr-draft`)
- Shared modules extracted when reusable (e.g., `human_in_the_loop/`)

## Development

- `cargo build` / `cargo test`
- Formatting and linting run automatically via lefthook pre-commit

## Architecture

### Human-in-the-Loop Pattern

Generic framework for interactive document editing:

- `DocumentSchema` trait: Define frontmatter structure with `is_approved()`
- `Document<S>`: Frontmatter (YAML) + body (markdown)
- `ReviewHandler<S>`: Callbacks for review workflow

### Error Handling

- Use `thiserror` for domain-specific error types
- Define `Result<T>` type aliases per module

## Standards

- Comments: English (public repo), explain WHY not WHAT
- Tests: Use rstest for parametrized tests (see test skill)
