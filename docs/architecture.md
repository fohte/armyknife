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

Pressing `y` confirms the partition: the watch process spawns `a cc clean-detached` as a **fully detached child** (`setsid`, stdio redirected to `/dev/null`) so closing `cc watch` does not abort the cleanup. The child writes per-PID JSONL progress to `~/.cache/armyknife/clean/<pid>.jsonl`; while `cc watch` is alive, it tails that file every 500 ms and shows progress in the bottom bar of session / worktree view (`Cleaning... (i/N) <path>`). After completion, the bottom bar displays `Cleaned X, failed Y` until the next key press.

On `cc watch` startup, the log directory is GC'd (entries older than 7 days are removed) and, if any prior run's `Done` event is on disk, a one-shot "Last clean: N ok, M failed" banner is shown until the user's next key press; the consumed log file is deleted.

## Internal Subcommands

Subcommands marked with `#[command(hide = true)]` are not user-facing entry points; they exist as spawn targets for other commands and are listed here for discoverability.

| Command               | Spawned by   | Purpose                                                                                                                                                                                                                                                                                                                                                                              |
| --------------------- | ------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `a cc clean-detached` | `a cc watch` | Non-interactive batch worktree cleanup. Reads paths from argv or `--paths-file`, runs `cleanup_worktree_resources` per path, and journals progress as JSONL under `~/.cache/armyknife/clean/<pid>.jsonl`. Never reads stdin; never writes stdout/stderr. The caller is responsible for detaching the process (`nohup`/`setsid`). Logs older than 7 days are GC'd on each invocation. |
