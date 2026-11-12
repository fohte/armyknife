import { expect, it } from 'vitest'

import { formatMessageTime } from '#components/message-time'

function localTimestamp(
  year: number,
  month: number,
  day: number,
  hour: number,
  minute: number,
) {
  return new Date(year, month - 1, day, hour, minute).toISOString()
}

it('shows only the time for a message sent today', () => {
  expect(
    formatMessageTime(
      localTimestamp(2026, 11, 13, 18, 58),
      new Date(2026, 10, 13, 20, 0),
    ),
  ).toBe('18:58')
})

it('shows the date when a message was sent on the previous local day', () => {
  expect(
    formatMessageTime(
      localTimestamp(2026, 11, 12, 23, 58),
      new Date(2026, 10, 13, 0, 3),
    ),
  ).toBe('Nov 12 23:58')
})

it('shows the year when a message was sent in a previous year', () => {
  expect(
    formatMessageTime(
      localTimestamp(2025, 12, 31, 23, 58),
      new Date(2026, 0, 1, 0, 3),
    ),
  ).toBe('Dec 31, 2025 23:58')
})
