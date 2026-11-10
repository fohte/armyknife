import type { Session } from '#types'

export type SessionShortcutAction =
  | { type: 'move'; key: 'j' | 'k' }
  | { type: 'view'; view: 'board' }
  | { type: 'later' }
  | null

export type LaterPreset =
  'one_hour' | 'this_evening' | 'tomorrow' | 'next_week' | 'until_it_changes'

export function getSessionShortcutAction(
  key: string,
  shiftKey: boolean,
): SessionShortcutAction {
  if (shiftKey && key.toLowerCase() === 'b') {
    return { type: 'view', view: 'board' }
  }

  if (key === 'L' || (shiftKey && key.toLowerCase() === 'l')) {
    return { type: 'later' }
  }

  if (!shiftKey && (key === 'j' || key === 'k')) {
    return { type: 'move', key }
  }

  return null
}

export function getLaterPresetShortcut(key: string): LaterPreset | null {
  const presets: Record<string, LaterPreset> = {
    '1': 'one_hour',
    '2': 'this_evening',
    '3': 'tomorrow',
    '4': 'next_week',
    '5': 'until_it_changes',
  }
  return presets[key] ?? null
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
