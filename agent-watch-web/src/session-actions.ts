import { err, errAsync, ok, ResultAsync } from 'neverthrow'

import {
  LATER_PRESET_OPTIONS,
  type LaterPreset,
  type LaterPresetTimes,
} from '#session-navigation'
import type { Session } from '#types'

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

class CloseSessionError extends Error {
  readonly code: string | null

  constructor(message: string, code: string | null) {
    super(message)
    this.code = code
  }
}

function closeSession(sessionId: string, keepWorktree: boolean) {
  return ResultAsync.fromPromise(
    fetch(`/api/sessions/${encodeURIComponent(sessionId)}/close`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ keepWorktree }),
    }),
    requestError,
  ).andThen((response) => {
    if (response.ok) return ok(undefined)
    return ResultAsync.fromPromise(response.json(), requestError).andThen(
      (body: unknown) => {
        const error = isRecord(body) ? body['error'] : undefined
        const code = isRecord(body) ? body['code'] : undefined
        return err(
          new CloseSessionError(
            typeof error === 'string' ? error : 'Could not close the session.',
            typeof code === 'string' ? code : null,
          ),
        )
      },
    )
  })
}

export async function closeDoneSessions(
  sessions: readonly Pick<Session, 'category' | 'id' | 'title'>[],
  keepWorktree = false,
) {
  for (const session of sessions) {
    if (session.category !== 'done') continue

    const failure = await closeSession(session.id, keepWorktree).match(
      () => null,
      (error) => ({
        message: error.message,
        unmergedSessionId:
          error instanceof CloseSessionError &&
          error.code === 'unmerged_worktree'
            ? session.id
            : null,
      }),
    )
    if (failure !== null) {
      return {
        failure: failure.message || `Could not close ${session.title}.`,
        unmergedSessionId: failure.unmergedSessionId,
      }
    }
  }
  return { failure: null, unmergedSessionId: null }
}

export function openSessionInTerminal(sessionId: string) {
  return postSessionAction(sessionId, 'focus', {})
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
