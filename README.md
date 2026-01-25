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

#### `a ai pr-draft`

Manage PR body drafts with human-in-the-loop review.

| Action   | Description                         |
| -------- | ----------------------------------- |
| `new`    | Create a new PR body draft file     |
| `review` | Open the draft in editor for review |
| `submit` | Create a PR from the approved draft |

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

| Option           | Description                                        |
| ---------------- | -------------------------------------------------- |
| `-R <repo>`      | Target repository (default: current repo)          |
| `--dry-run`      | Show what would be changed without applying        |
| `--force`        | Overwrite local/remote changes (context-dependent) |
| `--edit-others`  | Allow editing other users' comments                |
| `--allow-delete` | Allow deleting comments removed locally            |

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

| Action         | Description                                           |
| -------------- | ----------------------------------------------------- |
| `hook <event>` | Record session events (called from Claude Code hooks) |
| `list`         | List all Claude Code sessions with status             |

#### Setup

Add the following to your Claude Code settings (`~/.claude/settings.json`):

```json
{
  "hooks": {
    "user-prompt-submit": [{ "command": "a cc hook user-prompt-submit" }],
    "pre-tool-use": [{ "command": "a cc hook pre-tool-use" }],
    "post-tool-use": [{ "command": "a cc hook post-tool-use" }],
    "notification": [{ "command": "a cc hook notification" }],
    "stop": [{ "command": "a cc hook stop" }]
  }
}
```

These hooks record session state changes, enabling `a cc list` to display active sessions with their current status (running, waiting for input, or stopped).

### `a wm`

Git worktree management with tmux integration.

| Action              | Description                                |
| ------------------- | ------------------------------------------ |
| `list`              | List all worktrees                         |
| `new <branch>`      | Create a new worktree and open tmux window |
| `delete [worktree]` | Delete a worktree and its branch           |
| `clean`             | Bulk delete merged worktrees               |

## License

[MIT](LICENSE)
