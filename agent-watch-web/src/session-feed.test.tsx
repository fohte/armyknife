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

it('fetches sessions on startup and after a sessions event reconnects', async () => {
  let revision = 5
  let title = 'Update retry handling'
  const revisions: number[] = []
  const titles: string[] = []
  const errors: (string | null)[] = []
  const session = {
    id: 'sample-session',
    category: 'needs_action',
    kind: 'Question',
    preview: 'Should retries stop after three attempts?',
    elapsedSeconds: 8,
    unread: true,
    task: {
      id: 'sample-task',
      number: 2048,
      title: 'Improve request reliability',
      priority: 1,
    },
    repo: 'example/service',
    pullRequest: null,
  }
  const source = new FakeEventSource()
  let releaseInitial: (() => void) | undefined
  let releaseUpdate: (() => void) | undefined
  const initialLoaded = new Promise<void>((resolve) => {
    releaseInitial = resolve
  })
  const updateLoaded = new Promise<void>((resolve) => {
    releaseUpdate = resolve
  })
  const fetcher = vi.fn(() =>
    Promise.resolve(
      new Response(
        JSON.stringify({ revision, sessions: [{ ...session, title }] }),
        {
          status: 200,
          headers: { 'Content-Type': 'application/json' },
        },
      ),
    ),
  )
  const stop = startSessionFeed(
    (response) => {
      revisions.push(response.revision)
      titles.push(response.sessions[0]?.title ?? 'No session')
      if (response.revision === 5) {
        releaseInitial?.()
      } else if (response.revision === 6) {
        releaseUpdate?.()
      }
    },
    (message) => errors.push(message),
    { fetcher, createEventSource: () => source },
  )

  await initialLoaded
  revision = 6
  title = 'Retry handling updated'
  source.emit('sessions')
  await updateLoaded
  stop()

  const snapshot = () => [
    revisions,
    titles,
    errors,
    fetcher.mock.calls.length,
    source.closed,
  ]

  expect(snapshot()).toEqual([
    [5, 6],
    ['Update retry handling', 'Retry handling updated'],
    [null, null],
    2,
    true,
  ])
})
