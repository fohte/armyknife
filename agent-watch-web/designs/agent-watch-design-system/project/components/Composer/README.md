# Composer

The message box pinned under a phone's conversation: a one-line textarea that grows, and a square Send button. Hand-written, mobile layout, from `agent-watch-web/src/components/message-composer.tsx` and `message-composer.css`.

- The consumer supplies the draft, the placeholder, whether sending and interrupting are possible, attachments, and the send, interrupt and attachment handlers.
- Placeholders: `Message session`, `Message PR <number>` when the session has a PR, `Choose an option above, or message the session` while a proposal waits.
- Send is `size-tap` square, `aw-text-primary` fill with an `ArrowUp` in `aw-bg-surface`. While the session can be interrupted, a `CircleStop` button in `aw-text-muted` sits before it.
- The textarea is 16px so iOS Safari does not zoom; the bottom padding is `max(30px, env(safe-area-inset-bottom))`.
