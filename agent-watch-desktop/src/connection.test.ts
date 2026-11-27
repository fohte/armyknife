import { describe, expect, it } from 'vitest'

import { applyConnectionProbe } from './connection.js'

describe('applyConnectionProbe', () => {
  it.each([
    {
      name: 'loads watch-web after a waiting page receives an HTTP response',
      isWaiting: true,
      serverResponded: true,
      expected: ['load-watch'],
    },
    {
      name: 'keeps the server error page visible for an HTTP 503 response',
      isWaiting: false,
      serverResponded: true,
      expected: [],
    },
    {
      name: 'shows the waiting page after a loaded page loses its server',
      isWaiting: false,
      serverResponded: false,
      expected: ['show-waiting'],
    },
    {
      name: 'keeps the waiting page while the server is unreachable',
      isWaiting: true,
      serverResponded: false,
      expected: [],
    },
  ])('$name', ({ isWaiting, serverResponded, expected }) => {
    const calls: string[] = []
    applyConnectionProbe(isWaiting, serverResponded, {
      loadWatchPage: () => calls.push('load-watch'),
      showWaitingPage: () => calls.push('show-waiting'),
    })

    expect(calls).toEqual(expected)
  })
})
