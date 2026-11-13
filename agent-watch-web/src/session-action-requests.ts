import { ResultAsync } from 'neverthrow'

import { jsonFromResponse } from '#api-requests'

function toError(error: unknown) {
  return error instanceof Error
    ? error
    : new Error('Request failed', { cause: error })
}

function postSessionAction(
  sessionId: string,
  path: string,
  payload: Record<string, unknown>,
) {
  return ResultAsync.fromPromise(
    fetch(`/api/sessions/${encodeURIComponent(sessionId)}/${path}`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(payload),
    }),
    toError,
  )
}

export function requestActionJson<T>(
  sessionId: string,
  path: string,
  payload: Record<string, unknown>,
  validate: (value: unknown) => value is T,
) {
  return postSessionAction(sessionId, path, payload).andThen((response) =>
    jsonFromResponse(response, validate),
  )
}
