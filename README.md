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

Unknown configuration keys fail parsing. Put worktree settings under `agent.worktree`, pause settings under `agent.auto_pause`, compaction settings under `agent.auto_compact`, background task reminders under `agent.bg_task_reminder`, merge notification text under `agent.merge_notification`, status names and states under `agent.statuses`, Stop hook instructions under `agent.stop_hook_instruction`, status inference prompts under `agent.status_inference_prompt`, and Codex inference settings under `agent.codex_status_inference`.

`agent.merge_notification` is an optional template sent to the delegator after a delegated PR merges. Use `{branch}` and `{pr_url}` where the branch name and PR URL should appear. When the key is omitted, no merge notification is sent.

`agent.statuses` maps user-defined names to `user`, `waiting`, or `done` states. `user` status names appear in YOUR TURN and tmux's next action; `waiting` and `done` status names mark TASKS rows without appearing in YOUR TURN. `a agent status set` accepts only names from this map. A stored name missing from the current map is treated as `user` so it remains visible.

`agent.stop_hook_instruction` is the optional blocking instruction returned when the hook blocks. Codex uses it when inference is disabled in a valid config, and Claude uses it when status inference is not configured. The hook blocks only when at least one status is configured; an absent or empty `agent.statuses` map disables blocking even when an instruction is present.

`agent.status_inference_prompt` lets a detached worker infer a missing status from a Claude Code fork or, when enabled below, a temporary Codex fork of the saved conversation using the parent's model. Both paths use tokens.

`agent.codex_status_inference.enabled` opts Codex sessions into detached classification through an ephemeral fork of the saved conversation. Inference requires Codex's built-in OpenAI provider, the `codex` CLI, and the `srt` sandbox runtime. Enabling it requires a non-empty `agent.status_inference_prompt`, at least one `agent.statuses` entry, and both `failure_status` and `background_status`. Each configured `kind` must name an entry in `agent.statuses`, and each `note` must be a non-empty single line. A worker failure records `failure_status` only if the stopped session remains unchanged and has no explicit status, unless shadow mode is active. When pending background work is the applicable Stop reason and no higher-priority condition is active, the hook records `background_status` without running the classifier. Invalid enabled settings do not prevent config loading; the Stop hook logs a warning and disables Codex inference until settings are valid.

Set `agent.codex_status_inference.shadow_mode` to `true` to classify without writing an inferred status. While the stopped session remains unchanged and has no explicit status, the worker logs the inferred kind and note, or the configured failure kind and note if inference fails. When pending background work is the applicable Stop reason and no higher-priority condition is active, the Stop hook logs the configured background kind and note. Shadow mode does not write inferred statuses to the session database.

The Codex fork uses the existing Codex OAuth session and a temporary loopback relay that forwards only to the official Responses endpoint. The relay sets `tool_choice` to `none` for HTTP and WebSocket requests and rejects any response that contains a tool call. It keeps request bodies and authorization headers in memory and does not persist them. The fork disables Codex hooks and armyknife hook registration, and its sandbox limits writes to the temporary inference directory. The relay uses the parent session ID for cache affinity; cache reuse is not guaranteed.

`agent.bg_task_reminder.timeout` is an optional duration such as `17m`; when omitted, background task reminders are disabled. `message` overrides the reminder text. Entries in `excluded_commands` are substring matches against the command line and skip the reminder for matching commands.

For editor autocompletion, add the following to the top of your config file:

```yaml
# yaml-language-server: $schema=https://raw.githubusercontent.com/fohte/armyknife/master/docs/config-schema.json
```

### Example

```yaml
# yaml-language-server: $schema=https://raw.githubusercontent.com/fohte/armyknife/master/docs/config-schema.json

agent:
  # Sent to the delegator after a delegated PR merges; omitted means no notification.
  merge_notification: 'Branch {branch} merged: {pr_url}'
  statuses: # user-defined status names mapped to display behavior
    needs_attention: { state: user }
    awaiting_build: { state: waiting }
    work_finished: { state: done }
  # Used to infer a missing status from a Claude Code fork or Codex fork.
  status_inference_prompt: |
    Classify the session's current state based on its saved conversation.
    Explain the next step in the note.
  codex_status_inference:
    enabled: true
    shadow_mode: true
    failure_status:
      { kind: needs_attention, note: 'status inference failure investigation' }
    background_status:
      { kind: awaiting_build, note: 'background task completion' }
  # Used by Codex without valid inference settings and Claude without a prompt.
  stop_hook_instruction: 'Summarize the result before stopping.'
  bg_task_reminder:
    timeout: 17m
    message: 'This is a reminder, not an error. Check the task output and monitored state, then stop it if it is stuck.'
    excluded_commands: ['human-review-tool', 'release-check --approval']
  work_type_fallback: # icon and color for watch rows without a recorded type
    icon: '?'
    color: gray
  vibe_marker: # icon and color for vibe sessions in watch rows
    icon: '*'
    color: 123
  work_types: # per-skill icon and color metadata
    example-workflow:
      icon: '◇'
      color: cyan
  default_engine: claude # coding agent CLI for `a agent new` when `--type` and `--engine` are omitted (default: "claude")
  codex: # defaults for `a agent new --engine codex` only; the `codex` you run yourself is unaffected
    model: example-codex-model # used when `--model` is omitted
    reasoning_effort: max # low | medium | high | xhigh | max; applied when `--reasoning-effort` is omitted
  active_profile: example-profile # selected by `a agent profile use`
  profiles: # execution settings keyed by profile name and type; model and reasoning_effort are optional
    example-profile:
      implementation:
        engine: codex
        model: example-codex-model
        reasoning_effort: high
      documentation:
        engine: claude
        model: example-claude-model
        reasoning_effort: medium
  launch_profiles: # per-kind engine and launch defaults for `a agent new --kind`
    sample-flow:
      engine: codex # selected when `--engine` is omitted
      claude:
        model: claude-example-model
      codex:
        model: codex-example-model
        reasoning_effort: high
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
  example-org:
    vibe: true # allow vibe-gated actions in this owner's repositories (default: false)

repos: # per-repository overrides, keyed by "owner/repo"
  fohte/dotfiles:
    language: en # language for commit messages and PR content (default: "ja" for private repos, "en" for public repos)
    direct_commit: true # allow direct commits to the default branch; consumed by external git hooks
    ai:
      review:
        reviewers: [devin, coderabbit] # repo-level reviewer override (takes precedence over org)
  example-org/example-repo:
    vibe: false # override the org vibe setting (default: inherit)
  example-org/standalone-tool:
    standalone_fork: true # independently maintained fork; consumed externally, only stored and exposed by armyknife
```

`agent.work_types` associates exact skill names with icon and color metadata. `agent.work_type_fallback` supplies the icon and color shown by `a agent watch` for sessions without a recorded work type; if omitted, those rows remain blank. Unconfigured recorded names remain blank. When a session starts without `a agent new --kind <skill>`, hooks record configured main-thread skill invocations as the session work type; subagent skill calls do not change it. A session started with `--kind` keeps that kind when hooks detect other skills. Configured icons appear before session titles in `a agent watch`. The mapping does not trigger skill detection. Colors accept the snake_case names `reset`, `black`, `red`, `green`, `yellow`, `blue`, `magenta`, `cyan`, `gray`, `dark_gray`, `light_red`, `light_green`, `light_yellow`, `light_blue`, `light_magenta`, `light_cyan`, and `white`, RGB triplets (`[r, g, b]`), or numeric indices from `0` through `255`.

`agent.vibe_marker` controls the icon and color for vibe-related sessions in the TASKS tree. Its icon appears after the name, unlike work type icons, and it uses the same icon and color formats as `agent.work_types`. Sessions that enabled vibe themselves use the configured color; sessions with a vibe-enabled ancestor show the icon dimmed. The dimmed marker shows the ancestor's state; a session inherits vibe mode when it was launched with `a agent new` while its caller's effective vibe was enabled, or when an enabled ancestor granted inheritance. If unset, no vibe marker is shown.

Without `--type`, `agent.launch_profiles` maps exact `--kind` values to launch defaults. Each entry can select an `engine` and set `model` and `reasoning_effort` under `claude` and `codex`. An omitted engine falls back to `agent.default_engine`. An explicit `--engine` overrides the profile's engine and selects only that engine's settings. Explicit `--model` and `--reasoning-effort` values override the selected engine's profile values; Codex then falls back to `agent.codex` for any unset value. When no matching profile exists, launches keep the existing global defaults.

`agent.profiles` maps arbitrary profile names to arbitrary type names. Every type requires an `engine` (`claude`, `codex`, or `agy`); `model` and `reasoning_effort` are optional and remain unset when omitted, and a configured `model` must be non-empty when its type is selected. `agy` can be stored in profile type settings but is not a session engine; config loading accepts `reasoning_effort` with `agy`, and `a agent new --type` rejects the selected type because agy is not a session engine. `agent.active_profile` selects the profile used by `a agent new --type <type>`; `a agent profile use <profile>` updates that selection in `zz-agent-profiles.local.yaml`. The `--type` path uses only that type's settings, independent of `--kind`; explicit `--engine`, `--model`, and `--reasoning-effort` flags override their matching values for that launch only. A `--type` session launch requires a final `model` and `reasoning_effort` from the profile or matching CLI flags; it does not fall back to `agent.codex` or `agent.launch_profiles`. When `--engine` differs from the type's engine, `--model` is required. Calls without `--type` keep the existing launch resolution. `a agent profile show` lists only configured values. Add `--dry-run` with `--type` to print the effective values and their sources without starting a session or creating a worktree.

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

When a review command runs inside tmux, the editor opens in a floating pane over the invoking pane and uses the review title as its pane title. The editor runs in a `shpool` session, so closing the pane only detaches it and the editor keeps running; the review completes when the editor exits. This requires tmux 3.7 or later and `shpool` on `PATH`; otherwise, the review command fails. Run it outside tmux to use the configured `editor.terminal` window. A closed editor can be brought back with `a agent crit open` (see below); outside an agent pane the review is not queued and cannot be re-opened that way.

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

| Action   | Description                                                                                                                                                                                                       |
| -------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `new`    | Create a new PR body draft file                                                                                                                                                                                   |
| `review` | Open the draft in editor for review (blocks until editor closes, exits 0 if steps changed, 1 if not, 2 if editor already open, 3 if terminal launch fails, 4 if the review editor session ends before completion) |
| `submit` | Create or update a PR from a draft (approval required unless `--no-review` is used in vibe mode) and cache its URL when the target is the current repo                                                            |

`new` options:

| Option              | Description                                                                                              |
| ------------------- | -------------------------------------------------------------------------------------------------------- |
| `--title <title>`   | PR title                                                                                                 |
| `--force`           | Overwrite existing draft file if it exists                                                               |
| `-R, --repo <repo>` | Target repository (owner/repo) instead of cwd's. Requires `--branch`                                     |
| `--branch <name>`   | Target branch instead of cwd's current branch (can be used alone to target another branch in cwd's repo) |

`submit` options:

| Option         | Description                                                                      |
| -------------- | -------------------------------------------------------------------------------- |
| `--base <ref>` | Base branch for the PR                                                           |
| `--draft`      | Create as draft PR                                                               |
| `--no-review`  | Skip approval verification (requires vibe mode and an allowed target repository) |

#### `a ai pr-series`

Register a task's PR split plan and synchronize its diagram into linked PR bodies.
Both commands require the `tq` CLI to be installed and available on `PATH`.

When the current branch has both `branch.<branch>.x-tq-task-id` and `branch.<branch>.x-tq-checklist-item-id` configured, `a ai pr-draft submit` links the submitted PR and synchronizes linked PR diagrams if a saved plan exists. This is best effort: failures are reported on stderr without failing the PR submit. Follow the printed recovery commands to relink the checklist item if needed and rerun `a ai pr-series sync <task-id>`.

When updating an existing PR, `submit` keeps its `pr-series` section if the draft does not include one.

```sh
a ai pr-series set <task-id> --file plan.json
a ai pr-series sync <task-id>
```

`set` validates the plan and tq checklist leaf IDs, then stores it in armyknife's per-user state so every worktree on the same machine can use it. Each `summary` must fit in one line below Mermaid's 200px label width. The JSON file contains the series name and purpose, and a `pullRequests` array with each PR's `id`, one-line `summary`, `dependsOn` IDs, `repository` (`owner/repo`), and `tqLeafId`:

```json
{
  "name": "Notification preferences",
  "purpose": "Let users choose which notifications they receive.",
  "pullRequests": [
    {
      "id": "schema",
      "summary": "Add preference tables",
      "dependsOn": [],
      "repository": "acme/notify-api",
      "tqLeafId": "<schema-leaf-id>"
    },
    {
      "id": "settings",
      "summary": "Add the settings page",
      "dependsOn": ["schema"],
      "repository": "acme/notify-web",
      "tqLeafId": "<settings-leaf-id>"
    }
  ]
}
```

`sync` finds linked PRs through the tq checklist and updates only the section between `<!-- pr-series:start -->` and `<!-- pr-series:end -->`. It adds the section if missing, leaves titles and the rest of each body unchanged, and can be rerun after partial failures. Public PR bodies omit PRs from private repositories.

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

| Command  | Description                                                                                                                                                                             |
| -------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `view`   | View issue and comments (read-only, no local cache)                                                                                                                                     |
| `pull`   | Fetch issue and save locally                                                                                                                                                            |
| `review` | Review a file before pushing (opens editor, exits 0 if approved, 1 if not, 2 if editor already open, 3 if terminal launch fails, 4 if the review editor session ends before completion) |
| `push`   | Push local changes to GitHub (requires file approval via review)                                                                                                                        |
| `diff`   | Show colored diff between local changes and remote                                                                                                                                      |
| `init`   | Create boilerplate files for new issues or comments                                                                                                                                     |

Each pulled issue directory contains `issue.md` for editable issue fields and
body, `.issue.base.md` as a managed issue snapshot, `.comments.base.json` as a
snapshot of comment IDs and bodies, and `comments/` for issue comments. When the
issue snapshot exists, `push` compares title, body, labels, sub-issues, and parent
issue against it. Remote-only issue field changes are preserved. Without
`--force`, edits to the same field on both sides with different values stop the
push. With a comment snapshot, `push` and `diff` compare local comment bodies
against the snapshot, preserving comments added remotely and remote-only edits.
Conflicts are reported when both sides changed a comment differently. Deleting
a comment still requires `--allow-delete`, and editing another user's comment
still requires `--edit-others`. Use `pull --force` to keep remote values or
`push --force` to overwrite them; an applied push still requires approval for
each changed file. Directories without an issue snapshot keep legacy issue-field
detection and body/title timestamp conflict checks. Directories without a
comment snapshot keep legacy two-way comment detection and timestamp conflict
checks until the next pull saves one.

| Option           | Description                                                          |
| ---------------- | -------------------------------------------------------------------- |
| `-R <repo>`      | Target repository (default: current repo)                            |
| `--dry-run`      | Show what would be changed without applying                          |
| `--force`        | Bypass conflict checks; applied local changes still require approval |
| `--edit-others`  | Allow editing other users' comments                                  |
| `--allow-delete` | Allow deleting comments removed locally                              |

##### `a gh issue-agent init issue`

Create a new issue boilerplate file. Fetches issue templates from the repository if available.

| Option              | Description                                       |
| ------------------- | ------------------------------------------------- |
| `--list-templates`  | List available issue templates and exit           |
| `--template <NAME>` | Use a specific issue template by name             |
| `--no-template`     | Use default boilerplate (skip template selection) |

The boilerplate includes `submit: false` as the review approval marker. `init issue` prints the created `issue.md` path as `Created: <path>`. Edit that file, then run `a gh issue-agent review <path>` and set `submit: true` in the editor. After approval, run `a gh issue-agent push <new-dir>`, where `<new-dir>` is the directory containing `issue.md`. A normal `push` requires this approval; `--dry-run` skips the approval check and does not create an issue.

The frontmatter accepts `parentIssue` and `subIssues`, so `push` can create an issue and link it via the Sub-issues API in one step. `submit` is approval metadata and is not sent to GitHub:

```yaml
---
title: Child Issue
submit: false
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

| Option               | Description                                        |
| -------------------- | -------------------------------------------------- |
| `-R, --repo <REPO>`  | Target repository (owner/repo)                     |
| `--include-resolved` | Include resolved threads                           |
| `--force`            | Overwrite even when local data cannot be preserved |
| `-d, --open-details` | Expand `<details>` blocks in comment bodies        |

Repeated pulls preserve draft replies and resolve selections for threads included in the refreshed file. Pull stops if the file cannot be parsed or a local reply or unresolved resolve selection would be discarded; pass `--force` to allow that overwrite.

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

New comments since pull conflict only on threads with a draft reply or resolve
action; `--force` overrides the conflict. Approval (`submit: true`) is required
only when changes are applied, so `--dry-run` and no-change pushes skip approval.

##### `a gh pr-review reply review`

Open the local threads.md in an editor for review. Setting `submit: true` in the frontmatter and saving marks the replies as approved. Run `reply push` afterwards to push. Exits 0 if approved, 1 if not, 2 if editor already open, 3 if terminal launch fails, 4 if the review editor session ends before completion.

Requires `reply pull` to have been run first.

```sh
a gh pr-review reply review <pr-number> [options]
```

| Option              | Description                    |
| ------------------- | ------------------------------ |
| `-R, --repo <REPO>` | Target repository (owner/repo) |

#### `a gh pr-merge`

Safely squash-merge an open pull request with no merge conflicts after running the optional `pre-pr-merge` hook and waiting for required checks to finish successfully. The merge is aborted if checks fail or the head commit changes. After the merge command succeeds, the command closes the matching local head-branch worktree and its agent session using the same safety checks as `a agent close`; cleanup failures are reported without changing the merge result. Cleanup is skipped unless exactly one worktree for the head repository and branch can be identified, or when the target session is the caller's current agent session. Inside an agent session, merging requires vibe mode and an allowed target repository. Outside an agent session, repository vibe settings do not restrict the command.

```sh
a gh pr-merge <pr-number> [options]
```

| Option              | Description                    |
| ------------------- | ------------------------------ |
| `-R, --repo <repo>` | Target repository (owner/repo) |

### `a agent`

Claude Code session monitoring with tmux integration. The canonical command is `a agent` (alias `a ag`); `a cc` is kept as a hidden backward-compatible alias, so existing hook and tmux configs that invoke `a cc ...` keep working unchanged.

| Action                                                     | Aliases | Description                                                                                 |
| ---------------------------------------------------------- | ------- | ------------------------------------------------------------------------------------------- |
| `new [--worktree[=<branch>]] [options]`                    |         | Start a Claude Code session, optionally in a new worktree                                   |
| `codex [<args>...]`                                        |         | Start Codex and bind its thread ID to the current tmux pane                                 |
| `close [target] [--child] [--force] [--skip-hooks]`        | `c`     | Close an agent session and its linked worktree                                              |
| `clean [--dry-run] [--all] [--force]`                      |         | Delete merged or closed worktrees                                                           |
| `hook <event>`                                             |         | Record coding agent hook events                                                             |
| `attribution pre-tool-use <provider>`                      |         | Pass through the existing provider permission proxy result                                  |
| `list`                                                     | `ls`    | List all Claude Code sessions with status                                                   |
| `focus <session_id> [--review-pane-id <pane_id>]`          |         | Focus the review pane when available, then the session's tmux pane                          |
| `mark-read [-t <pane_id>]`                                 |         | Mark the pane's session as read (wire from tmux `pane-focus-in`)                            |
| `status set <kind> <note>`                                 |         | Set the calling session's reason for stopping                                               |
| `profile use <profile> / show`                             |         | Select a profile and show its effective settings for every type                             |
| `vibe on / off / status / grant <session-id>`              |         | Enable, disable, check, or grant vibe mode to a descendant session                          |
| `resume [session_id]`                                      | `r`     | Resume the pane's Claude Code session (reads pane option if no argument)                    |
| `resurrect save`                                           |         | Save pane session IDs for tmux-resurrect (run from post-save hook)                          |
| `resurrect restore`                                        |         | Restore pane session IDs and relaunch Claude Code (from post-restore)                       |
| `peer parent`                                              |         | List the session that delegated to this one, if any (JSON)                                  |
| `peer children`                                            |         | List the sessions this one delegated to (JSON)                                              |
| `peer adopt <session_id> [--force]`                        |         | Adopt an existing session as a direct child                                                 |
| `peer list [-R <repo>]`                                    |         | List tracked sessions, with their SendMessage names (JSON)                                  |
| `peer me`                                                  |         | Print the session running in the caller's own tmux pane (JSON)                              |
| `peer wake <session_id>`                                   |         | Resume a paused peer session; print its SendMessage name (Claude only)                      |
| `peer notify <session_id> -m <text>`                       |         | Send a message to another session (SendMessage socket / Codex app-server)                   |
| `peer notify --parent -m <text>`                           |         | Send a message to the session that delegated to this one                                    |
| `crit add <url>`                                           |         | Associate a crit review with the calling session                                            |
| `crit open [--session <id> \| --pane <pane_id>]`           |         | Show or switch the review of an agent pane in a tmux floating pane                          |
| `crit close [--pane <pane_id>]`                            |         | Close the shown review of an agent pane (the review keeps running)                          |
| `bg run -- <cmd> [args...]`                                |         | Run a command detached and notify this session when it finishes                             |
| `bg reminder-hook`                                         |         | Internal PostToolUse hook for Claude Code background Bash reminders                         |
| `sweep`                                                    |         | Reconcile failed or interrupted Codex turns, pause idle sessions, and close merged sessions |
| `auto-compact schedule --session <id>`                     |         | Detached worker spawned by the Stop hook (not for direct use)                               |
| `status-inference --session <id> --updated-at <timestamp>` |         | Detached hidden worker spawned by Stop with its RFC 3339 timestamp (not for direct use)     |
| `window-status <window_id>`                                |         | Print status symbols for the sessions in a tmux window                                      |
| `pane-has-paused <pane_id>`                                |         | Print `1` when the pane holds a Paused Claude Code session, else empty                      |

#### AI commit attribution

`a agent attribution pre-tool-use <claude-code|codex>` reads a JSON envelope from stdin and returns its `hook_output` unchanged. It emits no output when `hook_output` is `null` or unavailable. The command remains as a compatibility entrypoint for existing provider proxy registrations and does not inspect or modify commands or permission decisions.

The `prepare-commit-msg` hook reads `CLAUDE_CODE_SESSION_ID` before `CODEX_SESSION_ID` to load the recorded session; when both variables contain non-empty IDs, Claude's ID takes precedence, and an empty Claude ID falls back to Codex's ID. A trailer-safe model adds `Co-authored-by: Codex ({model}) <noreply@openai.com>` or `Co-authored-by: Claude Code ({model}) <noreply@anthropic.com>`; the model ID is used without normalization. A non-empty, trailer-safe effort is included after the model as `({model}, {effort})`. The hook adds no attribution when the session ID, record, or usable model is unavailable. Existing `Co-authored-by` trailers are preserved, and an identical trailer is not added twice. Legacy `AI-Tool`, `AI-Model`, and `AI-Model-Reason` footer trailers are removed when attribution is added. See the [Claude Code attribution documentation](https://code.claude.com/docs/en/settings-reference#attribution-commit).

Human commits without a matching agent session remain unchanged. If session loading fails, or message replacement fails, the original message remains unchanged and a diagnostic is written to stderr, allowing Git to continue. Existing author, committer, signature, and unrelated trailers are left alone.

Existing provider proxies can pass their PreToolUse envelope through `a agent attribution pre-tool-use <provider>` and use the returned response as the provider's hook response. Existing Git hook scripts can run `a agent attribution prepare-commit-msg "$1"` from `prepare-commit-msg`. The `commit-msg` and `reference-transaction` commands remain as no-op compatibility entrypoints for older hook registrations. The legacy `--unknown-reason` and `--final-object` options are accepted but have no effect.

Only provider metadata attribution is performed. Git command permissions, aliases, compound commands, pull, merge, and rebase remain under the existing permission proxy and Git configuration.

Run `a agent status set` from a tracked Claude Code or Codex session. Claude Code gets `ARMYKNIFE_SESSION_ID` from the registered `session-start` hook; Codex provides `CODEX_SESSION_ID`. When `CODEX_THREAD_ID` differs from `CODEX_SESSION_ID`, the command succeeds without saving a status because Codex subagent threads share the root session record. The one-line note is cleared on the next `UserPromptSubmit` or the first main-thread activity hook after `Stop`; `Stop`, `SessionEnd`, notifications, and `SessionStart(resume)` keep it. A set status appears as `agent_status` in `a agent peer` JSON output.

Vibe mode is an opt-in marker recorded by armyknife. Vibe-gated actions are denied for every repository by default, even when the marker is enabled. Set `orgs.<owner>.vibe` to `true` to allow repositories for an owner; `repos.<owner>/<repo>.vibe` can override that value for one repository. The effective repository value defaults to `false`.

`a ai pr-draft submit --no-review` requires the marker and an allowed target repository. `a agent new --vibe` remains an explicit guard that requires the caller's effective vibe and an allowed target repository. `a gh pr-merge` applies both checks inside agent sessions. Hooks, skills, and prompt instructions can branch on `a agent vibe status`, which succeeds when the current session ran `vibe on`, or when it has an inheritance marker and a recorded ancestor still has vibe enabled. It does not check repository access:

```console
if a agent vibe status >/dev/null 2>&1; then
  # Continue with the vibe workflow.
fi
```

A session also enables the marker when its main-thread user prompt starts with `/enable-vibe-coding` in Claude Code or `$enable-vibe-coding` in Codex.

Run `a agent vibe on` in a tracked session to enable the marker. A new descendant automatically receives inheritance eligibility when `a agent new` runs while its caller's effective vibe is enabled; this records eligibility without copying the caller's explicit `on` state. `a agent vibe grant <session-id>` remains available for existing descendants, including sessions launched before an ancestor was enabled or after adoption. Inheritance remains active only while a recorded ancestor's session record still has vibe enabled. If armyknife removes that ancestor record during session cleanup, the descendant no longer inherits the marker. `a agent vibe off` clears only the calling session's explicit marker; descendants lose inherited status when the enabling ancestor turns it off.

When a Claude Code `Stop` hook cannot infer why the agent stopped and no status is set, `agent.status_inference_prompt` starts a detached worker that forks the session and fills the status without spending another turn. The result is stored only after its kind and note pass validation and only if no status has been set and no new turn has started. Fork failures, timeouts, and invalid output are logged and leave the status unset. The fork is non-persistent; it keeps tool definitions available while a global ask rule and `dontAsk` reject tool requests, and disables all hooks.

When Codex inference is enabled, the Stop hook immediately allows the turn to end and starts a detached worker. The worker preserves the Stop payload's `turn_id` and final assistant message, waits for that turn to finish and the message to appear in app-server history, then starts an ephemeral `codex exec fork` with the parent's model and saved conversation. The child alone routes HTTP and WebSocket inference requests through a temporary loopback relay to the official Responses endpoint. The relay sets `tool_choice` to `none`, rejects tool-call responses, and uses the parent session ID for cache affinity. The fork disables Codex hooks and armyknife hook registration, and its sandbox limits writes to the temporary inference directory. A valid result is stored only if the session still matches the stopped timestamp and has no explicit status; the status update checks those conditions again while holding the session lock. Acquisition, relay, classification, invalid output, timeout, and spawn failures record the configured failure status only if the stopped session remains unchanged and has no explicit status, unless shadow mode is active. Shadow mode logs the result without writing an inferred status. When pending background work is the applicable Stop reason and no higher-priority condition is active, the hook uses the configured background status without starting classification.

Status inference uses a private snapshot of the parent conversation and file-backed credentials. The worker creates read-only online backups of the SQLite databases and copies the required conversation lineage into its private home; a WAL database without readable `-wal` and `-shm` sidecars fails closed. The private auth snapshot omits the refresh token and is used only when the access token remains valid beyond the snapshot and fork safety window. The fork keeps its state and temporary writes in that snapshot, disables hooks and armyknife session registration, and retains MCP tool definitions while the relay rejects tool calls. The `agent.status_inference.usage` event records numeric input, cached-input, and output counts from official Responses usage with session correlation; it omits response bodies, credentials, and fork-process cumulative counters. Unsupported or inconsistent snapshots follow the configured failure-status path, and temporary resources are removed when the worker exits.

When inference is disabled in a valid config, Codex uses `agent.stop_hook_instruction`. If inference is enabled with incomplete settings, config loading succeeds; the Stop hook logs a warning, disables Codex inference, and uses `agent.stop_hook_instruction` when configured. If another config parsing error prevents loading, the Stop hook uses defaults, which have no statuses or instruction, and allows Stop without setting a status. Background work, pending permission prompts, `AskUserQuestion`, an existing status, paused sessions, and `stop_hook_active` are recognized automatically and do not trigger inference or blocking. A blocked stop keeps the session running and skips stop notifications and auto-compaction. To require status reports from sessions without inference, configure an instruction that asks the agent to run `a agent status set <kind> <note>` with a name from `agent.statuses`.

Define status names under `agent.statuses`; the configured state controls how each status is shown:

- `user`: the user has a turn; the status name and note explain what they need to do
- `waiting`: another session, CI job, or external event needs to finish
- `done`: the work is finished

When introducing `agent.statuses`, include names already stored in session records to preserve their configured states. A stored name missing from the map is treated as `user` so it remains visible.

```console
$ a agent status set awaiting_build "waiting for a build"
```

[`crit`](https://github.com/tomasz-tomczyk/crit) review URLs passed to `a agent crit add <url>` must include an explicit port. When a tracked session is available, the command associates the URL with that session and sends a desktop notification. Its lifecycle monitor needs `crit` on `PATH`; without it, automatic cleanup is unavailable. If no tracked session ID is available, the command opens a review pane in the current tmux pane when `TMUX_PANE` is set; otherwise, macOS opens the URL with `open` and other platforms use `xdg-open`.

If recording the URL to a tracked session fails, the command warns and still attempts to display the review in a tmux pane or browser and send a notification.

`a agent crit open` shows the review waiting on an agent pane in a tmux floating pane, one at a time. Pending editor reviews (in start order) come first and the Crit review last. If none is shown, it shows the last one shown (or the first); if one is shown, it switches to the next, wrapping around. `a agent crit close` closes the shown review without ending it. It requires tmux 3.7 or later, `shpool`, and `terminal-browser` on `PATH`. A review that starts while another editor is shown waits hidden until it is switched to or the shown one ends, when the next one is shown. Reviews started outside an agent pane open right away and are not queued. In `a agent watch`, sessions with a review show a `[crit]` badge; press `o` to close watch and show the Crit review. The queue is mirrored to the pane options `@armyknife-review-queue` on the floating pane (for example `[comment 1] · comment 2 · crit`, with the shown review in brackets) and `@armyknife-review-count` on the agent pane (only while nothing is shown), and the `window-layout-changed` hook is re-run so tmux configuration can show them in the pane border:

```sh
a agent crit add http://127.0.0.1:12345/review
a agent watch
```

```tmux
set -g pane-border-format '#{?#{@armyknife-review-queue},#{@armyknife-review-queue},#{?#{@armyknife-review-count},review #{@armyknife-review-count},#{pane_index}}}'
```

After moving an agent pane to another tmux window, run `a agent crit close` and then `a agent crit open` to bring the review beside the moved pane. If another agent's Crit pane uses the same port, the command exits with an error instead of taking over that connection.

`a agent watch` shows YOUR TURN/TASKS in one tree. `Tab` explicitly opens or closes the selected session's implementation plan inline, and `j/k` moves one cursor through sessions, Batch headings, and PR items. Moving the cursor does not open another plan or change the row count. When opened from an implementation child session with a task ancestor, watch keeps the child selected and leaves the plan closed until `Tab` is pressed. Closing the plan keeps active implementation sessions in the tree. `Esc` clears an active filter or quits.

When tq data is available, implementation checklist items appear under Batch headings, including planned PRs and existing PRs. A checklist item with a matching implementation session combines both into one row. Unmatched sessions and their descendants keep their TASKS hierarchy. `Enter` focuses a real session's pane; it does nothing on a planned item. `o` opens a real PR for a progress item or session that has one; on other session rows it keeps the existing Crit action. While progress loads or fails, an inline status row distinguishes that state from a task with no implementation checklist, and session navigation remains available.

Worker and planned PR rows show one current stage: `implementing`, `crit`, `body`, `CI running`, `merge`, or `merged`. Stages waiting for user action (`crit`, `body`, and `merge`) use the red YOUR TURN label style. An existing PR marks implementation, diff review, and PR body done unless a linked review is active; otherwise implementation follows the session state. Linked Crit and PR-body review activity updates the stage. CI follows the latest commit's check rollup; merge follows PR state and CI progress. Child PR lookup uses saved repository and branch context when present; it consults the live worktree only when saved context is absent. A row says `(PR unavailable)` only when neither source can resolve a PR.

Fully merged batches start collapsed. `j/k` selects planned PR rows as well as existing PRs. On an implementation-plan or Batch heading, `h` collapses it, `l` expands it, and `Enter` toggles it. Press `h` on a PR item to collapse its Batch and select the Batch heading.

The session list shows YOUR TURN first, then TASKS: a tree of sessions where each root is a session without a displayed parent and descendants appear at their full depth beneath the nearest displayed parent. The root badge counts all displayed descendants. Every child session appears in TASKS, including sessions that cannot start their own children. Each TASKS row shows a mark: `✓` done, `◐` your turn, `●` working, `◷` waiting on something else (another session, CI, a background command); stops without a known reason keep the plain status glyph. Roots that contain your turn come first, then the most recently active. A session listed in YOUR TURN is also selectable in TASKS. Session state and titles update immediately. While navigating or scrolling the list, or editing a search, existing rows keep their order; two seconds after the last list interaction, the latest order is applied once. New sessions are appended under their parent. When rows disappear or stop matching a filter, the selection and its screen position are preserved where possible.

`a agent bg run -- <cmd> [args...]` returns immediately and runs the command in a detached worker. It stores stdout and stderr in separate files and prints their paths. On completion, it sends this session a `<background-task-complete>` message with the command, exit code, and output paths. If delivery fails, the worker saves the message under `agent-bg/undelivered/` in the armyknife cache directory, which defaults to `~/.cache/armyknife/` and uses `$XDG_CACHE_HOME/armyknife/` when set; a later `SessionStart(resume)` retries delivery for up to 30 seconds at one-second intervals. Messages remain saved if all attempts fail and are retried on the next resume. If the command exceeds `agent.bg_task_reminder.timeout`, the worker sends the configured reminder once unless the command matches `excluded_commands`. While the worker is active and the main loop is stopped, `a agent list` and tmux window status show `◎ background`, or `◐ waiting` while a crit review is linked or a Human-in-the-Loop review is waiting for approval; the `a agent watch` TASKS tree shows the background case as `◷`. Stop notifications and auto-compaction are suppressed, and `a agent sweep` leaves the session alone. `a agent clean` also preserves its worktree unless `--force` is set.

For Claude Code background Bash commands, add this entry alongside the existing `PostToolUse` hook in your Claude Code settings:

```json
{
  "hooks": {
    "PostToolUse": [
      {
        "matcher": "Bash",
        "hooks": [
          {
            "type": "command",
            "command": "a agent bg reminder-hook",
            "asyncRewake": true,
            "timeout": 1080
          }
        ]
      }
    ]
  }
}
```

Set the hook timeout in seconds to a value longer than `agent.bg_task_reminder.timeout` (the example above uses 18 minutes for the 17-minute config example). The existing `a agent hook post-tool-use` registration records each background Bash task ID, and the `Stop` hook in [Setup](#setup) refreshes that list from Claude Code's task registry at each Stop event. Both hooks must be registered for reminders to work. When the task remains pending after the configured timeout, the hook exits with status `2` and writes the reminder to stderr.

`a agent list --tmux` prints one next user action for a tmux status line and includes the session name at the end. The shared action order is `failed`, `crit`, `body`, user-state status names, `allow`, `ask`, then `merge`; configured status names are shown as labels. The oldest session update wins within each action. `waiting` and `done` statuses produce no user action.

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

`a agent close [target] [--child] [--force] [--skip-hooks]` (alias: `a ag c`) accepts a session ID, worktree name (branch name), or worktree path. With no target, it closes the session in the current pane; if the pane has no tracked session, it uses the current linked worktree. Closing a session in a linked worktree also removes that worktree and its branch only when no other listed session uses it. Another session counts as using it when its cwd is inside the worktree and its tmux pane is alive or cannot be confirmed dead, including sessions without pane info. If another session uses it, only the target session is closed and the worktree is kept, even with `--force`. This rule also applies to `--child` targets. A worktree target closes all tracked sessions in that worktree that are eligible for close. A `--child` worktree target with no matching tracked session is rejected because its parent relationship cannot be verified. It validates every candidate before shutting down the first session; if a validation fails, no session is shut down and the worktree remains. A session on the caller's current pane is not shut down. `--child` requires a target and must be run from a tracked Claude Code or Codex session identified by `ARMYKNIFE_SESSION_ID` or `CODEX_SESSION_ID`; every matching session must be a direct child of the calling session, as listed by `a agent peer children`. `--force` does not bypass this check.

The command sends Ctrl+D and waits up to five seconds for the agent to exit, then sends SIGTERM if needed. It removes the pane after confirming the agent has exited. Unless `--force` is passed, it refuses sessions that are running, waiting for input, have pending tasks, or contain an unsent draft. If a live agent process is found but the pane cannot be parsed to check for a draft, close protects the session while the elapsed time since `session.updated_at` is within the configured auto-pause timeout; it proceeds when that time has elapsed. When closing a worktree, sessions without tmux information receive the same running, waiting-for-input, pending-task, and recent-activity checks unless they are paused or `--force` is passed. For a running Codex session, it checks the latest app-server turn first and allows closing after an interrupted turn when no permission request is pending. An active turn, a pending permission request, or an unavailable app-server keeps the session protected. If no agent process is found, it allows closing only for sessions marked ended or paused unless `--force` is passed. Even with `--force`, it refuses when the pane is no longer bound to that session. For worktree cleanup, `--force` also skips the unmerged-branch confirmation, and `--skip-hooks` skips the `pre-worktree-delete` and `post-worktree-delete` hooks. An already-removed pane is treated as closed.

`new` options:

| Option                                               | Description                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| ---------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `--worktree[=<branch>]`                              | Create a worktree for the branch and run the session there, opening a new tmux window. Branch is auto-generated from `--prompt` when the value is omitted; opens `$EDITOR` to write a prompt when both are omitted. When the flag itself is omitted, no worktree is created: the session runs in the current directory (or the root of the worktree containing `-R`) -- see below for whether that starts a pane split or a new tmux window |
| `--from <ref>`                                       | Base branch for new branch creation (requires `--worktree`; default: origin/main or origin/master)                                                                                                                                                                                                                                                                                                                                          |
| `--force`                                            | Force create new branch even if it already exists (requires `--worktree`)                                                                                                                                                                                                                                                                                                                                                                   |
| `--skip-hooks`                                       | Skip the `post-worktree-create` and `pre-session-start` hooks (works with or without `--worktree`)                                                                                                                                                                                                                                                                                                                                          |
| `--vibe`                                             | Explicitly require the caller's effective vibe and an allowed target repository; record inheritance eligibility in the new session                                                                                                                                                                                                                                                                                                          |
| `--child-launch <allow\|deny>`                       | Set whether the new session may launch child sessions (default: `allow`)                                                                                                                                                                                                                                                                                                                                                                    |
| `-R, --repo <path>`                                  | Target repository or linked worktree path. Without `--worktree`, the session runs from the root of the worktree containing this path; with `--worktree`, new worktrees are created from the main repository root                                                                                                                                                                                                                            |
| `--prompt <text>`                                    | Initial prompt to send to the agent (Claude Code or Codex)                                                                                                                                                                                                                                                                                                                                                                                  |
| `--label <title>`                                    | Label for the new session (displayed in `agent watch`)                                                                                                                                                                                                                                                                                                                                                                                      |
| `--kind <skill>`                                     | Initial work type for the launched session, pinned to the exact skill name. A matching `agent.work_types` key displays its icon and color; without `--type`, a matching `agent.launch_profiles` key supplies launch defaults                                                                                                                                                                                                                |
| `--type <name>`                                      | Use the active `agent.profiles` entry as the source for engine, model, and reasoning effort. Explicit matching flags override those values for this launch; a different `--engine` requires `--model`                                                                                                                                                                                                                                       |
| `--dry-run`                                          | With `--type`, print the selected profile, type, effective values, and each value's source without launching                                                                                                                                                                                                                                                                                                                                |
| `--model <model>`                                    | Model for the new session, passed through to `--model` of the selected engine CLI; accepts whatever that CLI supports. With `--type`, overrides its model. Without `--type`, defaults to the selected engine's `agent.launch_profiles.<kind>` value, then `agent.codex.model` when the selected engine is Codex                                                                                                                             |
| `--parent-session-id <id>`                           | Parent session ID for tree view hierarchy                                                                                                                                                                                                                                                                                                                                                                                                   |
| `--engine <claude\|codex>`                           | Coding agent CLI to launch. With `--type`, overrides its engine; if different, also pass `--model`. Without `--type`, defaults to `agent.launch_profiles.<kind>.engine`, then `agent.default_engine`, itself defaulting to `claude`. With `--worktree`, see below                                                                                                                                                                           |
| `--reasoning-effort <low\|medium\|high\|xhigh\|max>` | Reasoning effort for this launch only. Passed as `claude --effort <effort>`; for Codex, sent in the first `turn/start` on the daemon route and passed as `-c model_reasoning_effort=<effort>` otherwise. With `--type`, overrides its reasoning effort. Without `--type`, defaults to the selected engine's `agent.launch_profiles.<kind>` value, then `agent.codex.reasoning_effort` when the selected engine is Codex                     |

`a agent profile use <profile>` selects a configured profile for future `a agent new --type <type>` launches, including the next launch, until another profile is selected or `agent.active_profile` is changed. The selection is stored in `$XDG_CONFIG_HOME/armyknife/zz-agent-profiles.local.yaml` (or `~/.config/armyknife/zz-agent-profiles.local.yaml` when `XDG_CONFIG_HOME` is unset); only `active_profile` is updated. `a agent profile show` prints the selected profile and all its type settings. Run `a agent new --type <type> --dry-run` to inspect the exact values and their sources before launching.

When invoked from a tracked session with `child_launch` set to `deny`, `a agent new` returns an error. This also applies to the `a ag new` and `a cc new` aliases. Calls without a tracked caller session ID, such as from a normal shell, continue to start sessions.

Worktree creation in the same repository is serialized through the `post-worktree-create` and `pre-session-start` hooks. Concurrent calls wait for the hooks or their rollback to finish.

Without `--worktree`, `a agent new` compares the target repo (from `-R`, or the current directory) against the repo of the invoking session. When they match and the caller pane resolves from `$TMUX_PANE` or the invoking agent session's recorded tmux pane, it splits that pane into a new pane in the same window. Otherwise -- the repos differ, or no caller pane can be resolved -- it opens a new tmux window in the target repo's own tmux session.

`a agent new` keeps the current focus when `CLAUDECODE` is set or the caller has a non-empty `ARMYKNIFE_SESSION_ID` or `CODEX_SESSION_ID`. This applies to pane splits and new windows, regardless of the child engine or child-launch permission; a tmux shell without those agent indicators switches focus as before.

For the default Claude engine, `--prompt` normally launches `claude [--model <model>] [--effort <effort>]` without putting the prompt on the pane's command line. With exactly one Claude pane, armyknife identifies the new session by its tmux location in Claude Code's session registry and delivers the prompt through its messaging socket.

With `--prompt`, Claude messaging requires exactly one Claude pane. A layout with a different number of Claude panes fails before opening the window or pane. If armyknife cannot resolve the pane's tmux location, the messaging socket does not appear within 20 seconds, or delivery fails, the command exits with an error that identifies the pane; the pane remains open without the initial prompt.

`--engine codex` launches `codex` instead of `claude` (pane command and, without `--worktree`, window-name placeholder). The launch route depends on the prompt and app-server availability:

- Without `--prompt`, armyknife runs `codex [--model <model>] [-c model_reasoning_effort=<effort>]`.
- With `--prompt`, exactly one Codex pane, and a running shared Codex app-server that exposes its control socket, armyknife connects before opening the pane. It launches `codex [--model <model>]` without config overrides, then sends the prompt and reasoning effort in the first `turn/start` request. If the app-server is unavailable or the layout does not have exactly one Codex pane, the command exits with an error before opening the window or pane.
- If the app-server rejects the initial `turn/start` request, or the turn ends before the prompt is recorded, the command exits with an error that identifies the pane; the pane remains open without the prompt. If `turn/start` was sent but its response or the prompt's recording could not be confirmed, armyknife binds the pane to the created thread, reports a warning, and does not retry the prompt through argv.

The daemon route marks its Codex pane as armyknife-managed, so an environment where `codex` is aliased to `a agent codex` does not perform pane binding twice. A hand-run `a agent codex` keeps its normal pane binding behavior.

With `--worktree`, the session runs in `config.agent.worktree.layout`, whose pane commands are yours to write. Every pane running `claude` (e.g. `command: claude`) is replaced by plain `codex`, dropping its arguments because they are Claude Code flags. The layout is left as written when it already has a `codex` pane, and panes running anything else are never touched. With `--prompt`, the layout must contain exactly one Codex pane because `thread/started` does not identify its originating pane.

A session's engine is recorded on first hook event (see `--engine` on `a agent hook` below) and later read back by `a agent resume` to decide which binary to relaunch. An explicit `a agent resume --engine` value takes precedence when tmux-resurrect restores a snapshot whose store record is missing. `resume` never uses `agent.default_engine`, so changing the default doesn't affect resuming existing sessions.

When `--type` is omitted, `agent.default_engine` supplies the engine if `--engine` and the matching kind profile's `engine` are omitted. Set `agent.default_engine: codex` in config, or prefix a single invocation with `ARMYKNIFE_AGENT__DEFAULT_ENGINE=codex` (see [Environment variable overrides](#environment-variable-overrides)), to change this fallback (with `--worktree` too, so a `codex` fallback replaces the layout's `claude` pane). An explicit `--engine` always wins.

Without `--type`, `agent.codex.model` and `agent.codex.reasoning_effort` are the global defaults for `codex` when a selected kind profile leaves a value unset. Explicit flags and the selected engine's kind settings take precedence. The model is passed on the `codex` command line. The effort follows the launch routes above, so it is sent through `turn/start` or a launch-only `-c` override without modifying `~/.codex/config.toml`. These defaults never apply to `claude` sessions; an explicit `--reasoning-effort` does, as `claude --effort`.

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
    "PostModelSwitch": [
      {
        "hooks": [
          { "type": "command", "command": "a agent hook post-model-switch" }
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
    "StopFailure": [
      {
        "hooks": [{ "type": "command", "command": "a agent hook stop-failure" }]
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

These hooks record session state changes, enabling `a agent list` to display active sessions with their current status (running, waiting for input, stopped, or failed). `StopFailure` marks API-error interruptions as `Failed`; when no permission request remains, the next user prompt returns the session to `Running`. An unclassified `Stop` remains `Running` while the hook asks the agent to set its status.

The `SessionStart` and `UserPromptSubmit` hooks store the agent session ID in the tmux pane user option `@armyknife-last-agent-session-id`, so that `a agent resume` can relaunch the session inside that pane. Reading this option falls back to the pre-rename key `@armyknife-last-claude-code-session-id` when the current key is unset, since a pane only gets its option rewritten on those two hook events, and a pane holding a still-running session would otherwise lose its binding right after the rename.

Claude Code records the model from `SessionStart.model` or `PostModelSwitch.to_model`; if no model has been recorded, the last assistant message in the transcript supplies it. Hook events that include `effort.level` record that effort. Codex sessions also record model and effort metadata when available.

The pane is normally found by walking the hook process's ancestry up to it. A Codex session running through the shared `codex app-server` daemon breaks that link: the daemon runs the agent loop, hooks included, outside any pane's process tree, so armyknife falls back to the launcher-written session ID option. It binds only when exactly one pane has that session ID and is currently running the Codex CLI. The lookup accepts both `@armyknife-last-agent-session-id` and the pre-rename `@armyknife-last-claude-code-session-id` option; if there is no unique match, the session remains unbound.

`a agent hook <event> --engine <claude|codex>` (default: `claude`) records which coding agent CLI fired the hook; this is the engine `a agent resume` reads back to pick which binary to relaunch (see `--engine` on `new` above). Event names and JSON payload shape are the same regardless of engine: an allowed `stop` resolves to `Stopped` ("waiting for the next prompt"), while an unclassified stop remains `Running` until the agent reports its status. Both `permission-request` and `notification` with `notification_type: "permission_prompt"` resolve to `WaitingInput` ("waiting for tool-call approval") -- so a Codex hook configuration that maps Codex's own hook events onto these same `--event` values and payload fields gets the same approval-wait/input-wait distinction in `a agent list` for free.

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

Register `SubagentStop` separately to queue a finished child turn result for its direct parent:

```json
{
  "hooks": {
    "SubagentStop": [
      {
        "hooks": [
          { "type": "command", "command": "a agent codex-subagent-stop" }
        ]
      }
    ]
  }
}
```

The handler reads `session_id`, `agent_id`, and `turn_id` from hook stdin, then starts a detached worker. The worker polls up to 600 times at one-second intervals for that exact child turn to become terminal. A missing turn, `inProgress`, or `interrupted` status remains unresolved and is read again within the same limit. Completed and failed turns proceed to resolve the direct parent from `parentThreadId`, and queue a result when the parent's live `thread/read` status is `idle` or `active`. The queued message includes the child thread ID, turn ID, terminal status, and final answer or failure reason. When Codex starts the queued item, the parent can inspect the result in its text even if the native agent mailbox has not arrived yet; the mailbox may arrive first. Long results are explicitly truncated to the queue input limit while keeping both IDs and a notice to inspect the child turn for the full result. `thread/queue/add` leaves the parent's active turn and `wait_agent` untouched; Codex starts the queued item after the parent becomes idle. If the turn is still missing, in progress, or interrupted after polling, the worker exits without marking the event processed, so a later hook can retry it. A missing or unrecognized parent status, a known status other than `idle` or `active`, or a failed parent status read does not receive input. Repeated hooks for the same child turn are processed once.

The parent status check and queue request are separate operations. If the parent reads the result from its native mailbox before the queued item starts, the queue can still cause an extra turn after the active turn ends. Codex rejects queue submissions to a parent that is itself a multi-agent v2 subagent, so that parent cannot be resumed by this handler.

Codex has no `Notification` event, so nothing maps to `a agent hook notification`. A Codex `SessionEnd` hook is best-effort and may not fire; when it does, it records the session as `Stopped`, or keeps it `Paused` when sweep has requested shutdown. The app-server daemon also fires this hook during shutdown, including restarts. Exiting through `a agent codex` marks the thread `Ended` when a thread ID is bound to its tmux pane. Exiting through `a agent resume` marks the thread bound to its pane `Ended`, falling back to the requested resume thread when no pane binding is available. Both paths preserve `Paused` when sweep has already requested shutdown. `a agent close` also marks a Codex session `Ended` after confirming its exit, and a new thread taking over a pane ends the previous Codex session. A raw `codex` invocation does not provide process-exit confirmation; if its `SessionEnd` hook does not arrive, `a agent sweep` handles the session like any other stopped session. Codex has no equivalent of `CLAUDE_ENV_FILE`, so `ARMYKNIFE_SESSION_ID` is never set inside a Codex session; commands that need the caller's own session ID (`a agent new`'s parent resolution, `a agent peer parent`/`children`) fall back to `CODEX_SESSION_ID`, which Codex exports to every command it runs.

Both engines delay permission notifications by about one second because another `PermissionRequest` hook can resolve the request without showing the approval UI. A subsequent event that clears the pending request, such as `PostToolUse` or `Stop`, cancels the notification. Codex sessions with a top-level `approvals_reviewer = "auto_review"` in `$CODEX_HOME/config.toml` (default `~/.codex/config.toml`) wait through Codex v0.155.1's 90-second automatic approval review timeout before rechecking, so the notification is delayed by about 92 seconds in that case. Profile-scoped values are not read. Launch-only `codex -c` overrides and Apps connector reviewer overrides are resolved inside Codex and are not visible to armyknife: when the persisted global setting is `auto_review`, those cases use the same conservative delayed path; when it is not, the one-second path can notify while an override review is running. An approved command that runs longer than this window can still leave the hook state pending until `PostToolUse` and may produce a notification because hooks do not expose a separate running-tool signal.

#### Peer session name resolution

Claude Code's `SendMessage`/`ListAgents` tools address other sessions by an opaque `name` that Claude Code assigns internally and exposes nowhere else except `~/.claude/sessions/<pid>.json`. When several sessions share a working directory (e.g. many delegated `a agent new` sessions in the same worktree), the names in `ListAgents` are indistinguishable from the outside. `a agent peer` resolves the right name by joining armyknife's own session tracking (`ancestor_session_ids`, populated whenever `a agent new` resolves a parent session) against that registry file, so a session doesn't have to guess which `ListAgents` row is its parent or child.

`a agent peer parent`, `a agent peer children`, `a agent peer list [-R <repo>]` (filter by a substring of the session's working directory), and `a agent peer me` all print a JSON array of `{name, session_id, cwd, label, status, work_type, agent_status, pane_id, engine}`. `work_type` is the recorded skill name, or `null` when unset; callers can use it to select sessions by work type. `agent_status` is `{kind, note}` when set and omitted otherwise; `name` is `null` when Claude Code's registry has no matching entry, and `pane_id` is `null` when the session wasn't started inside tmux. `engine` is `"claude"` or `"codex"` (see `--engine` on `new` above), letting a caller juggling several peers tell which CLI a session belongs to before deciding how to reach it. `parent` and `children` are filtered subsets of `list`: `parent` has zero entries when this session has no tracked parent, `children` has zero entries when nothing was delegated to it.

```console
$ a agent peer parent
[{"name":"myproject-4f","session_id":"1111...","cwd":"/Users/example/ghq/github.com/example/myproject","label":null,"status":"running","work_type":"sample-work-type","agent_status":{"kind":"wait","note":"waiting for job-42"},"pane_id":"%3","engine":"claude"}]
$ a agent peer parent | jq -r '.[0].name // empty'
myproject-4f
$ a agent peer list -R myproject
[{"name":"myproject-9c","session_id":"2222...","cwd":"/Users/example/ghq/github.com/example/myproject/.worktrees/feature-x","label":"fix login bug","status":"running","work_type":"sample-work-type","pane_id":"%7","engine":"claude"},{"name":null,"session_id":"3333...","cwd":"/Users/example/ghq/github.com/example/myproject/.worktrees/feature-y","label":null,"status":"stopped","work_type":null,"pane_id":null,"engine":"codex"}]
```

Run `a agent peer adopt <session_id>` from a tracked Claude Code or Codex session to make an existing session its direct child. The command updates the target's `ancestor_session_ids` to the caller's ancestors followed by the caller, so `peer children` lists it and `a agent close --child <session_id>` accepts it. It prints nothing on success; list the adopted session with `peer children`. If the target already is a direct child, the command succeeds without writing. It refuses a missing or Ended target, the calling session itself, and any of the caller's ancestors because those would create a cycle. A target whose current parent record exists and is not Ended requires `--force`; a missing or Ended current parent does not. Adoption clears the target's inherited vibe marker, so an enabled ancestor must explicitly grant it again with `a agent vibe grant <session_id>`.

```console
$ a agent peer adopt session-x
$ a agent peer children | jq -r '.[].session_id'
session-x
$ a agent peer adopt session-y --force
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

`a agent peer notify <session_id> -m <text>` delivers a message to a Claude Code session's `SendMessage` socket directly (for a Codex target, see below), without any Claude Code session driving the call -- useful when the caller is a background process rather than another Claude Code session. Use `--parent` instead of `<session_id>` to send to the session that delegated to the caller; the current session and its recorded parent must both be tracked. When the caller's session ID is known, it refuses to notify that same session. It resumes a `Paused` target via the same flow as `peer wake` first, and refuses outright for an `Ended` session (the user terminated it intentionally). It fails loudly, rather than silently succeeding, when the target's registry entry has no `messagingSocketPath` -- this happens when the target session was started by a Claude Code build that predates peer messaging.

When the requested session ID is not tracked, the error may include up to three close, non-ended session IDs. These are suggestions only: `notify` never changes the recipient or sends to a suggested session automatically. If no close candidate is found, it keeps the plain `Session not found` error.

For a Codex target (see `engine` above), direct delivery requires the persistent Codex app-server for the target's `$CODEX_HOME` (default: `~/.codex`). It injects the message into an active turn immediately or starts a new turn when the thread is idle. The command prints which case applied, based on armyknife's tracked session status. If `turn/start` is explicitly rejected with `thread not found: <session_id>`, `notify` resumes the stored thread and retries the same message once. It returns a non-zero error when delivery is rejected or cannot be confirmed, and never registers a fallback queue item. A `Paused` Codex target is resumed before delivery is attempted.

`notify` identifies the sender automatically: it tries `ARMYKNIFE_SESSION_ID` (set by the Claude Code `session-start` hook), then `CLAUDE_CODE_SESSION_ID`, then `CODEX_SESSION_ID` (the ambient variables each CLI exports, which cover Codex sessions and Claude Code sessions whose hooks aren't registered), and wraps the message in a `<peer-message>` envelope naming whichever one resolves, since the underlying `SendMessage` protocol carries no sender field of its own -- without it, a session juggling several peers can't tell which one a message came from. When the resolved sender is a tracked session, the envelope also names its `engine` (`claude`/`codex`, see `peer parent`/`children`/`list`/`me` above), so the recipient knows whether to expect a `SendMessage`-capable reply. When it isn't tracked, a sender resolved via `CLAUDE_CODE_SESSION_ID`/`CODEX_SESSION_ID` still gets an `engine` guessed from that variable; one resolved via `ARMYKNIFE_SESSION_ID` has no such hint, so the line is omitted instead. When nothing resolves (e.g. `a agent close` calling `notify` directly, with no session in the loop), the message is delivered unwrapped.

```console
$ a agent peer notify 1111... -m "PR merged, worktree cleaned up"
$ a agent peer notify --parent -m "The delegated work is complete"
```

#### tmux-resurrect integration

Pane user options are not preserved by tmux-resurrect, so `a agent resurrect save` persists them to `~/.cache/armyknife/cc/resurrect/pane_sessions.txt`, and `a agent resurrect restore` re-applies them and types `a agent resume <session-id>` into each pane, so the coding agent comes back automatically after a tmux server crash or restart. Restore skips typing the resume command into any pane whose process tree already has a live process for the selected engine, so re-running it against a session that is already active does not retype the command into its input box.

`a agent resurrect save` also records each session's engine, `ancestor_session_ids` (used by `a agent peer parent`/`children`, see above), and vibe inheritance marker alongside its session ID, since a tmux server restart can wipe the session database before restore runs. `a agent resurrect restore` passes the snapshot to `a agent resume --engine ... --ancestor-session-ids ...`, restoring the marker when present so `a agent vibe status` keeps its result after the database is rebuilt.

Wire the commands into tmux-resurrect via its post-save / post-restore hooks:

```tmux
set -g @resurrect-hook-post-save-all '$HOME/.cargo/bin/a agent resurrect save'
set -g @resurrect-hook-post-restore-all '$HOME/.cargo/bin/a agent resurrect restore'
```

Use `$HOME` rather than `~`: tmux escapes a leading `~` in option values, which prevents tilde expansion when tmux-resurrect `eval`s the hook.

#### Auto-pause

Sessions that stay in the `stopped` state for longer than the configured timeout are automatically terminated to free up system resources. The session record is preserved in the database and the status is flipped to `paused` once the process is confirmed gone, so `a agent resume` can restore the conversation.

Sweep also checks the latest app-server turn for each running Codex session. It marks failed turns as `failed` and interrupted turns as `stopped`, or `waiting` when a permission request is pending. If the app-server cannot be reached, that session is skipped for the current pass.

`a agent sweep` scans every stored session record once. On the first shutdown request for a Codex session with a recorded tmux pane, it sends Ctrl+D so Codex can restore the terminal, then falls back to SIGTERM if the process remains alive. Codex sessions without a pane and Claude Code sessions receive SIGTERM directly. Later sweeps re-send SIGTERM while the process remains alive; the status stays `stopped` until exit is confirmed. Run sweep periodically via a launchd agent so idle sessions eventually get paused even while no hook is firing.

Each pass also checks `stopped` and `paused` sessions with linked worktrees. When the worktree branch has a merged pull request and no background task is pending, sweep closes the session through the regular `a agent close` path, which removes the worktree and notifies its delegator. Open pull requests, closed unmerged pull requests, and sessions without linked worktrees remain untouched. A successful automatic close sends a desktop notification with the PR number and title. If `magick` (ImageMagick) is installed, the notification uses a git-merge icon downloaded from Octicons and cached under `~/.cache/armyknife`; download or conversion failures leave the notification without an icon.

The `agent.auto_pause.enabled` setting controls only the pause pass; merged pull request cleanup still runs when auto-pause is disabled.

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
| `--dry-run`        | Print what would be paused or closed without making changes    |

The `install`, `uninstall`, and `status` subcommands require macOS. The launchd agent is installed at `~/Library/LaunchAgents/fohte.armyknife.cc-sweep.plist`.

Configure via `~/.config/armyknife/config.yaml`:

```yaml
agent:
  auto_pause:
    enabled: true # default: true
    timeout: 30m # default: "30m" (accepts "30s", "10m", "1h30m", etc.)
```

Set `enabled: false` to disable auto-pausing. Merged pull request cleanup continues, and the launchd agent stays installed, so toggling via config does not require `uninstall`.

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

`a agent hook` keeps each tmux window's aggregated Claude Code status in the window-scoped user option `@armyknife-cc-window-status`. On every session state change it recomputes the status symbols (`●` running, `◐` waiting for input or stopped with pending work and a linked crit or Human-in-the-Loop review, `◎` main loop idle with only a background task/subagent still in flight and no review waiting for user input, `✱` stopped & unread, `○` stopped & read, `⏸` paused, `✗` failed) of every Claude Code session in the window's panes, concatenates them without a separator, writes the result to `@armyknife-cc-window-status`, and refreshes the status bar — but only when the rendered value actually changed, so no-op transitions cause no redraw. `a agent bg run` workers also use `◎` while the main loop is stopped and no review is waiting for user input.

The same sync also mirrors a session title into the window-scoped `@armyknife-cc-window-title` option: the `label` of the first session in the window (in pane order) that has one set, or an empty string if none do — titles are not concatenated across sessions in the same window. Press `e` in `a agent watch` to rename the selected session's title, persisting it as `label`; the tmux option is refreshed best-effort on confirm (skipped silently if the pane has no resolvable window), and otherwise catches up on the next status-changing hook event for that window. While renaming, press `Ctrl+g` to generate a title from the session's transcript (its first user message and latest assistant message) — this returns you to the session list immediately, no waiting: generation runs in a fully detached background process that keeps going even if `agent watch` is closed entirely, and applies the generated title directly once it lands, but only if you haven't renamed the session again in the meantime. Generation shells out to the same backend as `a agent new` (the `claude` CLI, falling back to `opencode`), so it requires one of those to be installed and authenticated.

Reference both options from tmux's `window-status-format` to surface per-window session state and title next to the window index. `#{?...}` falls back to `#W` (the tmux window name) when no session in the window has a title set:

```tmux
set -g window-status-format '#{@armyknife-cc-window-status}#I:#{?#{@armyknife-cc-window-title},#{@armyknife-cc-window-title},#W}'
```

`a agent window-status <window_id>` prints the same status symbols on demand, for manual inspection or a polling-based `window-status-format`. The output contains no tmux style markup so the symbols inherit the surrounding `window-status-*` style (avoids `reverse` painting the icon cell as a colored block).

#### Pane border metadata

`a agent hook` writes the session status to the pane-scoped `@armyknife-cc-pane-status` option, the agent status kind to `@armyknife-agent-status-kind`, and the configured work type icon and foreground color to `@armyknife-cc-pane-work-type`. The status and status-kind options are unset when the session ends; the status-kind option is also unset when no kind is set. The work type option is unset when no configured work type applies. `a agent watch` also writes a linked tq task to `@armyknife-tq-task` as `##1234 Example task` (the doubled `#` is required by tmux format parsing), and sets `@armyknife-tq-closed` to `1` only for completed tasks. These tq options refresh when `a agent watch` fetches task data.

Reference the options from tmux's `pane-border-format` to display them in the pane border:

```tmux
set -g pane-border-format '#{@armyknife-cc-pane-status}#{@armyknife-agent-status-kind}#{@armyknife-cc-pane-work-type} #{@armyknife-tq-task}'
```

#### Pane has-paused flag

`a agent hook` also materializes a per-pane paused flag as a marker file at `${TMPDIR:-/tmp}/armyknife-cc-paused-${USER:-unknown}-<pane_id>` (e.g. `/tmp/armyknife-cc-paused-fohte-%17`). The file exists exactly while the pane's Claude Code session is `Paused` (e.g. SIGTERMed by `auto_pause`); every other state (`Running` / `WaitingInput` / `Stopped` / `Failed` / `Ended`) removes it. The `${USER:-unknown}` segment prevents collisions on multi-user hosts where `TMPDIR` falls back to a shared `/tmp` and tmux pane IDs clash across users. Downstream prompt renderers (e.g. starship) can surface a resumable-session label with a `test -e "${TMPDIR:-/tmp}/armyknife-cc-paused-${USER:-unknown}-${TMUX_PANE}"` check, which avoids the tmux client round trip a pane user option would require on every prompt. A file-existence flag is used rather than the session name so the prompt distinguishes an armyknife-paused session (file exists) from a user-driven Ctrl-C exit (no file). `a agent pane-has-paused <pane_id>` prints `1` / empty on demand from the session state and is intended for manual inspection; prompt renderers on the hot path should read the file instead.

#### Task linking (tq)

When the `tq` CLI is on `PATH`, a linked session shows its `#<number>` in a fixed-width column, and its TASKS row shows the task title instead of the session name. TASKS rows show this column; YOUR TURN rows show the session name and configured status name or automatic label before the note. The note is shortened or omitted when the row is too narrow. Rows without a linked task reserve the same column, keeping every TASKS-row title aligned. The number dims unless the task is related to the cursor row's task -- the same task, or its direct parent/child task -- so scanning for everything tied to the task you're currently looking at doesn't require reading every number. If the linked task is closed, its number renders struck through in a dusty purple, overriding the related/dimmed coloring. Older `tq` binaries that predate task status reporting are treated as open. Linking a session to a task happens on tq's side, keyed by Claude Code session ID; armyknife only reads the resulting association by shelling out to `tq session list`, so tq's own base URL and authentication (e.g. Cloudflare Access) stay entirely tq's concern.

When `tq` isn't on `PATH`, or the command fails, rows keep task associations from the last successful lookup, including data restored from cache at startup. During an initial load without a cache, the task-number column shows skeleton placeholders; task numbers or blank cells replace the placeholders without moving session titles. Local operations (focus, resume) are never blocked by `tq` being down.

Press `t` to open the selected session's linked task in the browser (no-op if the session has no linked task).

armyknife distinguishes a `Paused` session (auto-paused by `a agent sweep`, resumable) from an `Ended` one (the user exited; gone for good) -- tq's own hooks see both as the same `SessionEnd` event. Claude Code `SessionEnd`, Codex exits through `a agent codex` or `a agent resume`, explicit Codex close, and thread takeovers can transition sessions to `Ended`. Codex `SessionEnd` keeps an existing `Paused` session paused, records a sweep-signaled shutdown as `Paused`, and otherwise records `Stopped` because a daemon restart also triggers it. Ended sessions are archived by a detached `a agent archive-tq-session-detached --session <id>` worker. Archiving is best-effort and never blocks session shutdown; failures are logged and ignored. This requires a tq CLI that supports `tq session archive`.

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

When `a agent close`, `a agent clean`, or the TUI clean view's background cleanup removes a worktree whose branch's PR was merged, and that worktree hosted a delegated Claude Code session (`a agent new --worktree` from another session), it sends the configured `agent.merge_notification` template to the delegator session via `a agent peer notify` so a delegator blocked on "wait for this PR to merge" can continue. No notification is sent when the key is unset; see [Configuration](#configuration). Best-effort: notification failures (delegator already ended, no messaging socket, etc.) don't affect the cleanup.

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

armyknife supports git-style hooks for command lifecycle events (session startup, worktree creation, PR draft review/submit/merge). See [docs/hooks.md](docs/hooks.md) for the available hook points, environment variables, and usage examples.

### Logging

armyknife writes JSONL diagnostic logs to `~/.cache/armyknife/logs/armyknife.log.YYYY-MM-DD` (daily rotation, 7 files retained). Set `ARMYKNIFE_LOG=off|error|info|debug` to control verbosity. See [docs/logging.md](docs/logging.md) for the event reference and debugging recipes.

### `a doctor`

Check availability and versions of external tools armyknife depends on (`git`, `gh`, `tmux`, `nvim`, `wezterm`, `ghostty`, `delta`, `claude`, `opencode`, `lsof`, and Hammerspoon on macOS). `codex` and `srt` are also checked when Codex Stop inference is enabled. Missing tools include an install hint. Exits with status 0 regardless of findings — it is informational, not a gate.

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

`agent.profile` returns the map for `agent.active_profile`. `agent.profile.<path>` reads a dot-separated path from that selected profile, using the same scalar and YAML output formats. If no profile is selected, the selected profile is undefined, or the path is missing, the key lookup fails without a fallback. The profile name is used as a key in `agent.profiles`; it is not part of the alias path.

```sh
$ a config get agent.profile.review.model
sample-model-alpha
```

`a config get repo.vibe` prints the effective allowlist value for the current repository.

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

$ cd ~/ghq/github.com/example-org/standalone-tool
$ a config get repo.standalone_fork
true

$ cd ~/ghq/github.com/example-org/unconfigured-fork
$ a config get repo.standalone_fork
false
```

## Release

Releases are automated with release-please. The [release workflow](.github/workflows/release-please.yml) creates or updates a release PR on pushes to `master` and automatically merges it. Merging the release PR creates a GitHub release; build jobs upload the pre-built binaries afterward.

After the release PR is merged and the GitHub release has been created, run `a update`. You do not need to wait for the binaries to finish uploading: `a update` retries while your platform's asset is being uploaded. See [`a update`](#a-update) for retry timing.

## License

[MIT](LICENSE)
