import type { Session } from '#types'

export function sumTodoLaterCount(sessions: readonly Session[]): number {
  return sessions.reduce(
    (count, session) => count + (session.todoLaterCount ?? 0),
    0,
  )
}
