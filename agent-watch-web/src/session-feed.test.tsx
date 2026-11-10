import { expect, it, vi } from 'vitest'

import { startSessionFeed } from '#session-feed'

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
  return new Response(JSON.stringify(body), {
    status: 200,
    headers: { 'Content-Type': 'application/json' },
  })
}

it('coalesces session events and refreshes when the stream reconnects', async () => {
  const source = new FakeEventSource()
  const firstRequestStarted = deferred<undefined>()
  const firstResponse = deferred<Response>()
  const updateReceived = deferred<undefined>()
  const reconnectReceived = deferred<undefined>()
  const responses: unknown[] = []
  const errors: (string | null)[] = []
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
    { fetcher, createEventSource: () => source },
  )

  await firstRequestStarted.promise
  source.emit('sessions')
  firstResponse.resolve(
    jsonResponse({
      revision: 5,
      sessions: [
        {
          id: 'session-initial',
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
    3,
    true,
  ])
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
