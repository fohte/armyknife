import type { Session, SessionsResponse } from '#types'

type SessionEventSource = {
  addEventListener: (type: string, listener: EventListener) => void
  removeEventListener: (type: string, listener: EventListener) => void
  close: () => void
}

type SessionFeedDependencies = {
  fetcher?: typeof fetch
  createEventSource?: (url: string) => SessionEventSource
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
      typeof pullRequest['state'] === 'string' &&
      ['Open', 'Closed', 'Merged'].includes(pullRequest['state']))
  const validCategory =
    category === 'needs_action' ||
    category === 'in_progress' ||
    category === 'done'
  const validKind =
    kind === null ||
    (typeof kind === 'string' &&
      ['Question', 'Approve', 'Review', 'Error', 'Plan', 'Stopped'].includes(
        kind,
      ))

  return (
    typeof value['id'] === 'string' &&
    validCategory &&
    validKind &&
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
  return (
    isRecord(value) &&
    typeof value['revision'] === 'number' &&
    Array.isArray(value['sessions']) &&
    value['sessions'].every(isSession)
  )
}

export function startSessionFeed(
  onSessions: (response: SessionsResponse) => void,
  onError: (message: string | null) => void,
  {
    fetcher = fetch,
    createEventSource = (url) => new EventSource(url),
  }: SessionFeedDependencies = {},
) {
  let active = true
  let latestRequest = 0

  const refresh = () => {
    const requestId = ++latestRequest

    const reportError = () => {
      if (active && requestId === latestRequest) {
        onError('Unable to load sessions. Reconnecting…')
      }
    }

    void Promise.resolve()
      .then(() => fetcher('/api/sessions'))
      .then((response) =>
        response.ok
          ? response.json()
          : Promise.reject(new Error('Session request failed')),
      )
      .then((body: unknown) => {
        if (!isSessionsResponse(body)) {
          reportError()
          return
        }

        if (active && requestId === latestRequest) {
          onSessions(body)
          onError(null)
        }
      })
      .catch(reportError)
  }

  const eventSource = createEventSource('/api/events')
  const onSessionsEvent: EventListener = () => {
    refresh()
  }

  eventSource.addEventListener('sessions', onSessionsEvent)
  refresh()

  return () => {
    active = false
    latestRequest += 1
    eventSource.removeEventListener('sessions', onSessionsEvent)
    eventSource.close()
  }
}
