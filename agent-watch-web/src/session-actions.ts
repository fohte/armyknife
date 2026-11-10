import { err, errAsync, ok, ResultAsync } from 'neverthrow'

import {
  LATER_PRESET_OPTIONS,
  type LaterPreset,
  type LaterPresetTimes,
} from '#session-navigation'

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

export function setSessionBackNow(sessionId: string) {
  return postSessionAction(sessionId, 'later', { preset: 'back_now' })
}

export function setSessionToday(sessionId: string, isToday: boolean) {
  return postSessionAction(sessionId, 'today', { isToday })
}

function isLaterPresetTimes(value: unknown): value is LaterPresetTimes {
  return (
    typeof value === 'object' &&
    value !== null &&
    LATER_PRESET_OPTIONS.every(
      ({ preset }) =>
        Object.hasOwn(value, preset) &&
        typeof Reflect.get(value, preset) === 'string',
    )
  )
}

export function getLaterPresetTimes() {
  return ResultAsync.fromPromise(
    fetch('/api/sessions/later-options'),
    requestError,
  )
    .andThen((response) =>
      response.ok
        ? ResultAsync.fromPromise(response.json(), requestError)
        : errAsync(new Error('Later preset request failed')),
    )
    .andThen((body) =>
      isLaterPresetTimes(body)
        ? ok(body)
        : err(new Error('Later preset response is invalid')),
    )
}
