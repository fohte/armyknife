# Hooks

armyknife supports git-style hooks for command lifecycle events. Place executable scripts in `~/.config/armyknife/hooks/` (or `$XDG_CONFIG_HOME/armyknife/hooks/`).

A hook fails the calling command if it exits with a non-zero status, or if the file exists without execute permission. A hook that does not exist is silently skipped.

> **Behavior change (post 0.1.160).** Earlier versions only printed a warning on hook failure and continued the calling command. All hooks (including `post-worktree-create`) now abort the calling command on a non-zero exit or a non-executable file. `pre-worktree-delete` is an exception: it runs best-effort and never blocks deletion (see below).

## `post-worktree-create`

Runs after `a wm new` finishes creating a worktree. If the hook exits non-zero, `a wm new` removes the worktree it just created and rewinds the associated branch before propagating the error:

- A branch newly created in this invocation is deleted.
- A branch that pre-existed and was only checked out is left alone.
- A branch that `--force` rewrote is restored to its previous tip.

If the worktree cannot be removed automatically, the branch is left intact and `a wm new` points the user at `a wm delete` for manual cleanup.

| Variable                  | Description                           |
| ------------------------- | ------------------------------------- |
| `ARMYKNIFE_WORKTREE_PATH` | Absolute path to the created worktree |
| `ARMYKNIFE_BRANCH_NAME`   | Branch name of the worktree           |
| `ARMYKNIFE_REPO_ROOT`     | Root path of the parent repository    |

Example: auto-trust worktrees for Claude Code.

```sh
#!/bin/sh
# ~/.config/armyknife/hooks/post-worktree-create
jq --arg path "$ARMYKNIFE_WORKTREE_PATH" \
  '.projects[$path].hasTrustDialogAccepted = true' \
  ~/.claude.json | sponge ~/.claude.json
```

## `pre-worktree-delete`

Runs before `a wm delete` (aliases `d`, `rm`) removes the worktree directory. Unlike other hooks, failures here are best-effort: a non-zero exit or a non-executable hook only logs a warning to stderr, and the worktree is deleted anyway. Deleting a worktree is something the user explicitly asked for, so a broken cleanup hook should not block it.

Use this to clean up processes tied to the worktree's lifecycle (e.g. a daemon started by a `post-worktree-create` hook) before the directory disappears out from under them.

| Variable                  | Description                                                         |
| ------------------------- | ------------------------------------------------------------------- |
| `ARMYKNIFE_WORKTREE_PATH` | Absolute path to the worktree being deleted                         |
| `ARMYKNIFE_BRANCH_NAME`   | Branch name of the worktree (empty string if it cannot be resolved) |
| `ARMYKNIFE_REPO_ROOT`     | Root path of the parent repository                                  |

Pass `--skip-hooks` to `a wm delete` to skip this hook entirely.

Example: stop a `crit` review daemon running against the worktree.

```sh
#!/bin/sh
# ~/.config/armyknife/hooks/pre-worktree-delete
pkill -f "crit.*--cwd $ARMYKNIFE_WORKTREE_PATH" || true
```

## `pre-pr-review` and `pre-pr-submit`

Both hooks share the same environment-variable contract and let user scripts lint the draft PR title and body. They differ only in when they fire:

- `pre-pr-review` runs right before `a ai pr-draft review` opens the editor. A non-zero exit aborts the review, surfacing violations early so they can be fixed in the same editor session before `submit`.
- `pre-pr-submit` runs right before `a ai pr-draft submit` creates or updates a PR on GitHub. A non-zero exit aborts submission and acts as the final gate.

| Variable                 | Description                                                                      |
| ------------------------ | -------------------------------------------------------------------------------- |
| `ARMYKNIFE_PR_TITLE`     | Draft PR title                                                                   |
| `ARMYKNIFE_PR_BODY_FILE` | Path to a temp file containing the draft PR body (removed when the hook returns) |
| `ARMYKNIFE_PR_OWNER`     | Target repository owner                                                          |
| `ARMYKNIFE_PR_REPO`      | Target repository name                                                           |
| `ARMYKNIFE_PR_HEAD`      | Head branch the PR is created from                                               |
| `ARMYKNIFE_PR_BASE`      | Base branch (`--base`); empty string at review time or when defaulted by GitHub  |
| `ARMYKNIFE_PR_NUMBER`    | Existing PR number when updating; empty string at review time or when creating   |
| `ARMYKNIFE_PR_IS_UPDATE` | `1` when updating an existing open PR, `0` at review time or when creating       |

Example: forbid links to issues or PRs in other organizations. Symlinking the same script to both hook names enforces the rule at review time and again at submit time.

```sh
#!/bin/sh
# ~/.config/armyknife/hooks/pre-pr-review (symlink the same file to pre-pr-submit)
# Match any github.com link to an issue/PR, then filter out the allow-listed owner.
if grep -oE 'https?://github\.com/[^/]+/[^/]+/(issues|pull)/[0-9]+' \
    "$ARMYKNIFE_PR_BODY_FILE" \
    | grep -vE '^https?://github\.com/fohte/'; then
  echo "cross-org issue/PR links are not allowed" >&2
  exit 1
fi
```
