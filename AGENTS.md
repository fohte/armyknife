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

### Release workflow

Release publication is automated with release-please. Do not tell the user to run `a update` immediately after their PR merges into `master`. Tell them to run it after the release PR is automatically merged and the GitHub release is created; they do not need to wait for binary uploads or monitor the asset build. `a update` retries when a published release is missing the asset for the current platform, but it does not wait for the release itself to be created. See the [Release section in README.md](README.md#release).

### Lints

- `unwrap()`, `expect()`, `panic!()` are forbidden in production code (allowed in tests)
- Use `#[expect(lint_name, reason = "...")]` instead of `#[allow]` when suppressing lints
- Do not create Tokio runtime directly; use async fn with the existing runtime from main()

### Dependencies

- Pin exact versions with `=` (e.g., `anyhow = "=1.0.100"`)

## Code organization rules

### Split files before they grow past ~500 lines of production code

When a change would push a file's non-test code past ~500 lines, split it along responsibility seams before adding more. Keep external import paths unchanged by keeping the entrypoint file in place and re-exporting the pieces you split out into new files (e.g. `foo.rs` gains a `foo/` directory for its submodules, `index.ts` re-exports from the new files). Tests move together with the code they verify.

Prefer creating a new focused file over appending to the largest existing one.

### テストや story のためだけに export しない

テストや story は、production のコードと同じ公開 API を通して対象を使う。内部の関数・定数・型をテストのためだけに export しない。単体でテストしたいロジックは別モジュールに切り出して production もそこから import し、story で描画する View は container とは別のファイルに置く。

`knip --production` がこの違反を検出する。指摘を `@internal` / `@public` タグや knip の `ignore` 系の設定で隠さない。

## Error handling rules

### Return a `Result` instead of throwing

`errorHandling` in `eslint.config.js` bans `throw`/`try-catch` in production code and requires every returned `Result` to be consumed (`no-restricted-syntax`, `neverthrow/must-use-result` in `@fohte/eslint-config`). Return a `Result`/`ResultAsync` from [neverthrow](https://github.com/supermacro/neverthrow) instead (add it as a dependency first if this package doesn't have it yet):

```ts
// bad: throws
function parseConfig(raw: string): Config {
  if (!isValid(raw)) throw new Error('invalid config')
  return JSON.parse(raw)
}

// good: returns a Result
function parseConfig(raw: string): Result<Config, ConfigError> {
  if (!isValid(raw)) return err(new ConfigError('invalid config'))
  return ok(JSON.parse(raw))
}
```

Use `ResultAsync.fromPromise()` or `Result.fromThrowable()` to interop with a throwing API without a local try/catch. If the throw-based contract genuinely can't be wrapped that way, define a package-local base error and subclass it for the boundary:

```ts
abstract class BoundaryError extends Error {
  constructor(message: string, cause: unknown) {
    super(message, { cause })
    this.name = new.target.name
  }
}

class DataSourceBoundaryError extends BoundaryError {}
```

When `error_tracking` or `is_web_app` is enabled, call `captureWithFingerprint` from `@fohte/service-kit/observability` immediately before rethrowing to give the error a stable Sentry fingerprint.

Catch the exception, wrap it in the boundary-specific subclass, and rethrow it — `no-restricted-syntax` bans `try`/`throw` as separate selectors, so both the `try` and the `throw` need their own `eslint-disable-next-line no-restricted-syntax` comment explaining why.

## Storybook

### Write a story for every presentational component

Every presentational component under `src/components/` should have a co-located `.stories.tsx` file matching the component's filename (e.g. `src/components/card.tsx` pairs with `src/components/card.stories.tsx`). If a source file exports multiple components, give each one its own `<component-name>.stories.tsx` file instead of matching the source filename. Write one story per meaningful state/variant of the component.

### Give each story a descriptive `name`

Write a short, natural-language sentence describing the rendered state, including the detail that distinguishes it from sibling stories. Splitting the story's export name into words (e.g. `open editor shows next preview`) does not describe the state as a sentence; write something like `the editor shows the next preview` instead. The `name` is displayed in Storybook.

### A story is a prop-driven visual state, not a behavior test

A story's args fully determine what renders; the story never clicks, types, or otherwise interacts to reach that state. `fohte/no-play-in-stories` (`@fohte/eslint-config`) rejects a `play` function on a story or its `meta`, and rejects `parameters.screenshot.skip` — a story exists to document one rendered state from its args, and an interaction-driven story produces whatever the `play` function happens to leave on screen instead of that state.

For a state that would otherwise take interaction to reach — an open menu/popover/dialog, a field mid-edit — expose it through props (e.g. `open`/`defaultOpen`) instead of driving it with a `play` function, adding the prop or extracting a presentational subcomponent into a separate file if the component doesn't already support it. Test the interaction itself (click, type, assert) in a co-located `.test.tsx` file, not in a story.

### Prefer Storybook over manual browser checks

When you need to check how a component looks in a given state, write or update its story and view it via the `storybook` script (`storybook dev`) before starting a dev server and driving a browser manually.

## Test code rules

### No external command or system service dependencies

Tests must not depend on external commands (tmux, ps, terminal-notifier, git CLI, etc.) or system services (XPC, D-Bus, network). When testing production code that calls external commands, isolate the external call boundary so tests exercise logic without invoking real commands. Reason: tests run in srt sandbox / CI where external commands may be blocked or absent, causing hangs or failures.

### Assert on the whole output with a single equality check

Treat each test as a spec: build the expected output as one literal value (object, struct, JSON, array, etc.) and compare it to the actual output with a single equality assertion. Do not split the assertion into per-field checks, and do not use partial matchers (substring contains, `toContain`, `toMatchObject`, prefix/suffix checks, regex-on-substring, etc.). Partial matches silently ignore unexpected fields and extra elements, so the test stops working as a spec the moment the shape of the output changes.

```ts
// bad: picks fields one by one — silent on any new/changed field
const ev = run()
expect(ev.path).toBe('/a')
expect(ev.event).toBe('ok')
expect(ev.message).toContain('done')

// good: one literal, one equality — any drift in shape fails the test
expect(run()).toEqual({
  path: '/a',
  event: 'ok',
  message: 'done',
})
```

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
