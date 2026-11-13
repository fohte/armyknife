import { err, ok, ResultAsync } from 'neverthrow'

import { errorFromResponse, jsonFromResponse } from '#api-requests'

function toError(error: unknown) {
  return error instanceof Error
    ? error
    : new Error('Request failed', { cause: error })
}

export function requestAction(
  sessionId: string,
  path: string,
  payload?: Record<string, unknown>,
) {
  return ResultAsync.fromPromise(
    fetch(`/api/sessions/${encodeURIComponent(sessionId)}/${path}`, {
      method: 'POST',
      ...(payload === undefined
        ? {}
        : {
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify(payload),
          }),
    }),
    toError,
  ).andThen((response) => {
    if (response.ok) return ok(undefined)
    return errorFromResponse(response).andThen((error) => err(error))
  })
}

export function requestActionJson<T>(
  sessionId: string,
  path: string,
  payload: Record<string, unknown>,
  validate: (value: unknown) => value is T,
) {
  return ResultAsync.fromPromise(
    fetch(`/api/sessions/${encodeURIComponent(sessionId)}/${path}`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(payload),
    }),
    toError,
  ).andThen((response) => jsonFromResponse(response, validate))
}
