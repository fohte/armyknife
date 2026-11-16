import { expect, it } from 'vitest'

import {
  formatResetTime,
  formatTokenCount,
  getExpectedUsagePercentage,
  getUsagePaceStatus,
  getUsageSeverity,
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

it.each([
  ['ahead of pace', 61, 60, 'ahead'],
  ['on pace', 60, 60, 'on-pace'],
  ['before pace is available', 12, null, null],
])(
  'classifies quota pace when usage is %s',
  (_description, used, expected, result) => {
    expect(getUsagePaceStatus(used, expected)).toEqual(result)
  },
)

it.each([
  ['normal', 52, 'normal'],
  ['warning', 82, 'warning'],
  ['danger', 97, 'danger'],
])('classifies %s quota usage', (_description, used, expected) => {
  expect(getUsageSeverity(used)).toEqual(expected)
})

it('accepts a complete usage response', () => {
  const complete = {
    sessions: [
      {
        sessionId: 'sample-session',
        engine: 'codex',
        model: 'fixture-model-cobalt',
        effort: 'high',
        context: {
          usedTokens: 520,
          windowTokens: 1_000,
          usedPercentage: 52,
        },
        rateLimits: {
          fiveHour: {
            usedPercentage: 31,
            resetsAt: 1_794_841_200,
            windowSeconds: 18_000,
          },
          sevenDay: {
            usedPercentage: 16,
            resetsAt: 1_795_435_200,
            windowSeconds: 604_800,
          },
        },
        version: '0.99.0',
        observedAt: '2026-11-16T12:00:00Z',
      },
    ],
    quotas: {
      claude: null,
      codex: {
        engine: 'codex',
        rateLimits: {
          fiveHour: {
            usedPercentage: 31,
            resetsAt: 1_794_841_200,
            windowSeconds: 18_000,
          },
          sevenDay: {
            usedPercentage: 16,
            resetsAt: 1_795_435_200,
            windowSeconds: 604_800,
          },
        },
        observedAt: '2026-11-16T12:00:00Z',
      },
    },
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
