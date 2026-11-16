import { expect, it } from 'vitest'

import {
  formatResetTime,
  formatTokenCount,
  getExpectedUsagePercentage,
  isAgentUsage,
} from '#agent-usage'

it.each([
  ['sub-thousand token counts stay exact', 842, '842'],
  ['token counts use one decimal place', 84_300, '84.3k'],
  ['whole thousands omit the decimal', 52_000, '52k'],
])('formats tokens when %s', (_description, tokens, expected) => {
  expect(formatTokenCount(tokens)).toEqual(expected)
})

it.each([
  ['minutes', 1_794_834_820, '47m'],
  ['hours and minutes', 1_794_839_800, '2h10m'],
  ['days and hours', 1_794_968_800, '1d14h'],
])('formats a reset in %s', (_description, resetsAt, expected) => {
  expect(formatResetTime(resetsAt, 1_794_832_000)).toEqual(expected)
})

it('calculates expected usage from the window reset time', () => {
  expect(
    getExpectedUsagePercentage(
      {
        usedPercentage: 41,
        resetsAt: 1_794_841_000,
        windowSeconds: 18_000,
      },
      1_794_832_000,
    ),
  ).toEqual(50)
})

it('does not calculate pace until the window has elapsed enough', () => {
  expect(
    getExpectedUsagePercentage(
      {
        usedPercentage: 1,
        resetsAt: 1_794_931_100,
        windowSeconds: 18_000,
      },
      1_794_832_000,
    ),
  ).toEqual(null)
})

it('accepts a complete usage response', () => {
  const complete = {
    sessions: [],
    quotas: { claude: null, codex: null },
  }

  expect(isAgentUsage(complete)).toEqual(true)
})

it('rejects a usage response without both quota entries', () => {
  const incomplete = {
    sessions: [],
    quotas: { claude: null },
  }

  expect(isAgentUsage(incomplete)).toEqual(false)
})
