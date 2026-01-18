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

| Command   | Description                                         |
| --------- | --------------------------------------------------- |
| `view`    | View issue and comments (read-only, no local cache) |
| `pull`    | Fetch issue and save locally                        |
| `refresh` | Discard local changes and fetch latest from GitHub  |
| `push`    | Push local changes to GitHub                        |

**Options:**

| Option           | Commands | Description                                 |
| ---------------- | -------- | ------------------------------------------- |
| `-R <repo>`      | all      | Target repository (default: current repo)   |
| `--dry-run`      | push     | Show what would be changed without applying |
| `--force`        | push     | Allow overwriting remote changes            |
| `--edit-others`  | push     | Allow editing other users' comments         |
| `--allow-delete` | push     | Allow deleting comments removed locally     |

**Directory Structure:**

```
~/.cache/gh-issue-agent/<owner>/<repo>/<issue-number>/
├── issue.md          # Issue body
├── metadata.json     # Title, labels, assignees, etc.
└── comments/
    ├── 001_comment_<id>.md   # Existing comments
    └── new_<name>.md         # New comments (created locally)
```

**Workflow:**

1. `pull` to fetch an issue locally
2. Edit `issue.md`, `metadata.json`, or files in `comments/`
3. Create new comments as `comments/new_<name>.md`
4. `push` to apply changes to GitHub

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
