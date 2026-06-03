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

## `cc watch` TUI

`a cc watch` launches a ratatui-based TUI with three top-level views:

- **Session view** (default): grouped list of Claude Code sessions.
- **Worktree view**: linked worktrees discovered under `wm.repos_root`, with session count and active-session marker overlaid.
- **Clean view**: reached by pressing `c` from session view or worktree view. Partitions the discovered worktrees into "To delete" (merged PR & no active session) and "Kept" (everything else). `Tab` does not enter or leave the clean view — it is reached only via `c` and exited via `Esc` / `n` / `q`.

PR statuses are fetched asynchronously when the clean view is entered (batched GraphQL via `GitHubClient::get_prs_for_branches_batch`); the result is shown after a brief "Loading PR status..." banner. Inside the clean view, `Enter` toggles the selected row between sections so the user can force-include an active worktree (override the default protection) or exclude a merged one.

Pressing `y` confirms the partition: the watch process generates a `run_id` and spawns `a cc clean-detached --run-id <id>` as a **fully detached child** (`setsid`, stdio routed to `/dev/null`) so closing `cc watch` does not abort the cleanup. The child journals each event (`cc.clean.start` / `cc.clean.ok` / `cc.clean.err` / `cc.clean.done`) into the shared rotating tracing log at `~/.cache/armyknife/logs/armyknife.log.YYYY-MM-DD` under a `run_id` span. While `cc watch` is alive, it tails today's log file every 500 ms, filters lines by `run_id`, and renders `Cleaning... (i/N) <path>` (with `(N error)` when any failure has been observed) in the bottom bar; on completion it shows `Cleaned X, failed Y` until the next key press.

## Internal Subcommands

Subcommands marked with `#[command(hide = true)]` are not user-facing entry points; they exist as spawn targets for other commands and are listed here for discoverability.

| Command               | Spawned by   | Purpose                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                          |
| --------------------- | ------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `a cc clean-detached` | `a cc watch` | Non-interactive batch worktree cleanup. Reads paths from argv or `--paths-file`, runs `cleanup_worktree_resources` per path, and journals each step (`cc.clean.start` / `ok` / `err` / `done`) into the shared tracing log under a `run_id` span (`--run-id` is passed by the caller so it can later filter the log). Never reads stdin; never writes stdout/stderr. The caller is responsible for detaching the process (`nohup`/`setsid`). Retention is handled by the shared 7-day rotation in `shared::log`. |
