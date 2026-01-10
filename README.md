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

### Commands

- `a update` - Update to the latest version
- `a ai pr-draft <subcommand>` - Manage PR body drafts for AI-assisted PR creation
  - `new` - Create a new PR body draft file
  - `review` - Open the draft in Neovim for review (via WezTerm)
  - `submit` - Create a PR from the approved draft
- `a wm <subcommand>` - Git worktree management with tmux integration
  - `list` - List worktrees with branch name, merge status, and PR info
  - `new <branch>` - Create a new worktree and open tmux window with nvim + claude
  - `delete [worktree]` - Delete a worktree (moves to main if run from within)
  - `clean` - Bulk delete merged worktrees

The CLI automatically checks for updates and prompts you to update when a new version is available.

## License

[MIT](LICENSE)
