# Designs

Run the skills' `artifact.mjs` from `.claude/skills/<skill>/scripts/`. [README.md](README.md) has the install and open commands.

## Roles

- The design session edits the canvases and the design system, and opens design PRs. A human reviews the canvas before a design PR merges.
- The implementation session reads `screens/` as the spec and changes `agent-watch-web/src`. It does not edit anything under `designs/`. Where the design cannot be built as drawn, it reports back to the design session instead of choosing another look.

## Layout

Each canvas is a folder with `project/canvas.json`.

- `design-system/`: the design system. It belongs in `@fohte/ui` and lives here until it moves there.
- `screens/`: the canvas of the screens and parts the app should have. It differs from the app only between a design PR and the implementation PR that follows it.
- `explore/<theme>/`: one canvas per theme being decided, holding its options and `<theme>-current.dc.html`, a copy of the screen as it is now to compare against.
- `decisions/<YYYY-MM-DD>-<theme>/`: a decided theme. Its `README.md` records the options, the one chosen, and why; its `project/` is the theme's canvas, kept as it was when decided.

## Explore a theme

1. Create a canvas at `explore/<theme>/` with the `claude-design` skill, installing the design system from `design-system/`.
2. Run `artifact.mjs check agent-watch-web/designs/explore/<theme>` until it reports no problems.

## Decide a theme

In one design PR:

1. Apply the chosen option to `screens/`.
2. `git mv explore/<theme> decisions/<YYYY-MM-DD>-<theme>` and write its `README.md`.

Implement the decision in a separate PR.

## Keep the design system in step with the app

The design system is written by hand from `agent-watch-web/src`. When the app's CSS changes a token or a built component, update `design-system/`, then copy it into `screens/` and each canvas under `explore/`:

```sh
node .claude/skills/claude-design/scripts/artifact.mjs ds-sync agent-watch-web/designs/screens agentwatch
node .claude/skills/claude-design/scripts/artifact.mjs ds-sync agent-watch-web/designs/explore/topbar agentwatch
```

Canvases under `decisions/` keep the copy they were decided with.
