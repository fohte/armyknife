# QuestionCard

A card in the conversation that asks the user to pick one of a session's choices or type their own answer. Hand-written, mobile layout, from `agent-watch-web/src/components/question-card.tsx`, `question-card.css`, `conversation-card.css` and `mobile-layout.css`.

- The consumer supplies the questions (header, text, choices with label and optional description, multi-select or not), whether the user can still answer, and the answer, skip and send handlers.
- The `aw-color-warning` border and `Question` heading mark it as waiting on the user; the permission card shares them.
- Choices are full-width rows, `size-choice` tall on phones, divided by `aw-border`. The picked one gets an `aw-color-neutral` border, `aw-bg-surface` and a `✓`.
- Desktop rows are 42px, 14px text, and show `1`–`9` key hints for a single-choice question; phones show none.
- Actions: `No answer (Esc)` (outline) and `Send answers` (primary), bottom right.
