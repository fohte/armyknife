import { err, ok } from 'neverthrow'
import { afterEach, expect, it, vi } from 'vitest'

import {
  formatResetTime,
  formatTokenCount,
  getExpectedUsagePercentage,
  getUsageLandingEstimate,
  getUsageSeverity,
  requestAgentUsage,
} from '#agent-usage'

afterEach(() => {
  vi.unstubAllGlobals()
})

function stubFetchJson(body: unknown) {
  const fetchMock = vi.fn<typeof fetch>().mockResolvedValue(
    new Response(JSON.stringify(body), {
      headers: { 'content-type': 'application/json' },
    }),
  )
  vi.stubGlobal('fetch', fetchMock)
}

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
  [
    'below pace',
    { usedPercentage: 40, resetsAt: 1_794_841_000, windowSeconds: 18_000 },
    { percentage: 80, status: 'on-pace' },
  ],
  [
    'at pace',
    { usedPercentage: 50, resetsAt: 1_794_841_000, windowSeconds: 18_000 },
    { percentage: 100, status: 'warning' },
  ],
  [
    'above pace',
    { usedPercentage: 70, resetsAt: 1_794_841_000, windowSeconds: 18_000 },
    { percentage: 140, status: 'warning' },
  ],
  [
    'well above pace',
    { usedPercentage: 75, resetsAt: 1_794_841_000, windowSeconds: 18_000 },
    { percentage: 150, status: 'danger' },
  ],
])(
  'projects window-end usage when the current usage is %s',
  (_description, window, expected) => {
    expect(getUsageLandingEstimate(window, 1_794_832_000)).toEqual(expected)
  },
)

it.each([
  [
    'the window has barely elapsed',
    { usedPercentage: 1, resetsAt: 1_794_849_280, windowSeconds: 18_000 },
  ],
  [
    'the window has reset',
    { usedPercentage: 88, resetsAt: 1_794_831_900, windowSeconds: 18_000 },
  ],
  [
    'there is no reset time',
    { usedPercentage: 88, resetsAt: null, windowSeconds: 18_000 },
  ],
])('omits a landing estimate when %s', (_description, window) => {
  expect(getUsageLandingEstimate(window, 1_794_832_000)).toEqual(null)
})

it('calculates a landing estimate once the window reaches five percent', () => {
  expect(
    getUsageLandingEstimate(
      {
        usedPercentage: 1,
        resetsAt: 1_794_849_100,
        windowSeconds: 18_000,
      },
      1_794_832_000,
    ),
  ).toEqual({ percentage: 20, status: 'on-pace' })
})

it.each([
  ['normal', 52, 'normal'],
  ['warning', 82, 'warning'],
  ['danger', 97, 'danger'],
])('classifies %s quota usage', (_description, used, expected) => {
  expect(getUsageSeverity(used)).toEqual(expected)
})

it('accepts a complete usage response through the request API', async () => {
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

  stubFetchJson(complete)
  expect(await requestAgentUsage()).toEqual(ok(complete))
})

it('rejects an incomplete usage response through the request API', async () => {
  const incomplete = {
    sessions: [],
    quotas: { claude: null },
  }

  stubFetchJson(incomplete)
  expect(await requestAgentUsage()).toEqual(
    err(new Error('The server returned an invalid response')),
  )
})
