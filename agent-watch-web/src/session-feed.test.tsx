import { afterEach, expect, it, vi } from 'vitest'

import { startSessionFeed } from '#session-feed'

afterEach(() => vi.restoreAllMocks())

class FakeEventSource {
  private listeners = new Map<string, EventListenerOrEventListenerObject>()
  closed = false

  addEventListener(type: string, callback: EventListenerOrEventListenerObject) {
    this.listeners.set(type, callback)
  }

  removeEventListener(type: string, listener: EventListener) {
    if (this.listeners.get(type) === listener) {
      this.listeners.delete(type)
    }
  }

  close() {
    this.closed = true
  }

  emit(type: string) {
    const listener = this.listeners.get(type)
    if (typeof listener === 'function') {
      listener(new Event(type))
    } else {
      listener?.handleEvent(new Event(type))
    }
  }
}

function deferred<T>() {
  let resolve: (value: T) => void = () => {}
  const promise = new Promise<T>((promiseResolve) => {
    resolve = promiseResolve
  })

  return { promise, resolve }
}

function jsonResponse(body: unknown) {
  const responseBody =
    typeof body === 'object' && body !== null && !('mode' in body)
      ? { mode: 'full', token: 'fixture-token', ...body }
      : body

  return new Response(JSON.stringify(responseBody), {
    status: 200,
    headers: { 'Content-Type': 'application/json' },
  })
}

function session(id: string, title: string, elapsedSeconds: number) {
  return {
    id,
    isPaused: false,
    category: 'in_progress',
    kind: null,
    title,
    preview: null,
    elapsedSeconds,
    unread: false,
    task: null,
    repo: 'example/service',
    pullRequest: null,
  }
}

it('requests deltas and restores the ordered full session response', async () => {
  const source = new FakeEventSource()
  const firstReceived = deferred<undefined>()
  const secondReceived = deferred<undefined>()
  const thirdReceived = deferred<undefined>()
  const responses: unknown[] = []
  const sessionA = session('session-a', 'First session', 10)
  const sessionB = session('session-b', 'Second session', 11)
  const updatedSessionB = session('session-b', 'Updated session', 12)
  const sessionC = session('session-c', 'New session', 13)
  const updatedSessionC = session('session-c', 'Updated new session', 14)
  const fetcher = vi
    .fn()
    .mockResolvedValueOnce(
      jsonResponse({
        mode: 'full',
        token: 'snapshot-1',
        revision: 1,
        workTypeNavLabels: { 'type-a': 'Type A' },
        sessions: [sessionA, sessionB],
      }),
    )
    .mockResolvedValueOnce(
      jsonResponse({
        mode: 'delta',
        token: 'snapshot-2',
        revision: 2,
        updatedSessions: [updatedSessionB, sessionC],
        removedSessionIds: ['session-a'],
        sessionOrder: ['session-c', 'session-b'],
      }),
    )
    .mockResolvedValueOnce(
      jsonResponse({
        mode: 'delta',
        token: 'snapshot-3',
        revision: 3,
        workTypeNavLabels: { 'type-a': 'Updated Type A' },
        updatedSessions: [updatedSessionC],
        removedSessionIds: [],
        sessionOrder: ['session-c', 'session-b'],
      }),
    )
  const now = vi.spyOn(Date, 'now').mockReturnValue(1_000)

  const stop = startSessionFeed(
    (response) => {
      responses.push(response)
      if (responses.length === 1) {
        firstReceived.resolve(undefined)
      } else if (responses.length === 2) {
        secondReceived.resolve(undefined)
      } else {
        thirdReceived.resolve(undefined)
      }
    },
    () => {},
    { fetcher, createEventSource: () => source },
  )

  await firstReceived.promise
  source.emit('sessions')
  await secondReceived.promise
  source.emit('sessions')
  await thirdReceived.promise
  stop()
  now.mockRestore()

  const snapshot = () => ({
    requests: fetcher.mock.calls.map(([url]) => String(url)),
    responses,
  })

  expect(snapshot()).toEqual({
    requests: [
      '/api/sessions',
      '/api/sessions?since=snapshot-1',
      '/api/sessions?since=snapshot-2',
    ],
    responses: [
      {
        revision: 1,
        workTypeNavLabels: { 'type-a': 'Type A' },
        sessions: [sessionA, sessionB],
      },
      {
        revision: 2,
        workTypeNavLabels: { 'type-a': 'Type A' },
        sessions: [sessionC, updatedSessionB],
      },
      {
        revision: 3,
        workTypeNavLabels: { 'type-a': 'Updated Type A' },
        sessions: [updatedSessionC, updatedSessionB],
      },
    ],
  })
})

it('delivers tree availability and work type metadata from a session response', async () => {
  const source = new FakeEventSource()
  const received = deferred<undefined>()
  const responses: unknown[] = []
  const errors: (string | null)[] = []
  const fetcher = vi.fn().mockResolvedValue(
    jsonResponse({
      revision: 1,
      workTypeNavLabels: { 'release-guide': 'Guides' },
      sessions: [
        {
          id: 'sample-session',
          isPaused: false,
          hasSessionTree: true,
          workType: 'release-guide',
          category: 'in_progress',
          activityStatus: 'waiting',
          waitingReason: 'waiting for a synthetic check',
          waitingForBackgroundTasks: true,
          kind: null,
          title: 'Prepare a launch checklist',
          preview: null,
          elapsedSeconds: 8,
          unread: false,
          task: null,
          repo: 'example/service',
          project: 'sample project',
          parent: null,
          ancestorSessionIds: [],
          pullRequest: null,
        },
      ],
    }),
  )
  const stop = startSessionFeed(
    (response) => {
      responses.push(response)
      received.resolve(undefined)
    },
    (message) => errors.push(message),
    { fetcher, createEventSource: () => source },
  )

  await received.promise
  stop()

  const snapshot = () => ({ responses, errors, closed: source.closed })

  expect(snapshot()).toEqual({
    responses: [
      {
        revision: 1,
        workTypeNavLabels: { 'release-guide': 'Guides' },
        sessions: [
          {
            id: 'sample-session',
            isPaused: false,
            hasSessionTree: true,
            workType: 'release-guide',
            category: 'in_progress',
            activityStatus: 'waiting',
            waitingReason: 'waiting for a synthetic check',
            waitingForBackgroundTasks: true,
            kind: null,
            title: 'Prepare a launch checklist',
            preview: null,
            elapsedSeconds: 8,
            unread: false,
            task: null,
            repo: 'example/service',
            project: 'sample project',
            parent: null,
            ancestorSessionIds: [],
            pullRequest: null,
          },
        ],
      },
    ],
    errors: [null],
    closed: true,
  })
})

it('rejects a session with a non-boolean paused state', async () => {
  const source = new FakeEventSource()
  const failureReported = deferred<undefined>()
  const errors: (string | null)[] = []
  let delivered = false
  const fetcher = vi.fn().mockResolvedValue(
    jsonResponse({
      revision: 1,
      sessions: [
        {
          id: 'invalid-paused-session',
          category: 'done',
          isPaused: 'true',
          kind: null,
          title: 'Prepare the next deployment window',
          preview: null,
          elapsedSeconds: 20,
          unread: false,
          task: null,
          repo: 'example/service',
          pullRequest: null,
        },
      ],
    }),
  )

  const stop = startSessionFeed(
    () => {
      delivered = true
    },
    (message) => {
      errors.push(message)
      if (message === 'Unable to load sessions. Retrying…') {
        failureReported.resolve(undefined)
      }
    },
    {
      fetcher,
      createEventSource: () => source,
      requestRetryDelayMs: 5_000,
    },
  )

  await failureReported.promise
  stop()

  const snapshot = () => ({ delivered, errors, closed: source.closed })
  expect(snapshot()).toEqual({
    delivered: false,
    errors: ['Unable to load sessions. Retrying…'],
    closed: true,
  })
})

it('coalesces session events and refreshes when the stream reconnects', async () => {
  const source = new FakeEventSource()
  const firstRequestStarted = deferred<undefined>()
  const firstResponse = deferred<Response>()
  const updateReceived = deferred<undefined>()
  const reconnectReceived = deferred<undefined>()
  const responses: unknown[] = []
  const errors: (string | null)[] = []
  const reloads: number[] = []
  const fetcher = vi
    .fn()
    .mockImplementationOnce(() => {
      firstRequestStarted.resolve(undefined)
      return firstResponse.promise
    })
    .mockResolvedValueOnce(
      jsonResponse({
        revision: 6,
        sessions: [
          {
            id: 'session-update',
            isPaused: false,
            category: 'in_progress',
            kind: 'FutureKind',
            title: 'Updated session',
            preview: null,
            elapsedSeconds: 12,
            unread: false,
            task: null,
            project: 'sample project',
            ancestorSessionIds: [],
            parent: null,
            repo: 'example/service',
            pullRequest: null,
          },
          {
            id: 'session-future-category',
            isPaused: false,
            category: 'later',
            kind: 'Question',
            title: 'Session in an unrecognized category',
            preview: null,
            elapsedSeconds: 12,
            unread: false,
            task: null,
            project: 'sample project',
            ancestorSessionIds: [],
            parent: null,
            repo: 'example/service',
            pullRequest: null,
          },
        ],
      }),
    )
    .mockResolvedValueOnce(
      jsonResponse({
        revision: 7,
        sessions: [
          {
            id: 'session-reconnected',
            isPaused: false,
            category: 'needs_action',
            kind: 'Question',
            title: 'Latest session after reconnect',
            preview: 'The stream reconnected.',
            elapsedSeconds: 14,
            unread: true,
            task: {
              id: 'task-reconnected',
              number: 2048,
              title: 'Restore event delivery',
              priority: 1,
            },
            project: 'sample project',
            ancestorSessionIds: [],
            parent: null,
            repo: 'example/service',
            pullRequest: { number: 81, state: 'Open' },
          },
        ],
      }),
    )
  const stop = startSessionFeed(
    (response) => {
      responses.push(response)
      if (response.revision === 6) {
        updateReceived.resolve(undefined)
      } else if (response.revision === 7) {
        reconnectReceived.resolve(undefined)
      }
    },
    (message) => errors.push(message),
    {
      fetcher,
      createEventSource: () => source,
      reloadPage: () => reloads.push(responses.length),
    },
  )

  await firstRequestStarted.promise
  source.emit('sessions')
  firstResponse.resolve(
    jsonResponse({
      revision: 5,
      sessions: [
        {
          id: 'session-initial',
          isPaused: false,
          category: 'needs_action',
          kind: 'Question',
          title: 'Initial session',
          preview: 'Please review this request.',
          elapsedSeconds: 10,
          unread: true,
          task: {
            id: 'task-initial',
            number: 2047,
            title: 'Review a request',
            priority: 1,
          },
          project: 'sample project',
          ancestorSessionIds: [],
          parent: null,
          repo: 'example/service',
          pullRequest: null,
        },
      ],
    }),
  )
  await updateReceived.promise

  source.emit('error')
  source.emit('open')
  await reconnectReceived.promise
  stop()

  const snapshot = () => [
    responses,
    errors,
    reloads,
    fetcher.mock.calls.length,
    source.closed,
  ]

  expect(snapshot()).toEqual([
    [
      {
        revision: 5,
        sessions: [
          {
            id: 'session-initial',
            isPaused: false,
            category: 'needs_action',
            kind: 'Question',
            title: 'Initial session',
            preview: 'Please review this request.',
            elapsedSeconds: 10,
            unread: true,
            task: {
              id: 'task-initial',
              number: 2047,
              title: 'Review a request',
              priority: 1,
            },
            project: 'sample project',
            ancestorSessionIds: [],
            parent: null,
            repo: 'example/service',
            pullRequest: null,
          },
        ],
      },
      {
        revision: 6,
        sessions: [
          {
            id: 'session-update',
            isPaused: false,
            category: 'in_progress',
            kind: 'FutureKind',
            title: 'Updated session',
            preview: null,
            elapsedSeconds: 12,
            unread: false,
            task: null,
            project: 'sample project',
            ancestorSessionIds: [],
            parent: null,
            repo: 'example/service',
            pullRequest: null,
          },
          {
            id: 'session-future-category',
            isPaused: false,
            category: 'later',
            kind: 'Question',
            title: 'Session in an unrecognized category',
            preview: null,
            elapsedSeconds: 12,
            unread: false,
            task: null,
            project: 'sample project',
            ancestorSessionIds: [],
            parent: null,
            repo: 'example/service',
            pullRequest: null,
          },
        ],
      },
      {
        revision: 7,
        sessions: [
          {
            id: 'session-reconnected',
            isPaused: false,
            category: 'needs_action',
            kind: 'Question',
            title: 'Latest session after reconnect',
            preview: 'The stream reconnected.',
            elapsedSeconds: 14,
            unread: true,
            task: {
              id: 'task-reconnected',
              number: 2048,
              title: 'Restore event delivery',
              priority: 1,
            },
            project: 'sample project',
            ancestorSessionIds: [],
            parent: null,
            repo: 'example/service',
            pullRequest: { number: 81, state: 'Open' },
          },
        ],
      },
    ],
    [null, null, 'Connection lost. Reconnecting…', null, null],
    [],
    3,
    true,
  ])
})

it('reloads after the server announces a restart and reconnects', async () => {
  const source = new FakeEventSource()
  const initialResponse = deferred<undefined>()
  const reloads: string[] = []
  const fetcher = vi
    .fn()
    .mockResolvedValue(jsonResponse({ revision: 1, sessions: [] }))
  const stop = startSessionFeed(
    () => {
      initialResponse.resolve(undefined)
    },
    () => {},
    {
      fetcher,
      createEventSource: () => source,
      reloadPage: () => reloads.push('reloaded'),
    },
  )

  await initialResponse.promise
  source.emit('restart')
  source.emit('error')
  source.emit('open')
  stop()

  const snapshot = () => [reloads, fetcher.mock.calls.length, source.closed]
  expect(snapshot()).toEqual([['reloaded'], 1, true])
})

it('retries a failed session request and clears the error after success', async () => {
  const source = new FakeEventSource()
  const failureReported = deferred<undefined>()
  const retryReceived = deferred<undefined>()
  const revisions: number[] = []
  const errors: (string | null)[] = []
  const fetcher = vi
    .fn()
    .mockResolvedValueOnce(new Response(null, { status: 503 }))
    .mockResolvedValueOnce(
      jsonResponse({
        revision: 8,
        sessions: [
          {
            id: 'session-retried',
            isPaused: false,
            category: 'done',
            kind: 'Stopped',
            title: 'Session after retry',
            preview: null,
            elapsedSeconds: 21,
            unread: false,
            task: null,
            project: 'sample project',
            ancestorSessionIds: [],
            parent: null,
            repo: 'example/service',
            pullRequest: null,
          },
        ],
      }),
    )

  const stop = startSessionFeed(
    (response) => {
      revisions.push(response.revision)
      retryReceived.resolve(undefined)
    },
    (message) => {
      errors.push(message)
      if (message === 'Unable to load sessions. Retrying…') {
        failureReported.resolve(undefined)
      }
    },
    {
      fetcher,
      createEventSource: () => source,
      requestRetryDelayMs: 5,
    },
  )

  await failureReported.promise
  await retryReceived.promise
  stop()

  const snapshot = () => [
    revisions,
    errors,
    fetcher.mock.calls.length,
    source.closed,
  ]

  expect(snapshot()).toEqual([
    [8],
    ['Unable to load sessions. Retrying…', null],
    2,
    true,
  ])
})
