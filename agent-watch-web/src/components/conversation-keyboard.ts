export function numberShortcutChoice(
  event: KeyboardEvent,
  choiceCount: number,
): number | null {
  if (
    event.defaultPrevented ||
    event.altKey ||
    event.ctrlKey ||
    event.metaKey ||
    event.shiftKey
  ) {
    return null
  }

  const target = event.target
  if (
    target instanceof HTMLElement &&
    (target.isContentEditable ||
      ['INPUT', 'TEXTAREA', 'SELECT'].includes(target.tagName))
  ) {
    return null
  }

  if (!/^[1-9]$/.test(event.key)) return null
  const choice = Number(event.key) - 1
  return choice < choiceCount ? choice : null
}
