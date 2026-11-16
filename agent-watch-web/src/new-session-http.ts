import { err, ok, ResultAsync } from 'neverthrow'

import { errorFromResponse } from '#api-requests'

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
  ).andThen((response) => {
    if (!response.ok) {
      return errorFromResponse(response).andThen((error) => err(error))
    }
    return ResultAsync.fromPromise(response.json(), toError).andThen((value) =>
      validate(value)
        ? ok<T, Error>(value)
        : err<T, Error>(new Error('The server returned an invalid response')),
    )
  })
}
