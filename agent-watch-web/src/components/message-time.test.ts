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

function localTime(timestamp: string) {
  return new Intl.DateTimeFormat(undefined, {
    hour: '2-digit',
    minute: '2-digit',
    hourCycle: 'h23',
  }).format(new Date(timestamp))
}

it('shows only the time for a message sent today', () => {
  const timestamp = localTimestamp(2026, 11, 13, 18, 58)
  expect(formatMessageTime(timestamp, new Date(2026, 10, 13, 20, 0))).toBe(
    localTime(timestamp),
  )
})

it('shows the date when a message was sent on the previous local day', () => {
  const timestamp = localTimestamp(2026, 11, 12, 23, 58)
  expect(formatMessageTime(timestamp, new Date(2026, 10, 13, 0, 3))).toBe(
    `Nov 12 ${localTime(timestamp)}`,
  )
})

it('shows the year when a message was sent in a previous year', () => {
  const timestamp = localTimestamp(2025, 12, 31, 23, 58)
  expect(formatMessageTime(timestamp, new Date(2026, 0, 1, 0, 3))).toBe(
    `Dec 31, 2025 ${localTime(timestamp)}`,
  )
})
