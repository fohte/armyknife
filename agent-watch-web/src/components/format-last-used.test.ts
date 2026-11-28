import { expect, it } from 'vitest'

import { formatLastUsed } from '#components/format-last-used'

const now = new Date('2026-05-06T12:00:00Z')

it.each([
  {
    name: 'just now',
    lastUsedAt: '2026-05-06T11:59:30Z',
    expected: 'just now',
  },
  {
    name: 'future',
    lastUsedAt: '2026-05-06T12:01:00Z',
    expected: 'just now',
  },
  { name: 'minutes', lastUsedAt: '2026-05-06T11:42:00Z', expected: '18m ago' },
  {
    name: 'exactly 60 minutes',
    lastUsedAt: '2026-05-06T11:00:00Z',
    expected: '1h ago',
  },
  { name: 'hours', lastUsedAt: '2026-05-06T08:00:00Z', expected: '4h ago' },
  {
    name: 'exactly 24 hours',
    lastUsedAt: '2026-05-05T12:00:00Z',
    expected: '1d ago',
  },
  { name: 'days', lastUsedAt: '2026-05-02T12:00:00Z', expected: '4d ago' },
  { name: 'invalid', lastUsedAt: 'not-a-timestamp', expected: null },
])('formats a last-used time for $name', ({ lastUsedAt, expected }) => {
  expect(formatLastUsed(lastUsedAt, now)).toBe(expected)
})
