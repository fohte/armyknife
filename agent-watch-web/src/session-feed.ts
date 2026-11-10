import { err, errAsync, ok, ResultAsync } from 'neverthrow'

import type { Session, SessionsResponse } from '#types'

type SessionEventSource = {
  addEventListener: (type: string, listener: EventListener) => void
  removeEventListener: (type: string, listener: EventListener) => void
  close: () => void
}

type SessionFeedDependencies = {
  fetcher?: typeof fetch
  createEventSource?: (url: string) => SessionEventSource
  requestRetryDelayMs?: number
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null
}

function isSession(value: unknown): value is Session {
  if (!isRecord(value)) {
    return false
  }

  const category = value['category']
  const kind = value['kind']
  const task = value['task']
  const pullRequest = value['pullRequest']
  const validTask =
    task === null ||
    (isRecord(task) &&
      typeof task['id'] === 'string' &&
      typeof task['number'] === 'number' &&
      typeof task['title'] === 'string' &&
      (task['priority'] === null || typeof task['priority'] === 'number'))
  const validPullRequest =
    pullRequest === null ||
    (isRecord(pullRequest) &&
      typeof pullRequest['number'] === 'number' &&
      typeof pullRequest['state'] === 'string')

  return (
    typeof value['id'] === 'string' &&
    typeof value['isPaused'] === 'boolean' &&
    (value['workType'] === undefined ||
      value['workType'] === null ||
      typeof value['workType'] === 'string') &&
    typeof category === 'string' &&
    (kind === null || typeof kind === 'string') &&
    typeof value['title'] === 'string' &&
    (value['preview'] === null || typeof value['preview'] === 'string') &&
    typeof value['elapsedSeconds'] === 'number' &&
    typeof value['unread'] === 'boolean' &&
    validTask &&
    typeof value['repo'] === 'string' &&
    validPullRequest
  )
}

function isSessionsResponse(value: unknown): value is SessionsResponse {
  const workTypeNavLabels = isRecord(value)
    ? value['workTypeNavLabels']
    : undefined

  return (
    isRecord(value) &&
    typeof value['revision'] === 'number' &&
    (workTypeNavLabels === undefined ||
      (isRecord(workTypeNavLabels) &&
        Object.values(workTypeNavLabels).every(
          (label) => typeof label === 'string',
        ))) &&
    Array.isArray(value['sessions']) &&
    value['sessions'].every(isSession)
  )
}

function toSessionFeedError(error: unknown) {
  return error instanceof Error
    ? error
    : new Error('Session request failed', { cause: error })
}

function fetchSessions(
  fetcher: typeof fetch,
): ResultAsync<SessionsResponse, Error> {
  return ResultAsync.fromPromise(fetcher('/api/sessions'), toSessionFeedError)
    .andThen((response): ResultAsync<unknown, Error> =>
      response.ok
        ? ResultAsync.fromPromise(response.json(), toSessionFeedError)
        : errAsync(new Error('Session request failed')),
    )
    .andThen((body) =>
      isSessionsResponse(body)
        ? ok(body)
        : err(new Error('Session response is invalid')),
    )
}

export function startSessionFeed(
  onSessions: (response: SessionsResponse) => void,
  onError: (message: string | null) => void,
  {
    fetcher = fetch,
    createEventSource = (url) => new EventSource(url),
    requestRetryDelayMs = 2_000,
  }: SessionFeedDependencies = {},
) {
  let active = true
  let requestPending = false
  let refreshQueued = false
  let requestError = false
  let streamError = false
  let requestRetryTimer: ReturnType<typeof setTimeout> | null = null
  let reconnectPollTimer: ReturnType<typeof setInterval> | null = null

  const reportStatus = () => {
    if (requestError) {
      onError('Unable to load sessions. Retrying…')
    } else if (streamError) {
      onError('Connection lost. Reconnecting…')
    } else {
      onError(null)
    }
  }

  const clearRequestRetry = () => {
    if (requestRetryTimer !== null) {
      clearTimeout(requestRetryTimer)
      requestRetryTimer = null
    }
  }

  const scheduleRequestRetry = () => {
    if (requestRetryTimer === null) {
      requestRetryTimer = setTimeout(() => {
        requestRetryTimer = null
        refresh()
      }, requestRetryDelayMs)
    }
  }

  const reportRequestError = () => {
    if (active) {
      requestError = true
      reportStatus()
      scheduleRequestRetry()
    }
  }

  const runRefresh = () => {
    refreshQueued = false

    void fetchSessions(fetcher)
      .match(
        (body) => {
          if (active) {
            onSessions(body)
            requestError = false
            clearRequestRetry()
            reportStatus()
          }
        },
        () => {
          reportRequestError()
        },
      )
      .then(() => {
        if (!active) {
          requestPending = false
        } else if (refreshQueued) {
          runRefresh()
        } else {
          requestPending = false
        }
      })
  }

  const refresh = () => {
    refreshQueued = true
    if (!requestPending) {
      requestPending = true
      runRefresh()
    }
  }

  const eventSource = createEventSource('/api/events')
  const onSessionsEvent: EventListener = () => {
    refresh()
  }
  const onConnectionError: EventListener = () => {
    streamError = true
    reportStatus()
    if (reconnectPollTimer === null) {
      reconnectPollTimer = setInterval(refresh, requestRetryDelayMs)
    }
  }
  const onConnectionOpen: EventListener = () => {
    streamError = false
    if (reconnectPollTimer !== null) {
      clearInterval(reconnectPollTimer)
      reconnectPollTimer = null
    }
    reportStatus()
    refresh()
  }

  eventSource.addEventListener('sessions', onSessionsEvent)
  eventSource.addEventListener('error', onConnectionError)
  eventSource.addEventListener('open', onConnectionOpen)
  refresh()

  return () => {
    active = false
    clearRequestRetry()
    if (reconnectPollTimer !== null) {
      clearInterval(reconnectPollTimer)
      reconnectPollTimer = null
    }
    eventSource.removeEventListener('sessions', onSessionsEvent)
    eventSource.removeEventListener('error', onConnectionError)
    eventSource.removeEventListener('open', onConnectionOpen)
    eventSource.close()
  }
}
