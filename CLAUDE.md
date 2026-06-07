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

### No external command or system service dependencies

Tests must not depend on external commands (tmux, ps, terminal-notifier, git CLI, etc.) or system services (XPC, D-Bus, network). When testing production code that calls external commands, isolate the external call boundary so tests exercise logic without invoking real commands. Reason: tests run in srt sandbox / CI where external commands may be blocked or absent, causing hangs or failures.

### Assert on the whole output with a single equality check

Treat each test as a spec: build the expected output as one literal value (object, struct, JSON, array, etc.) and compare it to the actual output with a single equality assertion. Do not split the assertion into per-field checks, and do not use partial matchers (substring contains, `toContain`, `toMatchObject`, prefix/suffix checks, regex-on-substring, etc.). Partial matches silently ignore unexpected fields and extra elements, so the test stops working as a spec the moment the shape of the output changes.

```rust
// bad
let ev = run();
assert_eq!(ev["path"], "/a");
assert_eq!(ev["event"], "ok");
assert!(ev["message"].as_str().unwrap().contains("done"));

// good
assert_eq!(
    run(),
    json!({
        "path": "/a",
        "event": "ok",
        "message": "done",
    }),
);
```

For dynamic fields (timestamps, UUIDs, random IDs), normalize them in a helper before the comparison (e.g. replace with a fixed placeholder) so the full output can still be asserted in one equality check. Do not weaken the assertion to dodge the dynamic value.

The `no-assert-contains` ast-grep rule rejects `assert!(x.contains(...))` at the expression level; this guideline is the broader principle that the rule is one instance of.

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
