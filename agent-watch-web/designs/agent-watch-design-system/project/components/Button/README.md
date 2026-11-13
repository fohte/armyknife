# Button

The shell's rectangular text button: outline for secondary actions, primary (an inverted `aw-text-primary` fill) for the one action that sends or confirms. Hand-written from `agent-watch-web/src/design-tokens.css` (`.aw-button`, `.aw-button--*` over @fohte/ui's `Button variant="plain"`).

- The consumer supplies the label, `type`, `onClick` and `disabled`.
- One primary per card (`Send answers`; `OK` in the decision panel); every other action is outline (`No answer (Esc)`, `Change…`).
- Labels render at 12px: the shell's `[data-slot='button']` rule wins over any class's font size.
- Disabled buttons drop to `opacity-disabled`.
- `aw-button--pressed` is declared for a toggled-on button; no component uses it.
