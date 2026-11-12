import { expect, it } from 'vitest'

import { critRelayUrl } from '#components/crit-relay-url'

it('uses the tailnet relay origin and preserves the crit path, query, and fragment', () => {
  expect(
    critRelayUrl('http://localhost:48761/review/sample?view=wide#notes', {
      protocol: 'https:',
      hostname: 'device.example.ts.net',
    }),
  ).toBe(
    'https://device.example.ts.net:8443/_crit/48761/review/sample?view=wide#notes',
  )
})

it('uses the localhost relay origin when watch-web is open locally', () => {
  expect(
    critRelayUrl('http://127.0.0.1:48761/review/sample', {
      protocol: 'http:',
      hostname: 'localhost',
    }),
  ).toBe('http://localhost:4318/_crit/48761/review/sample')
})

it('keeps URLs that do not target a local crit daemon outside the relay', () => {
  expect(
    critRelayUrl('http://review.example.invalid:48761/review/sample', {
      protocol: 'https:',
      hostname: 'device.example.ts.net',
    }),
  ).toBe('http://review.example.invalid:48761/review/sample')
})
