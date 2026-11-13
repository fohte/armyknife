agent watch is a dark, dense, square-cornered console for watching coding-agent sessions on desktop and phone. Color marks state, never decoration; hairlines separate regions; type stays small and plain.

## Content

- Write fixed UI text in English, short and plain: `Later`, `Close`, `Send answers`, `No answer (Esc)`, `Back to Inbox`, `Message session`.
- Session titles, messages, task names and repo names come from the sessions and are often Japanese. Show them as they are; never translate, re-case or decorate them.
- Labels and placeholders are sentence case (`Or enter your own answer`, `Select one or more choices.`).
- No emoji. Small states use the glyphs the app already draws: `✓` done or selected, `☆` / `★` star, `▾` menu, `›` disclosure, `↑` parent, `·` separator, `…` truncation.
- Counts are bare numbers beside their icon (the status-panel toggle shows `2`, not `2 tasks`).

## Color

- Dark only. Put every view on `aw-bg-app`, the nav and side panels on `aw-bg-nav`, cards on `aw-bg-surface`, menus and popovers on `aw-bg-popover`.
- Text: titles in `aw-text-primary`, message body and control labels in `aw-text-body`, context lines and labels in `aw-text-muted`, timestamps and counts in `aw-text-secondary`. Keep `aw-text-secondary` off `aw-bg-pressed` and `aw-bg-selected`, where it drops under 4.5:1.
- Color means state: `aw-color-warning` is waiting on the user (question and permission cards, unread needs-action dots) and the star; `aw-color-working` is running; `aw-color-error` is failed; `aw-color-neutral` is a settled choice. `aw-color-brand` is only the `>` of the wordmark and link text in messages.
- Separate with hairlines, not fills: `aw-border` between regions, `aw-border-row` and `aw-border-row-mobile` between rows, `aw-border-control` around controls, `aw-border-emphasis` for pressed, selected and focused.
- Pairs the source ships under the contrast floor, kept as they are: `aw-text-disabled` on `aw-bg-app` (4.10:1), `aw-text-crit-line` on `aw-bg-app` (2.56:1, an icon), `aw-border-control` on `aw-bg-app` (1.90:1, a control border).
- The `background` … `tint-*` names are what @fohte/ui reads; they alias the `aw-*` tokens. Style app code with the `aw-*` names.

## Type

- Two families: `sans` (the system's Helvetica Neue, then Hiragino for Japanese) for everything people read; `mono` (JetBrains Mono) for the wordmark, ids, task numbers, key hints and code.
- Message body is `body` (14px / 1.55), `body-mobile` (15px / 1.55) on phones. Text inputs on phones are 16px so iOS Safari does not zoom in.
- Every button label in the shell renders at `control` (12px): the shell's `[data-slot='button']` rule wins over a button's own font size.
- Weights are 700 for titles and headings and 400 for the rest; 600 appears only on question headers.
- The small steps (`label` 12.5px, `meta` 11.5px, `fine` 11px, `micro` 10.5px, `tiny` 10px) are already many: reuse one, never add another.

## Space and layout

- Use the steps the rules use, `space-2` to `space-24`. There is no 4px grid: `space-5` and `space-7` are real steps.
- The conversation column is at most `size-reading` wide, centered, with `space-24` padding on desktop; on phones the message list has `space-16` side padding and `space-14` between messages.
- The mobile layout applies below `breakpoint-mobile`. Pad for the safe areas: the top bar's top padding is `max(space-8, env(safe-area-inset-top))`, the composer's bottom padding `max(30px, env(safe-area-inset-bottom))`.
- On phones, primary actions are `size-tap` (44px) square, answer choices `size-choice` (48px) tall, and the header's text buttons and star `size-tap-secondary` (36px). Two header controls are smaller: the copy-id button (`size-control`, 26px) and the `+N tasks` toggle (22px tall).

## Shape and depth

- Square corners everywhere (`aw-radius`). Only status dots go round (`radius-round`).
- No shadow on buttons, cards or panels (`aw-shadow`). Only layers floating over content cast one: `shadow-dialog`, `shadow-confirm`, `shadow-suggestions`, `shadow-selection-reply`, `shadow-drawer`.
- Status dots are 7px squares: `aw-color-warning` for unread needs-action, `aw-color-working` for in progress (round in the mobile inbox), a `✓` in `aw-text-muted` for done.

## States

- Pressed or expanded: `aw-bg-pressed` with `aw-border-emphasis`. Highlighted row: `aw-bg-selected`. Active tab or field: `aw-bg-active`.
- Disabled: `opacity-disabled`. Ended sessions: `opacity-ended`.
- A plain button's hover turns its label `aw-text-body`.

## Iconography

- Lucide stroke icons (lucide-react), drawn in `currentColor`: `ChevronLeft` for Back (30px, stroke 2.5 on phones), `PanelRightOpen` / `PanelRightClose` for the status panel (18px on phones, 15px on desktop), `Copy` / `Check` for the session id, `ArrowUp` for Send (14px), `CircleStop` for Interrupt (18px).
- There is no logo file. The wordmark is text: `>` in `aw-color-brand`, then `agent watch`, both in `brand` (mono 13px / 700).

## Not synced

Skipped: `--tint-red`, `--tint-yellow` and `--tint-blue`, which are `color-mix()` (12% of `aw-color-error`, `aw-color-warning`, `aw-color-working` over `aw-bg-app`). The source declares no spacing, size, z-index or opacity tokens: those families list the literal values its rules use. Fonts: the sans stack is system fonts, so it has no file; only JetBrains Mono's latin subset is copied (its latin-ext file is not). Components are static renditions hand-written from the app's CSS (read-only route), because the app's components render from live session state; Button, StatusDot, ConversationHeader, QuestionCard and Composer are built. Not built: the inbox and its session rows, the left nav, the board, the command palette, the crit pane, the session status panel and session tree, decision, proposal, todo and permission cards, peer and markdown messages, and @fohte/ui's Chip.
