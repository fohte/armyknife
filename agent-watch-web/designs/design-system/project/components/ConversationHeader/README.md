# ConversationHeader

The bar at the top of a session's conversation: Back, the title and its context, then the session's actions. Hand-written from the mobile branch of `agent-watch-web/src/components/conversation-pane-header.tsx`, with `mobile-layout.css`, `session-task-links.css`, `starred-session-toggle.css` and `copy-session-id-button.tsx`; the desktop branch is not rendered here.

- The consumer supplies the session (title, primary task and linked tasks, repo, category), the background-task count, whether the status panel is open, whether the session is starred, and the Back, Later, Close, status-panel and star handlers.
- Order, left to right: Back (`size-tap`), the heading (title on one line in `mobile-conversation-title`, a copy-id button, then the task and repo in `context-mobile`), the status-panel toggle (`size-tap`, `aw-color-working`, with the background-task count), Later, Close (done sessions only) and the star (each `size-tap-secondary`).
- The title ellipsizes after one line; the context line ellipsizes its last part.
- The top padding is `max(space-8, env(safe-area-inset-top))`; the bar never draws a fake status bar.
