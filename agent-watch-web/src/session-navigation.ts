import type { Session } from '#types'

export type SessionView = 'inbox' | 'board'

export type SessionShortcutAction =
  | { type: 'move'; key: 'j' | 'k' }
  | { type: 'view'; view: SessionView }
  | { type: 'later' }
  | null

export const LATER_PRESET_OPTIONS = [
  { preset: 'one_hour', label: 'In 1 hour', key: '1' },
  { preset: 'this_evening', label: 'This evening', key: '2' },
  { preset: 'tomorrow', label: 'Tomorrow', key: '3' },
  { preset: 'next_week', label: 'Next week', key: '4' },
  {
    preset: 'until_it_changes',
    label: 'Until it changes',
    key: '5',
  },
] as const

export type LaterPreset = (typeof LATER_PRESET_OPTIONS)[number]['preset']
export type LaterPresetTimes = Record<
  LaterPreset,
  { when: string; deadline: string | null }
>

export function getSessionShortcutAction(
  key: string,
  shiftKey: boolean,
  currentView: SessionView,
): SessionShortcutAction {
  if (shiftKey && key.toLowerCase() === 'b') {
    return {
      type: 'view',
      view: currentView === 'inbox' ? 'board' : 'inbox',
    }
  }

  if (key.toLowerCase() === 'l') {
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
