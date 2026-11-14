# StatusDot

A 7px square at the start of a session row that says, in color alone, what the session needs. Hand-written from `agent-watch-web/src/index.css` (`.session-status-dot`) and `src/components/later-today.css`.

- The consumer supplies the status: `unread` (a needs-action session with unread output) in `aw-color-warning`, `working` in `aw-color-working`, `done` as a `✓` in `aw-text-muted`, or nothing (a read needs-action session).
- The working dot goes round (`radius-round`) in the mobile inbox; every other dot is square.
- The row's title and section heading say the same thing in words; never let the dot be the only signal.
