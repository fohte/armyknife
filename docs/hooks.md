# Hooks

armyknife supports git-style hooks for command lifecycle events. Place executable scripts in `~/.config/armyknife/hooks/` (or `$XDG_CONFIG_HOME/armyknife/hooks/`).

A hook fails the calling command if it exits with a non-zero status, or if the file exists without execute permission. A hook that does not exist is silently skipped.

## `post-worktree-create`

Runs after `a wm new` finishes creating a worktree.

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
