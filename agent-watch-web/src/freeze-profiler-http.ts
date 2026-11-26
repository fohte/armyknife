import { err, ok, ResultAsync } from 'neverthrow'

import { errorFromResponse } from '#api-requests'
import type { LongTaskReport } from '#freeze-profiler'

function toError(error: unknown) {
  return error instanceof Error
    ? error
    : new Error('Long task report request failed', { cause: error })
}

export function postLongTaskReport(report: LongTaskReport) {
  return ResultAsync.fromPromise(
    fetch('/api/long-tasks', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify(report),
      keepalive: true,
    }),
    toError,
  )
    .andThen((response) => {
      if (!response.ok) {
        return errorFromResponse(response).andThen((error) =>
          err<undefined, Error>(error),
        )
      }
      return ok<undefined, Error>(undefined)
    })
    .match(
      () => undefined,
      (error) => {
        console.warn('Failed to send watch-web long-task profile', error)
      },
    )
}
