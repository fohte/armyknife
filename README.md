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

| Action   | Description                                                        |
| -------- | ------------------------------------------------------------------ |
| `new`    | Create a new PR body draft file                                    |
| `review` | Open the draft in editor for review                                |
| `submit` | Create a PR from the approved draft (updates existing PR if found) |

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

| Option                  | Description                                      |
| ----------------------- | ------------------------------------------------ |
| `-R, --repo <repo>`     | Target repository (owner/repo)                   |
| `-r, --reviewer <name>` | Reviewer to request/wait for (default: `gemini`) |
| `--interval <seconds>`  | Polling interval (default: 15)                   |
| `--timeout <seconds>`   | Timeout (default: 300)                           |

### `a gh`

GitHub-related utilities.

#### `a gh issue-agent`

Manage GitHub Issues as local files for AI agents.

```sh
a gh issue-agent <command> <issue-number> [options]
```

| Command | Description                                         |
| ------- | --------------------------------------------------- |
| `view`  | View issue and comments (read-only, no local cache) |
| `pull`  | Fetch issue and save locally                        |
| `push`  | Push local changes to GitHub                        |
| `diff`  | Show colored diff between local changes and remote  |
| `init`  | Create boilerplate files for new issues or comments |

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

##### `a gh issue-agent init comment <issue-number>`

Create a new comment boilerplate file for an existing issue.

| Option          | Description                                    |
| --------------- | ---------------------------------------------- |
| `--name <NAME>` | Name for the comment file (default: timestamp) |

#### `a gh check-pr-review`

Fetch PR review comments in a concise format for AI agents.

```sh
a gh check-pr-review <pr-number> [options]
```

| Option               | Description                           |
| -------------------- | ------------------------------------- |
| `--review <n>`       | Show details for a specific review    |
| `--full`             | Show full details for all reviews     |
| `--include-resolved` | Include resolved threads              |
| `--open-details`     | Expand `<details>` blocks in comments |

### `a cc`

Claude Code session monitoring with tmux integration.

| Action               | Description                                           |
| -------------------- | ----------------------------------------------------- |
| `hook <event>`       | Record session events (called from Claude Code hooks) |
| `list`               | List all Claude Code sessions with status             |
| `focus <session_id>` | Focus on a session's tmux pane                        |
| `resume`             | Resume a session from tmux pane title after restart   |

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

The `SessionStart` hook stores the session ID in the tmux pane title, allowing `a cc resume` to restore the session after a tmux resurrect.

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

| Action              | Description                                |
| ------------------- | ------------------------------------------ |
| `list`              | List all worktrees                         |
| `new <branch>`      | Create a new worktree and open tmux window |
| `delete [worktree]` | Delete a worktree and its branch           |
| `clean`             | Bulk delete merged worktrees               |

### `a completions <shell>`

Generate shell completion scripts.

Supported shells: `bash`, `elvish`, `fish`, `powershell`, `zsh`

```sh
# Example: Add to your shell profile
a completions zsh > ~/.zfunc/_a
```

## License

[MIT](LICENSE)
