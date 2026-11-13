import { err, ok, Result, ResultAsync } from 'neverthrow'

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null
}

function toError(error: unknown) {
  return error instanceof Error
    ? error
    : new Error('Request failed', { cause: error })
}

export function errorFromResponse(response: Response) {
  const fallback =
    response.statusText || `Request failed (${String(response.status)})`
  return ResultAsync.fromPromise(response.text(), toError).map((body) => {
    let message = fallback
    let code: string | null = null
    const parsed =
      body === ''
        ? null
        : Result.fromThrowable(() => JSON.parse(body) as unknown, toError)()
    if (parsed !== null && parsed.isOk() && isRecord(parsed.value)) {
      if (typeof parsed.value['error'] === 'string') {
        message = parsed.value['error']
      }
      if (typeof parsed.value['code'] === 'string') {
        code = parsed.value['code']
      }
    }
    return Object.assign(new Error(message), { code })
  })
}

export function jsonFromResponse<T>(
  response: Response,
  validate: (value: unknown) => value is T,
) {
  if (!response.ok) {
    return errorFromResponse(response).andThen((error) => err<T, Error>(error))
  }
  return ResultAsync.fromPromise(response.json(), toError).andThen((body) =>
    validate(body)
      ? ok<T, Error>(body)
      : err<T, Error>(new Error('The server returned an invalid response')),
  )
}

export function requestJson<T>(
  url: string,
  validate: (value: unknown) => value is T,
) {
  return ResultAsync.fromPromise(fetch(url), toError).andThen((response) =>
    jsonFromResponse(response, validate),
  )
}

export function requestText(url: string) {
  return ResultAsync.fromPromise(fetch(url), toError).andThen((response) => {
    if (!response.ok) {
      return errorFromResponse(response).andThen((error) => err(error))
    }
    return ResultAsync.fromPromise(response.text(), toError)
  })
}
