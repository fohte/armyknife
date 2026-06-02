# Architecture

This document describes the design principles and conventions for armyknife.

## Command Design

Commands follow the pattern:

```
a [<scope>...] <action>
```

- **Scope**: Optional, can be nested to group related commands
- **Action**: The verb representing what to do

### Examples

| Command                | Scope         | Action            |
| ---------------------- | ------------- | ----------------- |
| `a update`             | (none)        | `update`          |
| `a wm new`             | `wm`          | `new`             |
| `a ai pr-draft submit` | `ai pr-draft` | `submit`          |
| `a gh check-pr-review` | `gh`          | `check-pr-review` |

### Naming Convention

- **Scope**: Noun or abbreviation representing the domain (e.g., `ai`, `wm`, `gh`)
- **Action**: Verb representing what to do (e.g., `new`, `submit`, `update`)

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

Shared modules are extracted when reusable (e.g., `human_in_the_loop/`).

## Internal Subcommands

Subcommands marked with `#[command(hide = true)]` are not user-facing entry points; they exist as spawn targets for other commands and are listed here for discoverability.

| Command               | Spawned by   | Purpose                                                                                                                                                                                                                                                                                                                                                                              |
| --------------------- | ------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `a cc clean-detached` | `a cc watch` | Non-interactive batch worktree cleanup. Reads paths from argv or `--paths-file`, runs `cleanup_worktree_resources` per path, and journals progress as JSONL under `~/.cache/armyknife/clean/<pid>.jsonl`. Never reads stdin; never writes stdout/stderr. The caller is responsible for detaching the process (`nohup`/`setsid`). Logs older than 7 days are GC'd on each invocation. |
