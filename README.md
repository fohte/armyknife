# armyknife

[![GitHub release](https://img.shields.io/github/v/release/fohte/armyknife)](https://github.com/fohte/armyknife/releases/latest)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)

Personal CLI toolkit written in Rust

## Installation

### Pre-built binaries

Download from [GitHub Releases](https://github.com/fohte/armyknife/releases/latest).

Available for:

- macOS (Apple Silicon, Intel)
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

## License

[MIT](LICENSE)
