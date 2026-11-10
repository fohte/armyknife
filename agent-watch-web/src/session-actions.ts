import { err, ok, ResultAsync } from 'neverthrow'

import type { LaterPreset } from '#session-navigation'

function requestError(error: unknown) {
  return error instanceof Error
    ? error
    : new Error('Session update failed', { cause: error })
}

function postSessionAction(sessionId: string, action: string, body: object) {
  return ResultAsync.fromPromise(
    fetch(`/api/sessions/${encodeURIComponent(sessionId)}/${action}`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(body),
    }),
    requestError,
  ).andThen((response) =>
    response.ok ? ok(undefined) : err(new Error('Session update failed')),
  )
}

export function setSessionLater(sessionId: string, preset: LaterPreset) {
  return postSessionAction(sessionId, 'later', { preset })
}

export function setSessionToday(sessionId: string, isToday: boolean) {
  return postSessionAction(sessionId, 'today', { isToday })
}
