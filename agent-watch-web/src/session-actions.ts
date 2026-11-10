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

export function setSessionLater(
  sessionId: string,
  preset: LaterPreset,
  expectedDeadline?: string | null,
) {
  return postSessionAction(sessionId, 'later', {
    preset,
    expectedDeadline,
  })
}

export function setSessionBackNow(sessionId: string) {
  return postSessionAction(sessionId, 'later', { preset: 'back_now' })
}

export function setSessionToday(sessionId: string, isToday: boolean) {
  return postSessionAction(sessionId, 'today', { isToday })
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null
}

function isLaterPresetTimes(value: unknown): value is LaterPresetTimes {
  if (!isRecord(value)) {
    return false
  }
  return LATER_PRESET_OPTIONS.every(({ preset }) => {
    if (!Object.hasOwn(value, preset)) {
      return false
    }
    const option = value[preset]
    return (
      isRecord(option) &&
      Object.hasOwn(option, 'when') &&
      typeof option['when'] === 'string' &&
      Object.hasOwn(option, 'deadline') &&
      (option['deadline'] === null || typeof option['deadline'] === 'string')
    )
  })
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
