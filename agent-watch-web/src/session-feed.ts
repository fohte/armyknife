import { err, errAsync, ok, Result, ResultAsync } from 'neverthrow'

import { isPullRequest, isSessionTask } from '#session-record-validation'
import type { Session, SessionsResponse } from '#types'

type SessionEventSource = {
  addEventListener: (type: string, listener: EventListener) => void
  removeEventListener: (type: string, listener: EventListener) => void
  close: () => void
}

type SessionFeedDependencies = {
  fetcher?: typeof fetch
  createEventSource?: (url: string) => SessionEventSource
  reloadPage?: () => void
  requestRetryDelayMs?: number
}

type SessionsFeedResponse =
  | {
      mode: 'full'
      token: string
      revision: number
      workTypeNavLabels?: Record<string, string>
      sessions: Session[]
    }
  | {
      mode: 'delta'
      token: string
      revision: number
      workTypeNavLabels?: Record<string, string>
      updatedSessions: Session[]
      removedSessionIds: string[]
      sessionOrder: string[]
    }

type SessionElapsedAnchor = {
  elapsedSeconds: number
  receivedAt: number
}

type CurrentSessions = {
  token: string
  response: SessionsResponse
  elapsedAnchors: ReadonlyMap<string, SessionElapsedAnchor>
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
  const validTask = task === null || isSessionTask(task)
  const validPullRequest = pullRequest === null || isPullRequest(pullRequest)

  return (
    typeof value['id'] === 'string' &&
    typeof value['isPaused'] === 'boolean' &&
    (value['activityStatus'] === undefined ||
      value['activityStatus'] === 'working' ||
      value['activityStatus'] === 'waiting' ||
      value['activityStatus'] === 'stopped') &&
    (value['waitingReason'] === undefined ||
      value['waitingReason'] === null ||
      typeof value['waitingReason'] === 'string') &&
    (value['waitingForBackgroundTasks'] === undefined ||
      typeof value['waitingForBackgroundTasks'] === 'boolean') &&
    (value['hasSessionTree'] === undefined ||
      typeof value['hasSessionTree'] === 'boolean') &&
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

function isWorkTypeNavLabels(value: unknown): value is Record<string, string> {
  return (
    isRecord(value) &&
    Object.values(value).every((label) => typeof label === 'string')
  )
}

function isSessionArray(value: unknown): value is Session[] {
  return Array.isArray(value) && value.every(isSession)
}

function isStringArray(value: unknown): value is string[] {
  return (
    Array.isArray(value) && value.every((entry) => typeof entry === 'string')
  )
}

function isSessionsFeedResponse(value: unknown): value is SessionsFeedResponse {
  const workTypeNavLabels = isRecord(value)
    ? value['workTypeNavLabels']
    : undefined

  if (
    !isRecord(value) ||
    typeof value['token'] !== 'string' ||
    value['token'].length === 0 ||
    typeof value['revision'] !== 'number' ||
    (workTypeNavLabels !== undefined && !isWorkTypeNavLabels(workTypeNavLabels))
  ) {
    return false
  }

  if (value['mode'] === 'full') {
    return isSessionArray(value['sessions'])
  }

  return (
    value['mode'] === 'delta' &&
    isSessionArray(value['updatedSessions']) &&
    isStringArray(value['removedSessionIds']) &&
    isStringArray(value['sessionOrder'])
  )
}

function toSessionFeedError(error: unknown) {
  return error instanceof Error
    ? error
    : new Error('Session request failed', { cause: error })
}

function fetchSessions(
  fetcher: typeof fetch,
  token: string | null,
): ResultAsync<SessionsFeedResponse, Error> {
  const url =
    token === null
      ? '/api/sessions'
      : `/api/sessions?since=${encodeURIComponent(token)}`

  return ResultAsync.fromPromise(fetcher(url), toSessionFeedError)
    .andThen((response): ResultAsync<unknown, Error> =>
      response.ok
        ? ResultAsync.fromPromise(response.json(), toSessionFeedError)
        : errAsync(new Error('Session request failed')),
    )
    .andThen((body) =>
      isSessionsFeedResponse(body)
        ? ok(body)
        : err(new Error('Session response is invalid')),
    )
}

function applySessionsFeedResponse(
  body: SessionsFeedResponse,
  current: CurrentSessions | null,
  receivedAt: number,
): Result<CurrentSessions, Error> {
  if (body.mode === 'full') {
    const response: SessionsResponse = {
      revision: body.revision,
      sessions: body.sessions,
      ...(body.workTypeNavLabels === undefined
        ? {}
        : { workTypeNavLabels: body.workTypeNavLabels }),
    }
    const elapsedAnchors = new Map(
      body.sessions.map((session): [string, SessionElapsedAnchor] => [
        session.id,
        { elapsedSeconds: session.elapsedSeconds, receivedAt },
      ]),
    )
    return ok({ token: body.token, response, elapsedAnchors })
  }

  if (current === null) {
    return err(new Error('Session delta arrived before a full response'))
  }

  const elapsedAnchors = new Map(current.elapsedAnchors)
  const sessionsById = new Map<string, Session>()
  for (const session of current.response.sessions) {
    sessionsById.set(session.id, session)
  }
  for (const id of body.removedSessionIds) {
    sessionsById.delete(id)
    elapsedAnchors.delete(id)
  }
  for (const session of body.updatedSessions) {
    sessionsById.set(session.id, session)
    elapsedAnchors.set(session.id, {
      elapsedSeconds: session.elapsedSeconds,
      receivedAt,
    })
  }

  const sessions: Session[] = []
  for (const id of body.sessionOrder) {
    const session = sessionsById.get(id)
    const elapsedAnchor = elapsedAnchors.get(id)
    if (session === undefined || elapsedAnchor === undefined) {
      return err(new Error('Session delta is invalid'))
    }
    const elapsedSecondsSinceAnchor = Math.floor(
      Math.max(0, receivedAt - elapsedAnchor.receivedAt) / 1_000,
    )
    sessions.push({
      ...session,
      elapsedSeconds: elapsedAnchor.elapsedSeconds + elapsedSecondsSinceAnchor,
    })
  }
  if (
    new Set(body.sessionOrder).size !== body.sessionOrder.length ||
    sessions.length !== sessionsById.size ||
    sessions.length !== elapsedAnchors.size
  ) {
    return err(new Error('Session delta is invalid'))
  }

  const workTypeNavLabels =
    body.workTypeNavLabels ?? current.response.workTypeNavLabels
  const response: SessionsResponse = {
    revision: body.revision,
    sessions,
    ...(workTypeNavLabels === undefined ? {} : { workTypeNavLabels }),
  }
  return ok({ token: body.token, response, elapsedAnchors })
}

export function startSessionFeed(
  onSessions: (
    response: SessionsResponse,
    changedSessionIds: readonly string[],
  ) => void,
  onError: (message: string | null) => void,
  {
    fetcher = fetch,
    createEventSource = (url) => new EventSource(url),
    reloadPage = () => {
      window.location.reload()
    },
    requestRetryDelayMs = 2_000,
  }: SessionFeedDependencies = {},
) {
  let active = true
  let requestPending = false
  let refreshQueued = false
  let requestError = false
  let streamError = false
  let restartPending = false
  let currentSessions: CurrentSessions | null = null
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

    void fetchSessions(fetcher, currentSessions?.token ?? null)
      .match(
        (body) => {
          if (active) {
            const previousSessionIds =
              currentSessions?.response.sessions.map((session) => session.id) ??
              []
            const applied = applySessionsFeedResponse(
              body,
              currentSessions,
              Date.now(),
            )
            if (applied.isErr()) {
              reportRequestError()
              return
            }

            currentSessions = applied.value
            const changedSessionIds =
              body.mode === 'full'
                ? [
                    ...new Set([
                      ...previousSessionIds,
                      ...body.sessions.map((session) => session.id),
                    ]),
                  ]
                : [
                    ...new Set([
                      ...body.updatedSessions.map((session) => session.id),
                      ...body.removedSessionIds,
                    ]),
                  ]
            onSessions(currentSessions.response, changedSessionIds)
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
  const onServerRestart: EventListener = () => {
    restartPending = true
  }
  const onConnectionOpen: EventListener = () => {
    const restartDetected = restartPending
    restartPending = false
    streamError = false
    if (reconnectPollTimer !== null) {
      clearInterval(reconnectPollTimer)
      reconnectPollTimer = null
    }
    reportStatus()
    if (restartDetected) {
      reloadPage()
      return
    }
    refresh()
  }

  eventSource.addEventListener('sessions', onSessionsEvent)
  eventSource.addEventListener('restart', onServerRestart)
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
    eventSource.removeEventListener('restart', onServerRestart)
    eventSource.removeEventListener('error', onConnectionError)
    eventSource.removeEventListener('open', onConnectionOpen)
    eventSource.close()
  }
}
