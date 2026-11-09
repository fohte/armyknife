import type { Session } from '#types'

export type SessionShortcutAction =
  { type: 'move'; key: 'j' | 'k' } | { type: 'view'; view: 'board' } | null

export function getSessionShortcutAction(
  key: string,
  shiftKey: boolean,
): SessionShortcutAction {
  if (shiftKey && key.toLowerCase() === 'b') {
    return { type: 'view', view: 'board' }
  }

  if (!shiftKey && (key === 'j' || key === 'k')) {
    return { type: 'move', key }
  }

  return null
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
