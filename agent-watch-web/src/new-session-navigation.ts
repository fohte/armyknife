import type { Session } from '#types'

export function findLaunchedSession(
  sessions: readonly Session[],
  launchId: string | null,
) {
  return launchId === null
    ? undefined
    : sessions.find((session) => session.launchId === launchId)
}
