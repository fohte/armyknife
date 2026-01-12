# armyknife

[![GitHub release](https://img.shields.io/github/v/release/fohte/armyknife)](https://github.com/fohte/armyknife/releases/latest)
[![codecov](https://codecov.io/gh/fohte/armyknife/graph/badge.svg)](https://codecov.io/gh/fohte/armyknife)
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

## License

[MIT](LICENSE)
