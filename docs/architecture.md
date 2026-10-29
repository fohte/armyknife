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
| `a agent new`          | `agent`       | `new`             |
| `a ai pr-draft submit` | `ai pr-draft` | `submit`          |
| `a gh check-pr-review` | `gh`          | `check-pr-review` |

### Naming Convention

- **Scope**: Noun or abbreviation representing the domain (e.g., `ai`, `agent`, `gh`)
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

## `agent watch` TUI

`a agent watch` launches a ratatui-based TUI with a session view and a clean view:

- **Session view** (default): TASKS session list with a PROGRESS pane.
- **Clean view**: reached by pressing `c` from session view. Partitions the discovered worktrees into "To delete" (merged PR & no active session) and "Kept" (everything else); it exits via `Esc` / `n` / `q`.

At sufficient terminal width, the session view renders the TASKS list and PROGRESS side by side. `Tab` switches focus between TASKS and PROGRESS; narrow terminals show only the focused pane. A disk snapshot supplies task names and numbers while the TUI refreshes tq data in the background.

PROGRESS follows the selected TASKS session. When tq data is available, a task-linked session shows all sessions linked to that task, descendant delegates, and the task's `実装` checklist. The selected session's work row stays in its task's stable order and remains emphasized when the cursor moves to another row. Project-manager sessions show only their own work row. Other sessions without a task link or tq data keep the selected session's direct delegates. Delegate PR lookup uses saved repository and branch context when present and resolves the live worktree only when saved context is absent; saved context also works after the worktree is removed. A delegate row says `(PR unavailable)` when neither saved context nor the live worktree can resolve a PR. Ended delegates without PR information are hidden. Stage marks are `✓` for done, `◐` for user attention, `●` for active, `◷` for waiting, and `○` for not started or unknown. An existing PR marks implementation, diff review, and PR body done unless a linked review is active; otherwise implementation follows the delegate's session state. Linked review activity updates the diff-review and PR-body marks. CI follows the latest commit's check rollup; merge follows PR state and CI progress. On a heading, `Enter` toggles it; on a session row, `Enter` focuses its pane and does nothing if no pane is available. Press `o` on a PR row to open the PR in the browser; it does nothing when the row has no PR.

At the model layer, `ProgressRow` keeps work sessions, descendant delegates, and implementation checklist entries in one ordered list. Task scope comes from tq links and delegate ancestor IDs. Selection is tracked by row key so the selected session can stay emphasized without changing the order. The plain and checklist views derive their visible rows from this shared list.

In the checklist view, unmatched delegates appear under a collapsible `チェックリスト外` heading. Unstarted entries without a delegate or PR link appear as non-selectable rows, and fully merged batches start collapsed. Use `j` and `k` to select the implementation, a Batch heading, or `チェックリスト外`. On a heading, press `h` to collapse it, `l` to expand it, or `Enter` to toggle it. Press `h` on a PR row to collapse its Batch or `チェックリスト外` group and select that heading.

Worktrees are discovered in the background and their PR statuses are fetched asynchronously when the clean view is entered (batched GraphQL via `GitHubClient::get_prs_for_branches_batch`); the result is shown after a brief "Loading PR status..." banner. Each worktree row in the clean view also shows its nested Claude Code sessions as tree children. `Enter` dispatches by row type: on a worktree row it toggles the section so the user can force-include an active worktree or exclude a merged one; on a nested session row it focuses that session's tmux pane.

Pressing `y` confirms the partition: the watch process generates a `run_id` and spawns `a agent clean-detached --run-id <id>` as a **fully detached child** (`setsid`, stdio routed to `/dev/null`) so closing `agent watch` does not abort the cleanup. The child journals each event (`agent.clean.start` / `agent.clean.ok` / `agent.clean.err` / `agent.clean.done`) into the shared rotating tracing log at `~/.cache/armyknife/logs/armyknife.log.YYYY-MM-DD` under a `run_id` span. While `agent watch` is alive, it tails today's log file every 500 ms, filters lines by `run_id`, and renders `Cleaning... (i/N) <path>` (with `(N error)` when any failure has been observed) in the bottom bar; on completion it shows `Cleaned X, failed Y` until the next key press.

## Internal Subcommands

Subcommands marked with `#[command(hide = true)]` are not user-facing entry points; they exist as spawn targets for other commands and are listed here for discoverability.

| Command                  | Spawned by      | Purpose                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                            |
| ------------------------ | --------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `a agent clean-detached` | `a agent watch` | Non-interactive batch worktree cleanup. Reads paths from argv or `--paths-file`; for each path, notifies the delegate session's delegator if the branch's PR merged, then runs `cleanup_worktree_resources` and starts `post-worktree-delete` after each successful removal. It journals each step (`agent.clean.start` / `ok` / `err` / `done`) into the shared tracing log under a `run_id` span (`--run-id` is passed by the caller so it can later filter the log). Never reads stdin; never writes stdout/stderr. The caller is responsible for detaching the process (`nohup`/`setsid`). Retention is handled by the shared 7-day rotation in `shared::log`. |
