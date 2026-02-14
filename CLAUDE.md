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

## Test code rules

### Parameterize similar test cases with rstest

Do not write multiple test functions that differ only in input/expected values. Use `#[rstest]` with `#[case]`.

```rust
// bad: separate functions per case
#[test]
fn test_parse_empty() { assert_eq!(parse(""), None); }
#[test]
fn test_parse_valid() { assert_eq!(parse("hello"), Some("hello")); }

// good: parameterized
#[rstest]
#[case::empty("", None)]
#[case::valid("hello", Some("hello"))]
fn test_parse(#[case] input: &str, #[case] expected: Option<&str>) {
    assert_eq!(parse(input), expected);
}
```

### Always name `#[case]` variants

Use `#[case::descriptive_name(...)]`, not bare `#[case(...)]`. Named cases identify failures without inspecting values.

### Use `#[fixture]` for shared test setup

Do not repeat the same setup code across tests. Extract into `#[fixture]`.

```rust
// bad: duplicated setup
#[rstest]
fn test_a() { let repo = make_repo(); /* ... */ }
#[rstest]
fn test_b() { let repo = make_repo(); /* ... */ }

// good: fixture injection
#[fixture]
fn repo() -> Repo { make_repo() }
#[rstest]
fn test_a(repo: Repo) { /* ... */ }
```

### Use `indoc!` for multiline string literals in tests

Do not embed `\n` in string literals. Use `indoc!` for readability.

### Extract repeated assertions into helper functions

If the same assertion chain appears in 3+ tests, extract it into a helper.

### Do not write tests that only verify test helpers

Tests must verify production code. Tests that only assert on test helpers, fixtures, or mocks are unnecessary. Remove them.
