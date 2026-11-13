import { expect, it } from 'vitest'

import { webUiRelayUrl } from '#components/crit-relay-url'

it('uses the tailnet relay origin and preserves the web UI path, query, and fragment', () => {
  expect(
    webUiRelayUrl('http://localhost:48761/review/sample?view=wide#notes', {
      protocol: 'https:',
      hostname: 'device.example.ts.net',
    }),
  ).toBe(
    'https://device.example.ts.net:8443/_web-ui/48761/review/sample?view=wide#notes',
  )
})

it('uses the localhost relay origin when watch-web is open locally', () => {
  expect(
    webUiRelayUrl('http://127.0.0.1:48761/review/sample', {
      protocol: 'http:',
      hostname: 'localhost',
    }),
  ).toBe('http://localhost:4318/_web-ui/48761/review/sample')
})

it('keeps URLs that do not target a local web UI outside the relay', () => {
  expect(
    webUiRelayUrl('http://review.example.invalid:48761/review/sample', {
      protocol: 'https:',
      hostname: 'device.example.ts.net',
    }),
  ).toBe('http://review.example.invalid:48761/review/sample')
})
