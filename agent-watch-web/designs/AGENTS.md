# Designs

Run the skills' `artifact.mjs` from `.claude/skills/<skill>/scripts/`. [README.md](README.md) has the install and open commands.

## Layout

- `design-system/` is the one design system. Edit it with the `claude-design-system` skill.
- `ui/` is the one canvas. Add a page to it for new work; do not create another canvas.
    - `ui/project/screens/`: the app's screens as they are, or as decided.
    - `ui/project/components/`: parts the screens mount with `<dc-import>`.
    - `ui/project/explore/<theme>/`: options for a theme still being decided, on a page of their own.
- `decisions/<YYYY-MM-DD>-<theme>.md`: one record per decided theme.

File stems must be unique across the whole canvas, so start each option's file name with its theme: `explore/<theme>/<theme>-<option>.dc.html`.

## Explore a theme

1. Add a page for the theme to `ui/project/canvas.json`.
2. Put each option in `ui/project/explore/<theme>/`, with `<theme>-current.dc.html` showing the screen as it is now to compare against.
3. Run `artifact.mjs check agent-watch-web/designs/ui` until it reports no problems.

## Decide a theme

Do these in one PR:

1. Apply the chosen option to the files in `ui/project/screens/` or `ui/project/components/`.
2. Delete `ui/project/explore/<theme>/` and its page and boards in `canvas.json`.
3. Write `decisions/<YYYY-MM-DD>-<theme>.md` with:
    - the options and the one chosen,
    - why it was chosen,
    - a permalink to `ui/project/explore/<theme>/` at a commit of the PR that still has the options. Squash merging drops that commit from `master`, but the PR keeps it.

Implement the decision in the app in a separate PR.

## Keep the design system in step with the app

The design system is written by hand from `agent-watch-web/src`. When the app's CSS changes a token or a built component, update `design-system/`, then copy it into the canvas:

```sh
node .claude/skills/claude-design/scripts/artifact.mjs ds-sync agent-watch-web/designs/ui agentwatch
```
