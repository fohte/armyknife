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

`a agent watch` launches a ratatui-based TUI with a session list and a clean view:

- **Session view** (default): grouped list of Claude Code sessions.
- **Clean view**: reached by pressing `c` from session view. Partitions the discovered worktrees into "To delete" (merged PR & no active session) and "Kept" (everything else); it exits via `Esc` / `n` / `q`.

At sufficient terminal width, the session view renders a tq project/task sidebar beside the existing status-grouped list. The sidebar filters by a task and its descendants, a project, or sessions without task links; `Tab` switches focus and `C-b` toggles visibility. A disk snapshot supplies the initial tree while the TUI refreshes tq data in the background.

Worktrees are discovered in the background and their PR statuses are fetched asynchronously when the clean view is entered (batched GraphQL via `GitHubClient::get_prs_for_branches_batch`); the result is shown after a brief "Loading PR status..." banner. Each worktree row in the clean view also shows its nested Claude Code sessions as tree children. `Enter` dispatches by row type: on a worktree row it toggles the section so the user can force-include an active worktree or exclude a merged one; on a nested session row it focuses that session's tmux pane.

Pressing `y` confirms the partition: the watch process generates a `run_id` and spawns `a agent clean-detached --run-id <id>` as a **fully detached child** (`setsid`, stdio routed to `/dev/null`) so closing `agent watch` does not abort the cleanup. The child journals each event (`agent.clean.start` / `agent.clean.ok` / `agent.clean.err` / `agent.clean.done`) into the shared rotating tracing log at `~/.cache/armyknife/logs/armyknife.log.YYYY-MM-DD` under a `run_id` span. While `agent watch` is alive, it tails today's log file every 500 ms, filters lines by `run_id`, and renders `Cleaning... (i/N) <path>` (with `(N error)` when any failure has been observed) in the bottom bar; on completion it shows `Cleaned X, failed Y` until the next key press.

## Internal Subcommands

Subcommands marked with `#[command(hide = true)]` are not user-facing entry points; they exist as spawn targets for other commands and are listed here for discoverability.

| Command                                  | Spawned by                       | Purpose                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                  |
| ---------------------------------------- | -------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `a agent clean-detached`                 | `a agent watch`                  | Non-interactive batch worktree cleanup. Reads paths from argv or `--paths-file`; for each path, notifies the delegate session's delegator if the branch's PR merged, then runs `cleanup_worktree_resources`. After processing paths, it checks each repository containing a merged worktree once for conflicts between surviving branches and fetched `origin/main` or `origin/master`; sessions whose working directory is in a conflicting worktree are notified unless their status is `Ended`. It journals each step (`agent.clean.start` / `ok` / `err` / `done`) into the shared tracing log under a `run_id` span (`--run-id` is passed by the caller so it can later filter the log). Never reads stdin; never writes stdout/stderr. The caller is responsible for detaching the process (`nohup`/`setsid`). Retention is handled by the shared 7-day rotation in `shared::log`. |
| `a agent notify-base-conflicts-detached` | `a agent close`, `a agent clean` | Hidden worker that fetches `origin`, checks surviving worktree branches against the repository's main branch with `git merge-tree`, and notifies sessions in conflicting worktrees to run the `sync-base-branch` skill. The caller excludes worktree paths being removed. Never reads stdin or writes stdout/stderr.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                     |
