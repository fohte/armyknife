import { describe, expect, it } from 'vitest'

import { resolveWatchTarget } from './config.js'

describe('resolveWatchTarget', () => {
  it.each([
    {
      name: 'uses the watch-web default URL',
      input: undefined,
      expected: {
        isOk: true,
        value: {
          url: 'http://127.0.0.1:4317/',
          origin: 'http://127.0.0.1:4317',
        },
      },
    },
    {
      name: 'accepts localhost on a development port',
      input: 'http://localhost:1234/',
      expected: {
        isOk: true,
        value: {
          url: 'http://localhost:1234/',
          origin: 'http://localhost:1234',
        },
      },
    },
    {
      name: 'rejects a non-loopback host',
      input: 'http://example.test/',
      expected: {
        isOk: false,
        error:
          'AGENT_WATCH_URL は localhost または 127.0.0.1 の http URL のみ指定できます',
      },
    },
    {
      name: 'rejects HTTPS',
      input: 'https://localhost/',
      expected: {
        isOk: false,
        error:
          'AGENT_WATCH_URL は localhost または 127.0.0.1 の http URL のみ指定できます',
      },
    },
    {
      name: 'rejects malformed URLs',
      input: '://invalid',
      expected: {
        isOk: false,
        error: 'AGENT_WATCH_URL は有効な URL である必要があります',
      },
    },
  ])('$name', ({ input, expected }) => {
    const result = resolveWatchTarget(input)

    expect(
      result.match(
        (value) => ({ isOk: true, value }),
        (error) => ({ isOk: false, error }),
      ),
    ).toEqual(expected)
  })
})
