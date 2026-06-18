# Setup

Optional integrations that wire armyknife into tmux's event model.

## Unread stopped sessions (tmux `pane-focus-in`)

A stopped session that has not been focused since it most recently entered the stopped state renders as `✱` (unread) in `a cc watch`, `a cc list`, the tree under `a wm` views, and the per-window `@armyknife-cc-window-status` indicator. Focusing the pane marks the session read and reverts the indicator to `○`. Every new Stop event re-clears the read mark so a follow-up turn surfaces as unread again, even if you had already focused the same session earlier.

Wire `a cc mark-read` into tmux's `pane-focus-in` hook so any path of focusing the pane (TUI `f` key, tmux keybindings, mouse, etc.) clears the unread state:

```tmux
set-hook -g pane-focus-in 'run-shell -b "a cc mark-read -t #{pane_id}"'
```

`-b` runs the command in the background so the focus transition is not blocked by the disk write.
