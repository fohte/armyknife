import { ResultAsync } from 'neverthrow'

import { jsonFromResponse } from '#api-requests'

function toError(error: unknown) {
  return error instanceof Error
    ? error
    : new Error('Session launch request failed', { cause: error })
}

export function postNewSession<T>(
  body: unknown,
  validate: (value: unknown) => value is T,
) {
  return ResultAsync.fromPromise(
    fetch('/api/sessions', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify(body),
    }),
    toError,
  ).andThen((response) => jsonFromResponse(response, validate))
}
