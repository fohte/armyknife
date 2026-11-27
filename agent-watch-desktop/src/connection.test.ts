import { describe, expect, it } from 'vitest'

import { applyConnectionProbe, resolveRetryUrl } from './connection.js'

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
      name: 'keeps a loaded page when its server probe fails',
      isWaiting: false,
      serverResponded: false,
      expected: [],
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
    })

    expect(calls).toEqual(expected)
  })
})

describe('resolveRetryUrl', () => {
  it.each([
    {
      name: 'preserves the failed same-origin session URL',
      failedUrl: 'http://127.0.0.1:4317/?session=session-example',
      expected: 'http://127.0.0.1:4317/?session=session-example',
    },
    {
      name: 'falls back for an external URL',
      failedUrl: 'https://example.test/',
      expected: 'http://127.0.0.1:4317/',
    },
    {
      name: 'falls back for an invalid URL',
      failedUrl: 'not a URL',
      expected: 'http://127.0.0.1:4317/',
    },
  ])('$name', ({ failedUrl, expected }) => {
    expect(
      resolveRetryUrl(
        failedUrl,
        'http://127.0.0.1:4317',
        'http://127.0.0.1:4317/',
      ),
    ).toEqual(expected)
  })
})
