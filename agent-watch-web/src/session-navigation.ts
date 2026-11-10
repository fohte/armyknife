import type { Session } from '#types'

export type SessionShortcutAction =
  | { type: 'move'; key: 'j' | 'k' }
  | { type: 'view'; view: 'board' }
  | { type: 'later' }
  | null

export const LATER_PRESET_OPTIONS = [
  { preset: 'one_hour', label: 'In 1 hour', when: '1h', key: '1' },
  { preset: 'this_evening', label: 'This evening', when: '19:00', key: '2' },
  { preset: 'tomorrow', label: 'Tomorrow', when: '9:00', key: '3' },
  { preset: 'next_week', label: 'Next week', when: 'Mon 11:00', key: '4' },
  {
    preset: 'until_it_changes',
    label: 'Until it changes',
    when: 'next activity',
    key: '5',
  },
] as const

export type LaterPreset = (typeof LATER_PRESET_OPTIONS)[number]['preset']

export function getSessionShortcutAction(
  key: string,
  shiftKey: boolean,
): SessionShortcutAction {
  if (shiftKey && key.toLowerCase() === 'b') {
    return { type: 'view', view: 'board' }
  }

  if (key === 'L' || (shiftKey && key === 'l')) {
    return { type: 'later' }
  }

  if (!shiftKey && (key === 'j' || key === 'k')) {
    return { type: 'move', key }
  }

  return null
}

export function getLaterPresetShortcut(key: string): LaterPreset | null {
  return (
    LATER_PRESET_OPTIONS.find((option) => option.key === key)?.preset ?? null
  )
}

export function moveSelection(
  sessions: readonly Session[],
  selectedId: string | null,
  key: 'j' | 'k',
): string | null {
  if (sessions.length === 0) {
    return null
  }

  const currentIndex = sessions.findIndex(
    (session) => session.id === selectedId,
  )
  if (currentIndex < 0) {
    return sessions[0]?.id ?? null
  }

  const offset = key === 'j' ? 1 : -1
  const nextIndex = Math.max(
    0,
    Math.min(sessions.length - 1, currentIndex + offset),
  )

  return sessions[nextIndex]?.id ?? null
}
