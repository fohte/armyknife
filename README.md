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

Mapping keys are merged recursively; sequences and scalars are replaced wholesale by later files. All fields are optional and fall back to sensible defaults. If no config files exist, armyknife runs entirely on defaults.

For editor autocompletion, add the following to the top of your config file:

```yaml
# yaml-language-server: $schema=https://raw.githubusercontent.com/fohte/armyknife/master/docs/config-schema.json
```

### Example

```yaml
# yaml-language-server: $schema=https://raw.githubusercontent.com/fohte/armyknife/master/docs/config-schema.json

wm:
  worktrees_dir: .worktrees # worktree directory name (default: ".worktrees")
  branch_prefix: fohte/ # branch name prefix for `a wm new` (default: "fohte/")
  repos_root: ~/ghq # root directory for repo discovery in `a wm clean --all` (default: GHQ_ROOT or ghq.root or ~/ghq)
  layout: # tmux pane layout for `a wm new`
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
        reviewers: [gemini] # default reviewers for `a ai review wait`/`request` in this org

repos: # per-repository overrides, keyed by "owner/repo"
  fohte/dotfiles:
    language: en # language for commit messages and PR content (default: "ja" for private repos, "en" for public repos)
    direct_commit: true # allow direct commits to the default branch; consumed by external git hooks
    ai:
      review:
        reviewers: [gemini, devin] # repo-level reviewer override (takes precedence over org)
```

### Splitting public and private config

Because every YAML file in the directory is merged, you can keep public (dotfiles-tracked) and private (company-only) config separate. For example, drop a single file from a private repository as a symlink:

```sh
ln -s ~/work/dotfiles-private/armyknife.yaml ~/.config/armyknife/work.yaml
```

`config.yaml` is loaded first (alphabetical), `work.yaml` overrides it. Subdirectories such as `hooks/` are not scanned and remain unaffected.

### Supported Terminal Emulators

The `editor.terminal` setting selects which terminal emulator opens for human-in-the-loop reviews. Each terminal has built-in support for window size and title options.

| `terminal` value | Terminal          |
| ---------------- | ----------------- |
| `wezterm`        | WezTerm (default) |
| `ghostty`        | Ghostty           |

## Commands

### `a update`

Update to the latest version.

The CLI automatically checks for updates and prompts you to update when a new version is available.

### `a name-branch <description>`

Generate a branch name from a description using AI.

### `a ai`

Commands designed for AI agents (e.g., Claude Code) to call programmatically.
These provide structured inputs/outputs suitable for AI workflows.

#### `a ai draft <path>`

Open a file in editor for review (no approval flow).

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

| Option                  | Description                                                                                                       |
| ----------------------- | ----------------------------------------------------------------------------------------------------------------- |
| `-R, --repo <repo>`     | Target repository (owner/repo)                                                                                    |
| `-r, --reviewer <name>` | Reviewer(s) to request/wait for; can be repeated. When omitted, falls back to repo > org config > `gemini, devin` |
| `--interval <seconds>`  | Polling interval (default: 15)                                                                                    |
| `--timeout <seconds>`   | Timeout (default: 300)                                                                                            |

When `--reviewer` is omitted, the reviewer set is resolved in this order:

1. `repos.<owner>/<repo>.ai.review.reviewers` (per-repo override)
2. `orgs.<owner>.ai.review.reviewers` (per-org default)
3. The built-in `[gemini, devin]`

This makes it possible to disable a reviewer that isn't enabled in a given org (e.g., `reviewers: [gemini]` for a fohte-only repo) without passing `--reviewer` on every invocation.

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

| Option           | Description                           |
| ---------------- | ------------------------------------- |
| `--review <n>`   | Show details for a specific review    |
| `--full`         | Show full details for all reviews     |
| `-a, --all`      | Include resolved threads              |
| `--open-details` | Expand `<details>` blocks in comments |

> `a gh check-pr-review` is a deprecated alias for `a gh pr-review check`.

##### `a gh pr-review reply pull`

Fetch review threads to a local Markdown file for editing.

```sh
a gh pr-review reply pull <pr-number> [options]
```

| Option               | Description                              |
| -------------------- | ---------------------------------------- |
| `-R, --repo <REPO>`  | Target repository (owner/repo)           |
| `--include-resolved` | Include resolved threads                 |
| `--force`            | Overwrite local changes without checking |

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

### `a cc`

Claude Code session monitoring with tmux integration.

| Action                                 | Aliases | Description                                                              |
| -------------------------------------- | ------- | ------------------------------------------------------------------------ |
| `hook <event>`                         |         | Record session events (called from Claude Code hooks)                    |
| `list`                                 | `ls`    | List all Claude Code sessions with status                                |
| `focus <session_id>`                   |         | Focus on a session's tmux pane                                           |
| `resume [session_id]`                  | `r`     | Resume the pane's Claude Code session (reads pane option if no argument) |
| `resurrect save`                       |         | Save pane session IDs for tmux-resurrect (run from post-save hook)       |
| `resurrect restore`                    |         | Restore pane session IDs and relaunch Claude Code (from post-restore)    |
| `sweep`                                |         | Pause long-stopped sessions (run periodically or manual)                 |
| `auto-compact schedule --session <id>` |         | Detached worker spawned by the Stop hook (not for direct use)            |

#### Setup

Add the following to your Claude Code settings (`~/.claude/settings.json`):

```json
{
  "hooks": {
    "SessionStart": [
      {
        "hooks": [{ "type": "command", "command": "a cc hook session-start" }]
      }
    ],
    "UserPromptSubmit": [
      {
        "hooks": [
          { "type": "command", "command": "a cc hook user-prompt-submit" }
        ]
      }
    ],
    "PreToolUse": [
      {
        "hooks": [{ "type": "command", "command": "a cc hook pre-tool-use" }]
      }
    ],
    "PostToolUse": [
      {
        "hooks": [{ "type": "command", "command": "a cc hook post-tool-use" }]
      }
    ],
    "Notification": [
      {
        "hooks": [{ "type": "command", "command": "a cc hook notification" }]
      }
    ],
    "Stop": [
      {
        "hooks": [{ "type": "command", "command": "a cc hook stop" }]
      }
    ],
    "SessionEnd": [
      {
        "hooks": [{ "type": "command", "command": "a cc hook session-end" }]
      }
    ]
  }
}
```

These hooks record session state changes, enabling `a cc list` to display active sessions with their current status (running, waiting for input, or stopped).

The `SessionStart` and `UserPromptSubmit` hooks store the Claude Code session ID in the tmux pane user option `@armyknife-last-claude-code-session-id`, so that `a cc resume` can relaunch `claude --resume <id>` inside that pane.

#### tmux-resurrect integration

Pane user options are not preserved by tmux-resurrect, so `a cc resurrect` persists them to `~/.cache/armyknife/cc/resurrect/pane_sessions.txt` during save and re-applies them during restore. On restore the hook also types `a cc resume <session-id>` into each pane, so Claude Code comes back automatically after a tmux server crash or restart.

Wire the commands into tmux-resurrect via its post-save / post-restore hooks:

```tmux
set -g @resurrect-hook-post-save-all '$HOME/.cargo/bin/a cc resurrect save'
set -g @resurrect-hook-post-restore-all '$HOME/.cargo/bin/a cc resurrect restore'
```

Use `$HOME` rather than `~`: tmux escapes a leading `~` in option values, which prevents tilde expansion when tmux-resurrect `eval`s the hook.

#### Auto-pause

Sessions that stay in the `stopped` state for longer than the configured timeout are automatically terminated with SIGTERM to free up system resources. The session file is preserved and the status is flipped to `paused`, so `a cc resume` can restore the conversation by invoking `claude --resume`.

`a cc sweep` scans every session file once, sends SIGTERM to any session whose `stopped` timeout has elapsed, and marks it as `paused`. Run it periodically via a launchd agent so idle sessions get paused even while no hook is firing.

| Command                | Description                                                       |
| ---------------------- | ----------------------------------------------------------------- |
| `a cc sweep`           | Run a single sweep pass (equivalent to `a cc sweep run`)          |
| `a cc sweep install`   | Install and bootstrap a launchd agent that runs sweep every 5 min |
| `a cc sweep status`    | Print the plist path and whether the agent is bootstrapped        |
| `a cc sweep uninstall` | Bootout the agent and remove its plist                            |

Options for the run command:

| Option             | Description                                                    |
| ------------------ | -------------------------------------------------------------- |
| `--timeout <spec>` | Override the config timeout for this run (e.g., `1m`, `1h30m`) |
| `--dry-run`        | Print what would be paused without sending signals or saving   |

The `install`, `uninstall`, and `status` subcommands require macOS. The launchd agent is installed at `~/Library/LaunchAgents/fohte.armyknife.cc-sweep.plist`.

Configure via `~/.config/armyknife/config.yaml`:

```yaml
cc:
  auto_pause:
    enabled: true # default: true
    timeout: 30m # default: "30m" (accepts "30s", "10m", "1h30m", etc.)
```

Set `enabled: false` to disable auto-pausing entirely. The launchd agent stays installed but exits immediately when `enabled` is false, so toggling via config does not require `uninstall`.

#### Auto-compact

Default Claude Code auto-compact fires the moment a hard token threshold is crossed, which often interrupts an in-flight chain of prompts and discards context the user still needs. Armyknife's auto-compact instead fires only when the session has been idle long enough that the user is likely done — but still soon enough that the prompt cache is warm, so the `/compact` invocation itself reuses the cache rather than re-paying for the whole context.

The Stop hook spawns a detached `a cc auto-compact schedule` worker per Stop event. After `idle_timeout` of inactivity (anchored on the Stop event) it SIGTERMs the live `claude` process and runs `claude -r <session_id> -p "/compact"` so the compaction lands on the same session.

The worker re-checks state at wake-up and aborts in any of these cases:

- The session is no longer `stopped` (user resumed, sweep paused it, …).
- The pane's pty atime is newer than the Stop time (user is mid-prompt).
- The session's branch has a merged PR (the conversation is shipped work; compacting it is wasteful).

Each new Stop hook cancels the previously-armed worker for the same pane via the `@armyknife-auto-compact-timer-pid` pane option, so a quick follow-up turn transparently re-arms the timer rather than firing a stale compaction.

Configure via `~/.config/armyknife/config.yaml`:

```yaml
cc:
  auto_compact:
    enabled: false # default: false (opt-in)
    idle_timeout: 4m30s # default: "4m30s" (slightly under the 5m prompt cache TTL)
```

The default `idle_timeout` of 4m30s targets the 5-minute prompt cache TTL on Claude Code subscriptions; tune it down (e.g. `idle_timeout: 50m`) if your Anthropic API account uses the 1-hour cache.

#### Environment Variables

| Variable                | Values                            | Description                 |
| ----------------------- | --------------------------------- | --------------------------- |
| `ARMYKNIFE_CC_HOOK_LOG` | `error` (default), `debug`, `off` | Controls hook logging level |

- `error`: Log only when JSON parsing fails (default)
- `debug`: Log all hook invocations including successful ones
- `off`: Disable all logging

Logs are saved to `~/Library/Caches/armyknife/cc/logs/` (macOS) or `~/.cache/armyknife/cc/logs/` (Linux).

### `a wm`

Git worktree management with tmux integration.

| Action              | Aliases  | Description                                |
| ------------------- | -------- | ------------------------------------------ |
| `list`              | `ls`     | List all worktrees                         |
| `new <branch>`      |          | Create a new worktree and open tmux window |
| `delete [worktree]` | `d`,`rm` | Delete a worktree and its branch           |
| `clean`             | `c`      | Bulk delete merged or closed worktrees     |

`clean` options:

| Option          | Description                                                |
| --------------- | ---------------------------------------------------------- |
| `-n, --dry-run` | Show what would be deleted without actually deleting       |
| `--all`         | Clean worktrees across all repositories under `repos_root` |

#### Hooks

armyknife supports git-style hooks for worktree lifecycle events. Place executable scripts in `~/.config/armyknife/hooks/` (or `$XDG_CONFIG_HOME/armyknife/hooks/`).

| Hook                   | Trigger                             |
| ---------------------- | ----------------------------------- |
| `post-worktree-create` | After `a wm new` creates a worktree |

The following environment variables are available in hook scripts:

| Variable                  | Description                           |
| ------------------------- | ------------------------------------- |
| `ARMYKNIFE_WORKTREE_PATH` | Absolute path to the created worktree |
| `ARMYKNIFE_BRANCH_NAME`   | Branch name of the worktree           |
| `ARMYKNIFE_REPO_ROOT`     | Root path of the parent repository    |

Hook failures (non-zero exit or missing execute permission) produce a warning but do not block the main operation.

**Example**: Auto-trust worktrees for Claude Code

```sh
#!/bin/sh
# ~/.config/armyknife/hooks/post-worktree-create
jq --arg path "$ARMYKNIFE_WORKTREE_PATH" \
  '.projects[$path].hasTrustDialogAccepted = true' \
  ~/.claude.json | sponge ~/.claude.json
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

#### `a config schema`

Print JSON Schema for the configuration file.

| Option                | Description                            |
| --------------------- | -------------------------------------- |
| `-o, --output <path>` | Write schema to file instead of stdout |

#### `a config get <key>`

Get a configuration value by dot-separated key. Supports any config field (e.g., `wm.branch_prefix`, `editor.terminal`, `notification.sound`). If the key has a value, it is printed to stdout. If not found, nothing is printed (exit 0).

For `repo.*` and `org.*` keys, the current directory's git remote is used to identify the repository. `repo.*` looks up `repos.<owner>/<repo>` and `org.*` looks up `orgs.<owner>`. Only scalar leaves (string, bool, number) are printed; sequence/object values such as `ai.review.reviewers` print nothing.

```sh
$ a config get wm.branch_prefix
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

## License

[MIT](LICENSE)
