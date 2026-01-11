# Architecture

This document describes the design principles and conventions for armyknife.

## Command Hierarchy

### Top-level Commands

Direct user actions that don't require subcommands.

- `update`: Self-update the CLI

### `ai` Subcommands

Commands designed for AI agents (e.g., Claude Code) to call programmatically.

These are NOT meant to be called directly by humans. Instead, AI agents use these during automated workflows (e.g., via Claude Code skills).

## Naming Convention

### Pattern: Noun + Verb

`ai` subcommands follow a noun-verb pattern:

```
a ai <noun> <verb>
```

| Level | Pattern         | Example           |
| ----- | --------------- | ----------------- |
| Noun  | `a ai <noun>`   | `a ai pr-draft`   |
| Verb  | `<noun> <verb>` | `pr-draft submit` |

### Rationale

- **Noun** represents the resource being managed (e.g., `pr-draft`)
- **Verb** represents the action to perform (e.g., `new`, `review`, `submit`)
- This allows multiple verbs per noun, keeping related actions grouped

### Examples

```sh
# pr-draft: Manage PR body drafts
a ai pr-draft new       # Create a new draft
a ai pr-draft review    # Open draft for human review
a ai pr-draft submit    # Submit as a PR

# Future example: branch-name generation
a ai gen new --branch   # Generate a branch name (hypothetical)
```

## Module Structure

Code is organized by subcommand:

```
src/
├── ai/
│   ├── mod.rs              # AiCommands enum
│   └── pr_draft/
│       ├── mod.rs          # PrDraftCommands enum
│       ├── new.rs          # `new` action
│       ├── review.rs       # `review` action
│       └── submit.rs       # `submit` action
├── cli.rs                  # Top-level CLI definition
└── main.rs                 # Entry point
```

When adding a new `ai` subcommand:

1. Create a new directory under `src/ai/` (e.g., `src/ai/gen/`)
2. Define the subcommand enum in `mod.rs`
3. Add each action as a separate module
4. Register in `src/ai/mod.rs`
