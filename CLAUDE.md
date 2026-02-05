# armyknife

Rust CLI toolkit for AI-assisted development workflows.

## Tech Stack

- Rust (see rust-toolchain.toml)
- clap (CLI framework with derive macros)
- rstest (testing)
- thiserror (error handling)
- serde, serde_yaml (serialization)

## Directory Structure

See [docs/architecture.md](docs/architecture.md).

## Development

- Build: `cargo build`
- Test: `mise run test` (runs tests in srt sandbox to prevent filesystem side effects)
- Formatting and linting run automatically via lefthook pre-commit
- Coverage: `cargo llvm-cov` (generates lcov.info, uploaded to Codecov in CI)

## Architecture

### Human-in-the-Loop Pattern

Generic framework for interactive document editing:

- `DocumentSchema` trait: Define frontmatter structure with `is_approved()`
- `Document<S>`: Frontmatter (YAML) + body (markdown)
- `ReviewHandler<S>`: Callbacks for review workflow

### Error Handling

- Use `thiserror` for domain-specific error types
- Define `Result<T>` type aliases per module
- Use `anyhow::Result<T>` for application-level function return types (e.g., command handlers)

## Standards

- Comments: English (public repo), explain WHY not WHAT
- Tests: Use `test` skill when writing/running tests. Tests must be isolated without side effects (no shared state, no serial execution)
- Documentation: Update README.md when adding, changing, or removing commands/subcommands

### Lints

- `unwrap()`, `expect()`, `panic!()` are forbidden in production code (allowed in tests)
- Use `#[expect(lint_name, reason = "...")]` instead of `#[allow]` when suppressing lints
- Do not create Tokio runtime directly; use async fn with the existing runtime from main()

### Dependencies

- Pin exact versions with `=` (e.g., `anyhow = "=1.0.100"`)
