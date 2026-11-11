import { err, ok, Result, ResultAsync } from 'neverthrow'

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null
}

function toError(error: unknown) {
  return error instanceof Error
    ? error
    : new Error('Request failed', { cause: error })
}

function errorFromResponse(response: Response) {
  const fallback =
    response.statusText || `Request failed (${String(response.status)})`
  return ResultAsync.fromPromise(response.text(), toError).map((body) => {
    const parsed =
      body === ''
        ? null
        : Result.fromThrowable(() => JSON.parse(body) as unknown, toError)()
    const message =
      parsed !== null &&
      parsed.isOk() &&
      isRecord(parsed.value) &&
      typeof parsed.value['error'] === 'string'
        ? parsed.value['error']
        : fallback
    return new Error(message)
  })
}

export function requestJson<T>(
  url: string,
  validate: (value: unknown) => value is T,
) {
  return ResultAsync.fromPromise(fetch(url), toError).andThen((response) => {
    if (!response.ok) {
      return errorFromResponse(response).andThen((error) => err(error))
    }
    return ResultAsync.fromPromise(response.json(), toError).andThen((body) =>
      validate(body)
        ? ok(body)
        : err(new Error('The server returned an invalid response')),
    )
  })
}

export function requestText(url: string) {
  return ResultAsync.fromPromise(fetch(url), toError).andThen((response) => {
    if (!response.ok) {
      return errorFromResponse(response).andThen((error) => err(error))
    }
    return ResultAsync.fromPromise(response.text(), toError)
  })
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
