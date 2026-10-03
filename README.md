# armyknife

[![GitHub release](https://img.shields.io/github/v/release/fohte/armyknife)](https://github.com/fohte/armyknife/releases/latest)
[![codecov](https://codecov.io/gh/fohte/armyknife/graph/badge.svg)](https://codecov.io/gh/fohte/armyknife)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)

Personal CLI toolkit written in Rust

## Installation

### Pre-built binaries

Download from [GitHub Releases](https://github.com/fohte/armyknife/releases/latest).

Available for:

- macOS (Apple Silicon)
- Linux (x86_64, aarch64)

### Build from source

```sh
cargo install --git https://github.com/fohte/armyknife
```

## Usage

```sh
a <command>
```

## Configuration

armyknife reads every `*.yaml` and `*.yml` file directly under `~/.config/armyknife/` (or `$XDG_CONFIG_HOME/armyknife/` if set), sorts them alphabetically by file name, and deep-merges them in order so that later files override earlier ones. Subdirectories (e.g., `hooks/`) and other extensions are ignored. Symlinks pointing to YAML files are followed, so private/company-specific config can live in a separate repository and be linked into this directory.

Mapping keys are merged recursively; sequences and scalars are replaced wholesale by later files. All fields are optional and fall back to sensible defaults. If no config files exist and no `ARMYKNIFE_*` environment variable overrides are set (see [Environment variable overrides](#environment-variable-overrides)), armyknife runs entirely on defaults.

Unknown configuration keys fail parsing. Put worktree settings under `agent.worktree`, pause settings under `agent.auto_pause`, and compaction settings under `agent.auto_compact`.

For editor autocompletion, add the following to the top of your config file:

```yaml
# yaml-language-server: $schema=https://raw.githubusercontent.com/fohte/armyknife/master/docs/config-schema.json
```

### Example

```yaml
# yaml-language-server: $schema=https://raw.githubusercontent.com/fohte/armyknife/master/docs/config-schema.json

agent:
  work_types: # per-skill icon and color metadata
    example-workflow:
      icon: '◇'
      color: cyan
  default_engine: claude # coding agent CLI for `a agent new` when `--engine` is omitted (default: "claude")
  codex: # defaults for `a agent new --engine codex` only; the `codex` you run yourself is unaffected
    model: gpt-5.6-luna # used when `--model` is omitted
    reasoning_effort: max # low | medium | high | xhigh | max; applied when `--reasoning-effort` is omitted
  worktree:
    dir: .worktrees # worktree directory name (default: ".worktrees")
    branch_prefix: fohte/ # branch name prefix for `a agent new --worktree` (default: "fohte/")
    repos_root: ~/ghq # root directory for repo discovery in `a agent clean --all` (default: GHQ_ROOT or ghq.root or ~/ghq)
    layout: # tmux pane layout for `a agent new --worktree`
      direction: horizontal
      first:
        command: nvim
        focus: true
      second:
        command: claude

editor:
  terminal: ghostty # terminal emulator: "wezterm" (default) or "ghostty"
  editor_command: nvim # editor for human-in-the-loop reviews (default: "nvim")
  focus_app: Ghostty # app to focus on notification click, macOS only (default: derived from terminal)

notification:
  enabled: true # enable desktop notifications (default: true)
  sound: Glass # notification sound name, empty string for silent (default: "Glass")

orgs: # per-org defaults, keyed by GitHub owner (org or user)
  fohte:
    ai:
      review:
        reviewers: [coderabbit] # default reviewers for `a ai review wait`/`request` in this org

repos: # per-repository overrides, keyed by "owner/repo"
  fohte/dotfiles:
    language: en # language for commit messages and PR content (default: "ja" for private repos, "en" for public repos)
    direct_commit: true # allow direct commits to the default branch; consumed by external git hooks
    ai:
      review:
        reviewers: [devin, coderabbit] # repo-level reviewer override (takes precedence over org)
```

`agent.work_types` associates exact skill names with icon and color metadata. Hooks record a configured skill name in session metadata when it is invoked; this mapping does not itself add icons to the session list. Colors accept the snake_case names `reset`, `black`, `red`, `green`, `yellow`, `blue`, `magenta`, `cyan`, `gray`, `dark_gray`, `light_red`, `light_green`, `light_yellow`, `light_blue`, `light_magenta`, `light_cyan`, and `white`, RGB triplets (`[r, g, b]`), or numeric indices from `0` through `255`.

### Splitting public and private config

Because every YAML file in the directory is merged, you can keep public (dotfiles-tracked) and private (company-only) config separate. For example, drop a single file from a private repository as a symlink:

```sh
ln -s ~/work/dotfiles-private/armyknife.yaml ~/.config/armyknife/work.yaml
```

`config.yaml` is loaded first (alphabetical), `work.yaml` overrides it. Subdirectories such as `hooks/` are not scanned and remain unaffected.

### Environment variable overrides

Any scalar config value (string, bool, number) can also be set via an `ARMYKNIFE_*` environment variable, which takes priority over every YAML file. Strip the `ARMYKNIFE_` prefix, lowercase what remains, and join the config key path with `__` (double underscore, since key names themselves contain `_`). For example:

```sh
ARMYKNIFE_AGENT__AUTO_COMPACT__ENABLED=false
```

maps to `agent.auto_compact.enabled`. Values are parsed as YAML scalars, so `false` becomes a bool and `3` a number. List- or map-typed fields (e.g. `reviewers`) can't be overridden this way, since env values are always scalars.

Variables whose path has no `__` are ignored rather than treated as a config key — every config field lives under a top-level section, so a bare `ARMYKNIFE_<NAME>` can never resolve to a real value. This also keeps unrelated `ARMYKNIFE_*` variables (session tracking, hook context, etc.) from being misread as config overrides. `repos.*` entries aren't reachable this way, since repo keys contain `/`, which can't appear in an environment variable name. `orgs.*` entries aren't reachable either, since org logins are matched case-sensitively but the overlay lowercases every path segment.

### Supported Terminal Emulators

The `editor.terminal` setting selects which terminal emulator opens for human-in-the-loop reviews. Each terminal has built-in support for window size and title options.

| `terminal` value | Terminal          |
| ---------------- | ----------------- |
| `wezterm`        | WezTerm (default) |
| `ghostty`        | Ghostty           |

When a review command runs inside tmux, the editor opens in a floating pane over the invoking pane and uses the review title as its pane title. This requires tmux 3.7 or later; on older versions, the review command fails. Run it outside tmux to use the configured `editor.terminal` window.

## Commands

### `a update`

Update to the latest version.

The CLI automatically checks for updates and prompts you to update when a new version is available.

When a release asset for a published target is still uploading, `a update` retries every 10 seconds with an authenticated GitHub token or every 60 seconds anonymously, for up to 30 minutes.

Version checks and release downloads hit the GitHub REST API, which limits anonymous requests to 60 per hour per IP. To authenticate and raise that limit, `a update` picks the first non-empty token from the following sources:

1. `ARMYKNIFE_GITHUB_TOKEN` environment variable
2. `GITHUB_TOKEN` environment variable
3. `GH_TOKEN` environment variable
4. `gh auth token` output (when `gh` is installed and signed in)

When none is available the request falls back to anonymous access.

### `a name-branch <description>`

Generate a branch name from a description using AI.

### `a ai`

Commands designed for AI agents (e.g., Claude Code) to call programmatically.
These provide structured inputs/outputs suitable for AI workflows.

#### `a ai draft <path>`

Open a file in editor for review (no approval flow). After the editor exits, a unified diff between the pre-edit and post-edit contents is written to stdout (or `(no edits)` when unchanged), letting the caller see what the human edited without re-reading the file. The same stdout contract applies to every editor-backed review command (`a ai pr-draft review`, `a gh issue-agent review`, `a gh pr-review reply review`).

| Option            | Description                                                |
| ----------------- | ---------------------------------------------------------- |
| `--title <title>` | Window title for WezTerm (defaults to "Draft: <filename>") |

#### `a ai pr-draft`

Manage PR body drafts with human-in-the-loop review.

| Action   | Description                                                                                                                    |
| -------- | ------------------------------------------------------------------------------------------------------------------------------ |
| `new`    | Create a new PR body draft file                                                                                                |
| `review` | Open the draft in editor for review (blocks until editor closes, exits 0 if steps changed, 1 if not, 2 if editor already open) |
| `submit` | Create a PR from the approved draft (updates existing PR if found)                                                             |

`new` options:

| Option              | Description                                                                                              |
| ------------------- | -------------------------------------------------------------------------------------------------------- |
| `--title <title>`   | PR title                                                                                                 |
| `--force`           | Overwrite existing draft file if it exists                                                               |
| `-R, --repo <repo>` | Target repository (owner/repo) instead of cwd's. Requires `--branch`                                     |
| `--branch <name>`   | Target branch instead of cwd's current branch (can be used alone to target another branch in cwd's repo) |

`submit` options:

| Option         | Description            |
| -------------- | ---------------------- |
| `--base <ref>` | Base branch for the PR |
| `--draft`      | Create as draft PR     |

#### `a ai review`

Request or wait for bot reviews on a PR.

| Command   | Description                                                           |
| --------- | --------------------------------------------------------------------- |
| `request` | Request a review from a bot reviewer and wait for completion          |
| `wait`    | Wait for an existing review to complete (does not trigger new review) |

| Option                  | Description                                                                                                                       |
| ----------------------- | --------------------------------------------------------------------------------------------------------------------------------- |
| `-R, --repo <repo>`     | Target repository (owner/repo)                                                                                                    |
| `-r, --reviewer <name>` | Reviewer(s) to request/wait for; can be repeated (`devin`, `coderabbit`). When omitted, falls back to repo > org config > `devin` |
| `--interval <seconds>`  | Polling interval (default: 15)                                                                                                    |
| `--timeout <seconds>`   | Timeout (default: 300)                                                                                                            |

When `--reviewer` is omitted, the reviewer set is resolved in this order:

1. `repos.<owner>/<repo>.ai.review.reviewers` (per-repo override)
2. `orgs.<owner>.ai.review.reviewers` (per-org default)
3. The built-in `[devin]`

This makes it possible to disable a reviewer that isn't enabled in a given org (e.g., `reviewers: [coderabbit]` for a fohte-only repo) without passing `--reviewer` on every invocation.

### `a gh`

GitHub-related utilities.

#### `a gh issue-agent`

Manage GitHub Issues as local files for AI agents.

```sh
a gh issue-agent <command> <issue-number> [options]
```

| Command  | Description                                                                                          |
| -------- | ---------------------------------------------------------------------------------------------------- |
| `view`   | View issue and comments (read-only, no local cache)                                                  |
| `pull`   | Fetch issue and save locally                                                                         |
| `review` | Review a file before pushing (opens editor, exits 0 if approved, 1 if not, 2 if editor already open) |
| `push`   | Push local changes to GitHub (requires file approval via review)                                     |
| `diff`   | Show colored diff between local changes and remote                                                   |
| `init`   | Create boilerplate files for new issues or comments                                                  |

| Option           | Description                                        |
| ---------------- | -------------------------------------------------- |
| `-R <repo>`      | Target repository (default: current repo)          |
| `--dry-run`      | Show what would be changed without applying        |
| `--force`        | Overwrite local/remote changes (context-dependent) |
| `--edit-others`  | Allow editing other users' comments                |
| `--allow-delete` | Allow deleting comments removed locally            |

##### `a gh issue-agent init issue`

Create a new issue boilerplate file. Fetches issue templates from the repository if available.

| Option              | Description                                       |
| ------------------- | ------------------------------------------------- |
| `--list-templates`  | List available issue templates and exit           |
| `--template <NAME>` | Use a specific issue template by name             |
| `--no-template`     | Use default boilerplate (skip template selection) |

The frontmatter accepts the same editable keys as pulled issues, so a new issue can declare `parentIssue` and `subIssues` and `push` will create the issue and link it via the Sub-issues API in one step:

```yaml
---
title: Child Issue
parentIssue: owner/repo#1
subIssues:
  - owner/repo#10
---
```

Unknown frontmatter keys (e.g. `parentIssues` typo) are rejected with an error rather than silently ignored.

##### `a gh issue-agent init comment <issue-number>`

Create a new comment boilerplate file for an existing issue.

| Option          | Description                                    |
| --------------- | ---------------------------------------------- |
| `--name <NAME>` | Name for the comment file (default: timestamp) |

#### `a gh pr-review`

PR review workflow commands.

##### `a gh pr-review check`

Fetch PR review comments in a concise format for AI agents.

```sh
a gh pr-review check <pr-number> [options]
```

| Option           | Description                                                                                            |
| ---------------- | ------------------------------------------------------------------------------------------------------ |
| `--review <n>`   | Show details for a specific review                                                                     |
| `--full`         | Show full details for all reviews                                                                      |
| `-a, --all`      | Include resolved threads                                                                               |
| `--open-details` | Expand `<details>` blocks in comments (HTML comments and `<picture>` badge blocks are always stripped) |

> `a gh check-pr-review` is a deprecated alias for `a gh pr-review check`.

##### `a gh pr-review reply pull`

Fetch review threads to a local Markdown file for editing.

```sh
a gh pr-review reply pull <pr-number> [options]
```

| Option               | Description                                 |
| -------------------- | ------------------------------------------- |
| `-R, --repo <REPO>`  | Target repository (owner/repo)              |
| `--include-resolved` | Include resolved threads                    |
| `--force`            | Overwrite local changes without checking    |
| `-d, --open-details` | Expand `<details>` blocks in comment bodies |

The Markdown file compresses each diff hunk to the lines surrounding the
commented region (5 before, 3 after). HTML comments and `<picture>` badge
blocks (commonly emitted by review bots) are stripped from comment bodies, and
`<details>` blocks are collapsed to a single-line marker by default; pass
`--open-details` to keep them expanded.

##### `a gh pr-review reply push`

Push draft replies and resolve actions from the local Markdown file to GitHub.

```sh
a gh pr-review reply push <pr-number> [options]
```

| Option              | Description                      |
| ------------------- | -------------------------------- |
| `-R, --repo <REPO>` | Target repository (owner/repo)   |
| `--dry-run`         | Preview changes without applying |
| `--force`           | Force push even with conflicts   |

##### `a gh pr-review reply review`

Open the local threads.md in an editor for review. Setting `submit: true` in the frontmatter and saving marks the replies as approved. Run `reply push` afterwards to push. Exits 0 if approved, 1 if not, 2 if editor already open.

Requires `reply pull` to have been run first.

```sh
a gh pr-review reply review <pr-number> [options]
```

| Option              | Description                    |
| ------------------- | ------------------------------ |
| `-R, --repo <REPO>` | Target repository (owner/repo) |

### `a agent`

Claude Code session monitoring with tmux integration. The canonical command is `a agent` (alias `a ag`); `a cc` is kept as a hidden backward-compatible alias, so existing hook and tmux configs that invoke `a cc ...` keep working unchanged.

| Action                                           | Aliases | Description                                                               |
| ------------------------------------------------ | ------- | ------------------------------------------------------------------------- |
| `new [--worktree[=<branch>]] [options]`          |         | Start a Claude Code session, optionally in a new worktree                 |
| `codex [<args>...]`                              |         | Start Codex and bind its thread ID to the current tmux pane               |
| `close [target] [--force] [--skip-hooks]`        | `c`     | Close an agent session and its linked worktree                            |
| `clean [--dry-run] [--all] [--force]`            |         | Delete merged or closed worktrees                                         |
| `hook <event>`                                   |         | Record session events (called from Claude Code hooks)                     |
| `list`                                           | `ls`    | List all Claude Code sessions with status                                 |
| `focus <session_id>`                             |         | Focus on a session's tmux pane                                            |
| `mark-read [-t <pane_id>]`                       |         | Mark the pane's session as read (wire from tmux `pane-focus-in`)          |
| `resume [session_id]`                            | `r`     | Resume the pane's Claude Code session (reads pane option if no argument)  |
| `resurrect save`                                 |         | Save pane session IDs for tmux-resurrect (run from post-save hook)        |
| `resurrect restore`                              |         | Restore pane session IDs and relaunch Claude Code (from post-restore)     |
| `peer parent`                                    |         | List the session that delegated to this one, if any (JSON)                |
| `peer children`                                  |         | List the sessions this one delegated to (JSON)                            |
| `peer list [-R <repo>]`                          |         | List tracked sessions, with their SendMessage names (JSON)                |
| `peer me`                                        |         | Print the session running in the caller's own tmux pane (JSON)            |
| `peer wake <session_id>`                         |         | Resume a paused peer session; print its SendMessage name (Claude only)    |
| `peer notify <session_id> -m <text>`             |         | Send a message to another session (SendMessage socket / Codex app-server) |
| `crit add <url>`                                 |         | Associate a crit review with the calling session                          |
| `crit open [--session <id> \| --pane <pane_id>]` |         | Toggle the latest associated review in a tmux floating pane               |
| `bg run -- <cmd> [args...]`                      |         | Run a command detached and notify this session when it finishes           |
| `sweep`                                          |         | Pause long-stopped sessions (run periodically or manual)                  |
| `auto-compact schedule --session <id>`           |         | Detached worker spawned by the Stop hook (not for direct use)             |
| `window-status <window_id>`                      |         | Print status symbols for the sessions in a tmux window                    |
| `pane-has-paused <pane_id>`                      |         | Print `1` when the pane holds a Paused Claude Code session, else empty    |

[`crit`](https://github.com/tomasz-tomczyk/crit) review URLs passed to `a agent crit add <url>` must include an explicit port. When a tracked session is available, the command associates the URL with that session and sends a desktop notification. Its lifecycle monitor needs `crit` on `PATH`; without it, automatic cleanup is unavailable. If no tracked session ID is available, the command opens a review pane in the current tmux pane when `TMUX_PANE` is set; otherwise, macOS opens the URL with `open` and other platforms use `xdg-open`.

`a agent crit open` toggles the latest associated review in a tmux floating pane. It requires tmux 3.7 or later, `shpool`, and `terminal-browser` on `PATH`. In `a agent watch`, sessions with a review show a `[crit]` badge; press `o` to close watch and toggle the review pane. `crit add` mirrors the latest URL to the pane option `@crit` and re-runs the `window-layout-changed` hook, allowing tmux configuration to show a review border:

```sh
a agent crit add http://127.0.0.1:12345/review
a agent watch
```

```tmux
set -g pane-border-format '#{?#{@crit},crit review,#{pane_index}}'
```

On wide terminals, `a agent watch` shows a tq task sidebar beside the session list. Use `C-b` to show or hide the sidebar, and `Enter` to apply a task or project filter. Task filters include descendant tasks; the **タスクなし** row shows sessions without linked tq tasks. Moving either the sidebar cursor or session-list cursor highlights matching rows in the other pane. `Tab` switches focus and moves the other pane's cursor to its first matching row. When the sidebar cursor is **すべて**, `Tab` moves the session-list cursor to its first session; if a matching session is hidden by the current sidebar filter, `Tab` applies the cursor's filter first. Press `r` while the sidebar has focus to refresh tq data. On narrow terminals, the sidebar and session list use separate screens: press `Enter` on a sidebar row to show its matching sessions, then press `Esc` to return to the task or **タスクなし** row linked to the selected session. `Esc` clears search, status, or drill-down filters before returning. At startup in the narrow layout, `ARMYKNIFE_FOCUS_SESSION` opens the session list first; otherwise the sidebar is shown.

`a agent bg run -- <cmd> [args...]` returns immediately and runs the command in a detached worker. It stores stdout and stderr in separate files and prints their paths. On completion, it sends this session a `<background-task-complete>` message with the command, exit code, and output paths. While the worker is active and the main loop is stopped, `a agent list`, `a agent watch`, and tmux window status show `◎ background`, or `◐ waiting` while a crit review is linked or a Human-in-the-Loop review is waiting for approval. Stop notifications and auto-compaction are suppressed, and `a agent sweep` leaves the session alone. `a agent clean` also preserves its worktree unless `--force` is set.

Run this command inside a tracked Claude Code or Codex session. The session must have an armyknife session record and expose `ARMYKNIFE_SESSION_ID` or `CODEX_SESSION_ID`. Paused sessions are resumed before delivery. Notifications are best-effort: Codex may queue a message until its thread is idle, and an ended session is not resumed. Output files are written under the system temporary directory and may be removed by the OS.

```console
$ a agent bg run -- printf 'ready\n'
Started background task <task-id>
stdout: <system-temp-dir>/armyknife-agent-bg-<task-id>/<task-id>.stdout
stderr: <system-temp-dir>/armyknife-agent-bg-<task-id>/<task-id>.stderr
```

The completion message has this form:

```text
<background-task-complete>
- Task ID: <task-id>
- Command argv: ["printf", "ready\\n"]
- Exit code: 0
- stdout: "<system-temp-dir>/armyknife-agent-bg-<task-id>/<task-id>.stdout"
- stderr: "<system-temp-dir>/armyknife-agent-bg-<task-id>/<task-id>.stderr"
</background-task-complete>
```

`a agent codex [codex args...]` connects to the shared Codex app-server before launching Codex, then records the new thread ID in the current tmux pane's `@armyknife-last-agent-session-id` option. This lets `a agent resume` find the session after Codex exits. Outside tmux, or when the app-server is unavailable, it launches Codex without pane binding. Concurrent launches in the same directory are serialized; a launch that cannot acquire the lock within one minute exits with an error.

`a agent close [target] [--force] [--skip-hooks]` (alias: `a ag c`) accepts a session ID, worktree name (branch name), or worktree path. With no target, it closes the session in the current pane; if the pane has no tracked session, it uses the current linked worktree. Closing a session in a linked worktree also removes that worktree and its branch. A worktree target with multiple tracked sessions is ambiguous; pass a session ID to select one.

The command sends Ctrl+D and waits up to five seconds for the agent to exit, then sends SIGTERM if needed. It removes the pane after confirming the agent has exited. Unless `--force` is passed, it refuses sessions that are running, waiting for input, have pending tasks, or contain an unsent draft or a draft that cannot be checked. It also refuses when it cannot find the agent process for a session that has not ended or paused, or when the pane is no longer bound to that session. For worktree cleanup, `--force` also skips the unmerged-branch confirmation, and `--skip-hooks` skips the `pre-worktree-delete` and `post-worktree-delete` hooks. An already-removed pane is treated as closed.

`new` options:

| Option                                               | Description                                                                                                                                                                                                                                                                                                                                                                                                              |
| ---------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `--worktree[=<branch>]`                              | Create a worktree for the branch and run the session there, opening a new tmux window. Branch is auto-generated from `--prompt` when the value is omitted; opens `$EDITOR` to write a prompt when both are omitted. When the flag itself is omitted, no worktree is created: the session runs in the current directory (or the repo root of `-R`) -- see below for whether that starts a pane split or a new tmux window |
| `--from <ref>`                                       | Base branch for new branch creation (requires `--worktree`; default: origin/main or origin/master)                                                                                                                                                                                                                                                                                                                       |
| `--force`                                            | Force create new branch even if it already exists (requires `--worktree`)                                                                                                                                                                                                                                                                                                                                                |
| `--skip-hooks`                                       | Skip the post-worktree-create hook (requires `--worktree`)                                                                                                                                                                                                                                                                                                                                                               |
| `-R, --repo <path>`                                  | Target repository path (default: current directory)                                                                                                                                                                                                                                                                                                                                                                      |
| `--prompt <text>`                                    | Initial prompt to send to the agent (Claude Code or Codex)                                                                                                                                                                                                                                                                                                                                                               |
| `--agent`                                            | Mark this invocation as coming from another Claude Code session (wraps prompt with delegation context)                                                                                                                                                                                                                                                                                                                   |
| `--label <title>`                                    | Label for the new session (displayed in `agent watch`)                                                                                                                                                                                                                                                                                                                                                                   |
| `--kind <skill>`                                     | Initial work type for the new session, using the exact skill name. A matching `agent.work_types` key displays its icon and color                                                                                                                                                                                                                                                                                         |
| `--model <model>`                                    | Model for the new session, passed through to `--model` of the `--engine` CLI; accepts whatever that CLI does (e.g. an alias `opus`/`sonnet` or a full name `claude-fable-5` for `claude`, `gpt-5` for `codex`); for `codex`, default: `agent.codex.model` config                                                                                                                                                         |
| `--parent-session-id <id>`                           | Parent session ID for tree view hierarchy                                                                                                                                                                                                                                                                                                                                                                                |
| `--engine <claude\|codex>`                           | Coding agent CLI to launch (default: `agent.default_engine` config, itself defaulting to `claude`). With `--worktree`, see below                                                                                                                                                                                                                                                                                         |
| `--reasoning-effort <low\|medium\|high\|xhigh\|max>` | Reasoning effort for this launch only. Passed as `claude --effort <effort>`; for Codex, sent in the first `turn/start` on the daemon route and passed as `-c model_reasoning_effort=<effort>` otherwise (default: `agent.codex.reasoning_effort` config)                                                                                                                                                                 |

Worktree creation in the same repository is serialized through the `post-worktree-create` hook. Concurrent calls wait for the hook or its rollback to finish.

Without `--worktree`, `a agent new` compares the target repo (from `-R`, or the current directory) against the repo of the invoking session. When they match and the caller pane resolves from `$TMUX_PANE` or the invoking agent session's recorded tmux pane, it splits that pane into a new pane in the same window. Otherwise -- the repos differ, or no caller pane can be resolved -- it opens a new tmux window in the target repo's own tmux session.

`a agent new` auto-detects the `CLAUDECODE` environment variable: when set (e.g. invoked from a Claude Code Bash tool), the split or new window is built in the background without stealing focus from the current pane/window. Run from a human shell, focus switches to the new pane or window as before.

For the default Claude engine, `--prompt` normally launches `claude [--model <model>] [--effort <effort>]` without putting the prompt on the pane's command line. With exactly one Claude pane, armyknife identifies the new session by its tmux location in Claude Code's session registry and delivers the prompt through its messaging socket.

With `--prompt`, Claude messaging requires exactly one Claude pane. A layout with a different number of Claude panes fails before opening the window or pane. If armyknife cannot resolve the pane's tmux location, the messaging socket does not appear within 20 seconds, or delivery fails, the command exits with an error that identifies the pane; the pane remains open without the initial prompt.

`--engine codex` launches `codex` instead of `claude` (pane command and, without `--worktree`, window-name placeholder). The launch route depends on the prompt and app-server availability:

- Without `--prompt`, armyknife runs `codex [--model <model>] [-c model_reasoning_effort=<effort>]`.
- With `--prompt`, exactly one Codex pane, and a running shared Codex app-server that exposes its control socket, armyknife connects before opening the pane. It launches `codex [--model <model>]` without config overrides, then sends the prompt and reasoning effort in the first `turn/start` request. If the app-server is unavailable or the layout does not have exactly one Codex pane, the command exits with an error before opening the window or pane.
- If the app-server rejects the initial `turn/start` request, or armyknife cannot confirm that the thread started, the command exits with an error that identifies the pane; the pane remains open without the initial prompt. If `turn/start` was sent but its response could not be read, armyknife binds the pane to the created thread, reports a warning, and does not retry the prompt through argv.

The daemon route marks its Codex pane as armyknife-managed, so an environment where `codex` is aliased to `a agent codex` does not perform pane binding twice. A hand-run `a agent codex` keeps its normal pane binding behavior.

With `--worktree`, the session runs in `config.agent.worktree.layout`, whose pane commands are yours to write. Every pane running `claude` (e.g. `command: claude`) is replaced by plain `codex`, dropping its arguments because they are Claude Code flags. The layout is left as written when it already has a `codex` pane, and panes running anything else are never touched. With `--prompt`, the layout must contain exactly one Codex pane because `thread/started` does not identify its originating pane.

A session's engine is recorded on first hook event (see `--engine` on `a agent hook` below) and later read back by `a agent resume` to decide which binary to relaunch. An explicit `a agent resume --engine` value takes precedence when tmux-resurrect restores a snapshot whose store record is missing. `resume` never uses `agent.default_engine`, so changing the default doesn't affect resuming existing sessions.

Set `agent.default_engine: codex` in config, or prefix a single invocation with `ARMYKNIFE_AGENT__DEFAULT_ENGINE=codex` (see [Environment variable overrides](#environment-variable-overrides)), to change what an omitted `--engine` resolves to (with `--worktree` too, so a `codex` default replaces the layout's `claude` pane); an explicit `--engine` on the command line always wins over both.

`agent.codex.model` and `agent.codex.reasoning_effort` are the defaults for `--model` and `--reasoning-effort` when the engine is `codex`; the flags win over them for a single invocation. The model is passed on the `codex` command line. The effort follows the launch routes above, so it is sent through `turn/start` or a launch-only `-c` override without modifying `~/.codex/config.toml`. These defaults never apply to `claude` sessions; an explicit `--reasoning-effort` does, as `claude --effort`.

#### Setup

Add the following to your Claude Code settings (`~/.claude/settings.json`):

```json
{
  "hooks": {
    "SessionStart": [
      {
        "hooks": [
          { "type": "command", "command": "a agent hook session-start" }
        ]
      }
    ],
    "UserPromptSubmit": [
      {
        "hooks": [
          { "type": "command", "command": "a agent hook user-prompt-submit" }
        ]
      }
    ],
    "PreToolUse": [
      {
        "hooks": [{ "type": "command", "command": "a agent hook pre-tool-use" }]
      }
    ],
    "PostToolUse": [
      {
        "hooks": [
          { "type": "command", "command": "a agent hook post-tool-use" }
        ]
      }
    ],
    "Notification": [
      {
        "hooks": [{ "type": "command", "command": "a agent hook notification" }]
      }
    ],
    "Stop": [
      {
        "hooks": [{ "type": "command", "command": "a agent hook stop" }]
      }
    ],
    "SessionEnd": [
      {
        "hooks": [{ "type": "command", "command": "a agent hook session-end" }]
      }
    ]
  }
}
```

These hooks record session state changes, enabling `a agent list` to display active sessions with their current status (running, waiting for input, or stopped).

The `SessionStart` and `UserPromptSubmit` hooks store the agent session ID in the tmux pane user option `@armyknife-last-agent-session-id`, so that `a agent resume` can relaunch the session inside that pane. Reading this option falls back to the pre-rename key `@armyknife-last-claude-code-session-id` when the current key is unset, since a pane only gets its option rewritten on those two hook events, and a pane holding a still-running session would otherwise lose its binding right after the rename.

The pane is normally found by walking the hook process's ancestry up to it. A Codex session running through the shared `codex app-server` daemon breaks that link: the daemon runs the agent loop, hooks included, outside any pane's process tree, so armyknife falls back to the launcher-written session ID option. It binds only when exactly one pane has that session ID and is currently running the Codex CLI. The lookup accepts both `@armyknife-last-agent-session-id` and the pre-rename `@armyknife-last-claude-code-session-id` option; if there is no unique match, the session remains unbound.

`a agent hook <event> --engine <claude|codex>` (default: `claude`) records which coding agent CLI fired the hook; this is the engine `a agent resume` reads back to pick which binary to relaunch (see `--engine` on `new` above). Event names and JSON payload shape are the same regardless of engine: `stop` resolves to `Stopped` ("waiting for the next prompt"), and both `permission-request` and `notification` with `notification_type: "permission_prompt"` resolve to `WaitingInput` ("waiting for tool-call approval") -- so a Codex hook configuration that maps Codex's own hook events onto these same `--event` values and payload fields gets the same approval-wait/input-wait distinction in `a agent list` for free.

Register these in Codex's `hooks.json` (each command with `--engine codex`) to track Codex sessions the same way as Claude Code sessions:

| Codex event         | Command                                          |
| ------------------- | ------------------------------------------------ |
| `SessionStart`      | `a agent hook session-start --engine codex`      |
| `UserPromptSubmit`  | `a agent hook user-prompt-submit --engine codex` |
| `PostToolUse`       | `a agent hook post-tool-use --engine codex`      |
| `PermissionRequest` | `a agent hook permission-request --engine codex` |
| `Stop`              | `a agent hook stop --engine codex`               |
| `SessionEnd`        | `a agent hook session-end --engine codex`        |

```json
{
  "hooks": {
    "Stop": [
      {
        "matcher": ".*",
        "hooks": [
          { "type": "command", "command": "a agent hook stop --engine codex" }
        ]
      }
    ]
  }
}
```

Codex has no `Notification` event, so nothing maps to `a agent hook notification`. `SessionEnd` is best-effort: Codex may exit without firing it or cut the hook short at its own per-hook `timeout`, and a Codex session that never reports it is handled by `a agent sweep` like any other stopped session. Codex has no equivalent of `CLAUDE_ENV_FILE`, so `ARMYKNIFE_SESSION_ID` is never set inside a Codex session; commands that need the caller's own session ID (`a agent new`'s parent resolution, `a agent peer parent`/`children`) fall back to `CODEX_SESSION_ID`, which Codex exports to every command it runs.

Both engines delay permission notifications by about one second because another `PermissionRequest` hook can resolve the request without showing the approval UI. A subsequent event that clears the pending request, such as `PostToolUse` or `Stop`, cancels the notification. Codex sessions with a top-level `approvals_reviewer = "auto_review"` in `$CODEX_HOME/config.toml` (default `~/.codex/config.toml`) wait through Codex v0.155.1's 90-second automatic approval review timeout before rechecking, so the notification is delayed by about 92 seconds in that case. Profile-scoped values are not read. Launch-only `codex -c` overrides and Apps connector reviewer overrides are resolved inside Codex and are not visible to armyknife: when the persisted global setting is `auto_review`, those cases use the same conservative delayed path; when it is not, the one-second path can notify while an override review is running. An approved command that runs longer than this window can still leave the hook state pending until `PostToolUse` and may produce a notification because hooks do not expose a separate running-tool signal.

#### Peer session name resolution

Claude Code's `SendMessage`/`ListAgents` tools address other sessions by an opaque `name` that Claude Code assigns internally and exposes nowhere else except `~/.claude/sessions/<pid>.json`. When several sessions share a working directory (e.g. many delegated `a agent new` sessions in the same worktree), the names in `ListAgents` are indistinguishable from the outside. `a agent peer` resolves the right name by joining armyknife's own session tracking (`ancestor_session_ids`, populated whenever `a agent new` resolves a parent session) against that registry file, so a session doesn't have to guess which `ListAgents` row is its parent or child.

`a agent peer parent`, `a agent peer children`, `a agent peer list [-R <repo>]` (filter by a substring of the session's working directory), and `a agent peer me` all print a JSON array of `{name, session_id, cwd, label, status, pane_id, engine}`; `name` is `null` when Claude Code's registry has no matching entry, and `pane_id` is `null` when the session wasn't started inside tmux. `engine` is `"claude"` or `"codex"` (see `--engine` on `new` above), letting a caller juggling several peers tell which CLI a session belongs to before deciding how to reach it. `parent` and `children` are filtered subsets of `list`: `parent` has zero entries when this session has no tracked parent, `children` has zero entries when nothing was delegated to it.

```console
$ a agent peer parent
[{"name":"myproject-4f","session_id":"1111...","cwd":"/Users/example/ghq/github.com/example/myproject","label":null,"status":"running","pane_id":"%3","engine":"claude"}]
$ a agent peer parent | jq -r '.[0].name // empty'
myproject-4f
$ a agent peer list -R myproject
[{"name":"myproject-9c","session_id":"2222...","cwd":"/Users/example/ghq/github.com/example/myproject/.worktrees/feature-x","label":"fix login bug","status":"running","pane_id":"%7","engine":"claude"},{"name":null,"session_id":"3333...","cwd":"/Users/example/ghq/github.com/example/myproject/.worktrees/feature-y","label":null,"status":"stopped","pane_id":null,"engine":"codex"}]
```

`a agent peer me` resolves the session running in the caller's own tmux pane -- via the pane's `@armyknife-last-agent-session-id` user option (see above), the same mechanism `a agent resume` uses, not `ARMYKNIFE_SESSION_ID` -- so it works from a human-typed shell command in the target session's own pane (e.g. bash mode: `!a agent peer me`), which doesn't carry that env var. It's how a human names a session that has no parent/child relationship to point at: run it in the pane they're looking at, then pass `session_id`/`pane_id` to whatever needs to address that session. It fails with a distinct error for each of: running outside tmux (`$TMUX_PANE` unset), the pane having no recorded session ID, and a recorded session ID that's no longer tracked (e.g. garbage-collected by `a agent sweep`).

```console
$ a agent peer me
[{"name":"myproject-4f","session_id":"1111...","cwd":"/Users/example/ghq/github.com/example/myproject","label":null,"status":"running","pane_id":"%3","engine":"claude"}]
```

`.[0].name` is `null` both when the array is empty (no tracked peer) and when the tracked peer's process has exited without leaving a registry entry -- most commonly a session `a agent sweep` has paused. `// empty` collapses both cases to empty output, so a caller can tell "no usable name" apart from the literal string `"null"`. When the peer is merely paused (its `session_id` still resolves via `a agent peer`), `a agent peer wake <session_id>` resumes its tmux pane, waits for it to re-register, and prints the freshly resolved name -- the name changes on every resume, so re-run `a agent peer` (or use `peer wake`'s own output) rather than reusing a name seen before the pause. `peer wake` is separate from `a agent resume`: `resume` replaces the calling pane's own process and only makes sense run from inside the target pane, while `peer wake` runs from an unrelated caller and never touches its own process. For a Codex session (which has no `SendMessage` name) `peer wake` only respawns the pane and prints nothing (and refuses an `Ended` one).

```console
$ a agent peer parent | jq -r '.[0].name // empty'
$ a agent peer parent | jq -r '.[0].session_id'
1111...
$ a agent peer wake 1111...
myproject-7e
```

`a agent peer notify <session_id> -m <text>` delivers a message to a Claude Code session's `SendMessage` socket directly (for a Codex target, see below), without any Claude Code session driving the call -- useful when the caller is a background process rather than another Claude Code session. When the caller's session ID is known, it refuses to notify that same session. It resumes a `Paused` target via the same flow as `peer wake` first, and refuses outright for an `Ended` session (the user terminated it intentionally). It fails loudly, rather than silently succeeding, when the target's registry entry has no `messagingSocketPath` -- this happens when the target session was started by a Claude Code build that predates peer messaging.

For a Codex target (see `engine` above), direct delivery requires the persistent Codex app-server for the target's `$CODEX_HOME` (default: `~/.codex`). It injects the message into an active turn immediately or starts a new turn when the thread is idle. The command prints which case applied, based on armyknife's tracked session status. If the app-server is unavailable, the thread belongs to an embedded app-server, or the server rejects the request, `notify` falls back to `codex queue --thread <session_id>` and prints the direct-delivery failure. The fallback requires `codex` in `PATH` with the target's `$CODEX_HOME`. A queued message is not delivered yet: the running `codex` polls about every 10 seconds and can start it only after the current turn finishes and the thread is idle. If both direct delivery and queueing fail, the command returns both errors. A `Paused` Codex target is resumed before delivery is attempted; if its thread is not registered yet, the resumed `codex` picks up the queued message when it loads the thread.

`notify` identifies the sender automatically: it tries `ARMYKNIFE_SESSION_ID` (set by the Claude Code `session-start` hook), then `CLAUDE_CODE_SESSION_ID`, then `CODEX_SESSION_ID` (the ambient variables each CLI exports, which cover Codex sessions and Claude Code sessions whose hooks aren't registered), and wraps the message in a `<peer-message>` envelope naming whichever one resolves, since the underlying `SendMessage` protocol carries no sender field of its own -- without it, a session juggling several peers can't tell which one a message came from. When the resolved sender is a tracked session, the envelope also names its `engine` (`claude`/`codex`, see `peer parent`/`children`/`list`/`me` above), so the recipient knows whether to expect a `SendMessage`-capable reply. When it isn't tracked, a sender resolved via `CLAUDE_CODE_SESSION_ID`/`CODEX_SESSION_ID` still gets an `engine` guessed from that variable; one resolved via `ARMYKNIFE_SESSION_ID` has no such hint, so the line is omitted instead. When nothing resolves (e.g. `a agent close` calling `notify` directly, with no session in the loop), the message is delivered unwrapped.

```console
$ a agent peer notify 1111... -m "PR merged, worktree cleaned up"
```

#### tmux-resurrect integration

Pane user options are not preserved by tmux-resurrect, so `a agent resurrect save` persists them to `~/.cache/armyknife/cc/resurrect/pane_sessions.txt`, and `a agent resurrect restore` re-applies them and types `a agent resume <session-id>` into each pane, so the coding agent comes back automatically after a tmux server crash or restart. Restore skips typing the resume command into any pane whose process tree already has a live process for the selected engine, so re-running it against a session that is already active does not retype the command into its input box.

`a agent resurrect save` also records each session's engine and `ancestor_session_ids` (used by `a agent peer parent`/`children`, see above) alongside its session ID, since a tmux server restart can wipe the session's store JSON before restore runs. `a agent resurrect restore` passes the snapshot to `a agent resume --engine ... --ancestor-session-ids ...`, so the correct agent is relaunched and `a agent peer parent`/`children` keep working even if the store JSON has to be rebuilt from scratch.

Wire the commands into tmux-resurrect via its post-save / post-restore hooks:

```tmux
set -g @resurrect-hook-post-save-all '$HOME/.cargo/bin/a agent resurrect save'
set -g @resurrect-hook-post-restore-all '$HOME/.cargo/bin/a agent resurrect restore'
```

Use `$HOME` rather than `~`: tmux escapes a leading `~` in option values, which prevents tilde expansion when tmux-resurrect `eval`s the hook.

#### Auto-pause

Sessions that stay in the `stopped` state for longer than the configured timeout are automatically terminated to free up system resources. The session file is preserved and the status is flipped to `paused` once the process is confirmed gone, so `a agent resume` can restore the conversation.

`a agent sweep` scans every session file once. On the first shutdown request for a Codex session with a recorded tmux pane, it sends Ctrl+D so Codex can restore the terminal, then falls back to SIGTERM if the process remains alive. Codex sessions without a pane and Claude Code sessions receive SIGTERM directly. Later sweeps re-send SIGTERM while the process remains alive; the status stays `stopped` until exit is confirmed. Run sweep periodically via a launchd agent so idle sessions eventually get paused even while no hook is firing.

| Command                   | Description                                                       |
| ------------------------- | ----------------------------------------------------------------- |
| `a agent sweep`           | Run a single sweep pass (equivalent to `a agent sweep run`)       |
| `a agent sweep install`   | Install and bootstrap a launchd agent that runs sweep every 5 min |
| `a agent sweep status`    | Print the plist path and whether the agent is bootstrapped        |
| `a agent sweep uninstall` | Bootout the agent and remove its plist                            |

Options for the run command:

| Option             | Description                                                    |
| ------------------ | -------------------------------------------------------------- |
| `--timeout <spec>` | Override the config timeout for this run (e.g., `1m`, `1h30m`) |
| `--dry-run`        | Print what would be paused without sending signals or saving   |

The `install`, `uninstall`, and `status` subcommands require macOS. The launchd agent is installed at `~/Library/LaunchAgents/fohte.armyknife.cc-sweep.plist`.

Configure via `~/.config/armyknife/config.yaml`:

```yaml
agent:
  auto_pause:
    enabled: true # default: true
    timeout: 30m # default: "30m" (accepts "30s", "10m", "1h30m", etc.)
```

Set `enabled: false` to disable auto-pausing entirely. The launchd agent stays installed but exits immediately when `enabled` is false, so toggling via config does not require `uninstall`.

#### Auto-compact

Default Claude Code auto-compact fires the moment a hard token threshold is crossed, which often interrupts an in-flight chain of prompts and discards context the user still needs. Armyknife's auto-compact instead fires only when the session has been idle long enough that the user is likely done — but still soon enough that the prompt cache is warm, so the `/compact` invocation itself reuses the cache rather than re-paying for the whole context.

The Stop hook spawns a detached `a agent auto-compact schedule` worker per Stop event. After `idle_timeout` of inactivity (anchored on the Stop event) it SIGTERMs the live `claude` process and runs `claude -r <session_id> -p "/compact"` so the compaction lands on the same session.

The worker re-checks state at wake-up and aborts in any of these cases:

- The session is no longer `stopped` (user resumed, sweep paused it, …).
- The pane's pty atime is newer than the Stop time (user is mid-prompt).
- The session's branch has a merged PR (the conversation is shipped work; compacting it is wasteful).
- The most recent assistant turn's prompt is smaller than `min_context_tokens` (compacting a tiny context discards useful state without freeing meaningful budget). Sessions whose transcript or usage record cannot be read are also skipped on this check.

Each new Stop hook cancels the previously-armed worker for the same pane via the `@armyknife-auto-compact-timer-pid` pane option, so a quick follow-up turn transparently re-arms the timer rather than firing a stale compaction.

Configure via `~/.config/armyknife/config.yaml`:

```yaml
agent:
  auto_compact:
    enabled: true # default: true
    idle_timeout: 4m30s # default: "4m30s" (slightly under the 5m prompt cache TTL)
    min_context_tokens: 180000 # default: 180000 (input + cache_read + cache_creation + output of the latest assistant turn)
```

The default `idle_timeout` of 4m30s targets the 5-minute prompt cache TTL on Claude Code subscriptions; tune it up (e.g. `idle_timeout: 55m`) if your Anthropic API account uses the 1-hour cache.

`min_context_tokens` is measured against the actual prompt size of the latest assistant turn (input + cache_read + cache_creation + output), so it tracks effective context use independent of which model context window (200k vs 1M) is in play.

#### Unread stopped sessions

Stopped sessions that have not been focused since their most recent Stop render as `✱` (unread); focusing the pane reverts them to `○` (read). Wire `a agent mark-read` into tmux's `pane-focus-in` hook to enable this — see [docs/setup.md](docs/setup.md).

#### Window status

`a agent hook` keeps each tmux window's aggregated Claude Code status in the window-scoped user option `@armyknife-cc-window-status`. On every session state change it recomputes the status symbols (`●` running, `◐` waiting for input or stopped with pending work and a linked crit or Human-in-the-Loop review, `◎` main loop idle with only a background task/subagent still in flight and no review waiting for user input, `✱` stopped & unread, `○` stopped & read, `⏸` paused) of every Claude Code session in the window's panes, concatenates them without a separator, writes the result to `@armyknife-cc-window-status`, and refreshes the status bar — but only when the rendered value actually changed, so no-op transitions cause no redraw. `a agent bg run` workers also use `◎` while the main loop is stopped and no review is waiting for user input.

The same sync also mirrors a session title into the window-scoped `@armyknife-cc-window-title` option: the `label` of the first session in the window (in pane order) that has one set, or an empty string if none do — titles are not concatenated across sessions in the same window. Press `e` in `a agent watch` to rename the selected session's title, persisting it as `label`; the tmux option is refreshed best-effort on confirm (skipped silently if the pane has no resolvable window), and otherwise catches up on the next status-changing hook event for that window. While renaming, press `Ctrl+g` to generate a title from the session's transcript (its first user message and latest assistant message) — this returns you to the session list immediately, no waiting: generation runs in a fully detached background process that keeps going even if `agent watch` is closed entirely, and applies the generated title directly once it lands, but only if you haven't renamed the session again in the meantime. Generation shells out to the same backend as `a agent new` (the `claude` CLI, falling back to `opencode`), so it requires one of those to be installed and authenticated.

Reference both options from tmux's `window-status-format` to surface per-window session state and title next to the window index. `#{?...}` falls back to `#W` (the tmux window name) when no session in the window has a title set:

```tmux
set -g window-status-format '#{@armyknife-cc-window-status}#I:#{?#{@armyknife-cc-window-title},#{@armyknife-cc-window-title},#W}'
```

`a agent window-status <window_id>` prints the same status symbols on demand, for manual inspection or a polling-based `window-status-format`. The output contains no tmux style markup so the symbols inherit the surrounding `window-status-*` style (avoids `reverse` painting the icon cell as a colored block).

#### Pane has-paused flag

`a agent hook` also materializes a per-pane paused flag as a marker file at `${TMPDIR:-/tmp}/armyknife-cc-paused-${USER:-unknown}-<pane_id>` (e.g. `/tmp/armyknife-cc-paused-fohte-%17`). The file exists exactly while the pane's Claude Code session is `Paused` (e.g. SIGTERMed by `auto_pause`); every other state (`Running` / `WaitingInput` / `Stopped` / `Ended`) removes it. The `${USER:-unknown}` segment prevents collisions on multi-user hosts where `TMPDIR` falls back to a shared `/tmp` and tmux pane IDs clash across users. Downstream prompt renderers (e.g. starship) can surface a resumable-session label with a `test -e "${TMPDIR:-/tmp}/armyknife-cc-paused-${USER:-unknown}-${TMUX_PANE}"` check, which avoids the tmux client round trip a pane user option would require on every prompt. A file-existence flag is used rather than the session name so the prompt distinguishes an armyknife-paused session (file exists) from a user-driven Ctrl-C exit (no file). `a agent pane-has-paused <pane_id>` prints `1` / empty on demand from the session state and is intended for manual inspection; prompt renderers on the hot path should read the file instead.

#### Task linking (tq)

When the `tq` CLI is on `PATH`, a session linked to a tq task shows its `#<number>` in a fixed-width column before the usual breadcrumb/title, so its task is visible without leaving the usual NEEDS YOU / RUNNING / UNREAD / PAUSED-STOPPED sections. Rows without a linked task reserve the same column, keeping every title aligned. The number dims unless the task is related to the cursor row's task -- the same task, or its direct parent/child task -- so scanning for everything tied to the task you're currently looking at doesn't require reading every number. If the linked task is closed, its number renders struck through in a dusty purple, overriding the related/dimmed coloring. Older `tq` binaries that predate task status reporting are treated as open. Linking a session to a task happens on tq's side, keyed by Claude Code session ID; armyknife only reads the resulting association by shelling out to `tq session list`, so tq's own base URL and authentication (e.g. Cloudflare Access) stay entirely tq's concern.

When `tq` isn't on `PATH`, or the command fails, rows keep task associations from the last successful lookup, including data restored from cache at startup. During an initial load without a cache, the task-number column and sidebar tree show skeleton placeholders; task numbers or blank cells replace the placeholders without moving session titles. If that load fails, the sidebar shows `Failed to load tq tasks`; press `r` to retry. Local operations (focus, resume) are never blocked by `tq` being down.

Press `t` to open the selected session's linked task in the browser (no-op if the session has no linked task).

armyknife is the only place that distinguishes a `Paused` session (auto-paused by `a agent sweep`, resumable) from an `Ended` one (the user exited; gone for good) -- tq's own hooks see both as the same `SessionEnd` event. So whenever a session transitions to `Ended` -- via a genuine `SessionEnd`, or via a `Paused` session getting evicted because its tmux pane was taken over by a different session -- `a agent hook` also spawns a detached `a agent delete-tq-session-detached --session <id>` to delete tq's record of that session via `tq session delete claude_code <id>`. This is best-effort and never blocks the hook: `tq` sits behind Cloudflare Access and can be slow or unreachable, and tq performs its own periodic cleanup of stale sessions regardless, so a failed or skipped deletion here is never the only cleanup path.

#### Environment Variables

| Variable                | Values                            | Description                 |
| ----------------------- | --------------------------------- | --------------------------- |
| `ARMYKNIFE_CC_HOOK_LOG` | `error` (default), `debug`, `off` | Controls hook logging level |

- `error`: Log only when JSON parsing fails (default)
- `debug`: Log all hook invocations including successful ones
- `off`: Disable all logging

Logs are saved to `~/Library/Caches/armyknife/cc/logs/` (macOS) or `~/.cache/armyknife/cc/logs/` (Linux).

### `a agent clean`

Delete merged or closed worktrees in the current repository. Pass `--all` to scan repositories under `agent.worktree.repos_root`.

When `a agent close`, `a agent clean`, or the TUI clean view's background cleanup removes a worktree whose branch's PR was merged, and that worktree hosted a delegated Claude Code session (`a agent new --worktree` from another session), it also notifies the delegator session via `a agent peer notify` so a delegator blocked on "wait for this PR to merge" can continue. Best-effort: notification failures (delegator already ended, no messaging socket, etc.) don't affect the cleanup.

After `a agent close`, `a agent clean`, or the TUI clean view removes a worktree successfully, armyknife starts the configured `post-worktree-delete` hook in a detached session. The hook receives the deleted worktree path, branch, repository root, and whether the branch was merged; see [Hooks](docs/hooks.md#post-worktree-delete).

Worktree cleanup also sends SIGTERM to any process group still rooted in the worktree (e.g. a dev server left running by a detached background job), so it doesn't linger holding a port after the directory is gone. The calling process and its ancestors (the shell that invoked the command, etc.) are never targeted. Best-effort: requires `lsof` and `ps`; if either is unavailable, or a process ignores SIGTERM, an orphaned process may be left running.

Options:

| Option          | Description                                                                       |
| --------------- | --------------------------------------------------------------------------------- |
| `-n, --dry-run` | Show what would be deleted without actually deleting                              |
| `--all`         | Clean worktrees across all repositories under `agent.worktree.repos_root`         |
| `--force`       | Delete even worktrees that currently host an active agent session (default: keep) |

Worktrees with an active agent session (not paused or ended, with pending
background tasks, or with an unsent composer draft) are kept by default
regardless of merge status. A draft keeps the worktree active until it is
submitted or cleared. The `STATUS` column shows `active session` for those
entries. Pass `--force` to override.

### Hooks

armyknife supports git-style hooks for command lifecycle events (worktree creation, PR draft review/submit). See [docs/hooks.md](docs/hooks.md) for the available hook points, environment variables, and usage examples.

### Logging

armyknife writes JSONL diagnostic logs to `~/.cache/armyknife/logs/armyknife.log.YYYY-MM-DD` (daily rotation, 7 files retained). Set `ARMYKNIFE_LOG=off|error|info|debug` to control verbosity. See [docs/logging.md](docs/logging.md) for the event reference and debugging recipes.

### `a doctor`

Check availability and versions of external tools armyknife depends on (`git`, `gh`, `tmux`, `nvim`, `wezterm`, `ghostty`, `delta`, `claude`, `opencode`, `lsof`, and Hammerspoon on macOS). Missing tools include an install hint. Exits with status 0 regardless of findings — it is informational, not a gate.

```sh
a doctor
```

### `a completions <shell>`

Generate shell completion scripts.

Supported shells: `bash`, `elvish`, `fish`, `powershell`, `zsh`

```sh
# Example: Add to your shell profile
a completions zsh > ~/.zfunc/_a
```

### `a config`

Configuration management.

#### `a config get <key>`

Get a configuration value by dot-separated key. Supports any config field (e.g., `agent.worktree.branch_prefix`, `editor.terminal`, `notification.sound`). Scalar leaves (string, bool, number) print as bare strings; maps and sequences (e.g., `orgs.<owner>`, `ai.review.reviewers`) print as YAML so the shape round-trips. If the key is missing, an error is printed to stderr and the process exits with status 1.

For `repo.*` and `org.*` keys, the current directory's git remote is used to identify the repository. `repo.*` looks up `repos.<owner>/<repo>` and `org.*` looks up `orgs.<owner>`. `repo.language` falls back to `ja` for private repos and `en` for public repos when no explicit value is set.

```sh
$ a config get agent.worktree.branch_prefix
fohte/

$ a config get notification.sound
Glass

$ cd ~/ghq/github.com/fohte/t-rader
$ a config get repo.language
ja

$ cd ~/ghq/github.com/fohte/dotfiles
$ a config get repo.direct_commit
true
```

## Release

Releases are automated with release-please. The [release workflow](.github/workflows/release-please.yml) creates or updates a release PR on pushes to `master` and automatically merges it. Merging the release PR creates a GitHub release; build jobs upload the pre-built binaries afterward.

After the release PR is merged and the GitHub release has been created, run `a update`. You do not need to wait for the binaries to finish uploading: `a update` retries while your platform's asset is being uploaded. See [`a update`](#a-update) for retry timing.

## License

[MIT](LICENSE)
