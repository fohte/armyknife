import { expect, it } from 'vitest'

import {
  formatResetTime,
  formatTokenCount,
  getExpectedUsagePercentage,
  getUsageLandingEstimate,
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

it.each([
  [
    'five-hour window',
    { usedPercentage: 1, resetsAt: 17_999, windowSeconds: 18_000 },
    0.005555555555555556,
  ],
  [
    'seven-day window',
    { usedPercentage: 1, resetsAt: 604_799, windowSeconds: 604_800 },
    0.00016534391534391536,
  ],
])(
  'shows the expected usage line at the start of the %s',
  (_description, window, expected) => {
    expect(getExpectedUsagePercentage(window, 0)).toEqual(expected)
  },
)

it.each([
  [
    'five-hour window',
    { usedPercentage: 1, resetsAt: 18_000, windowSeconds: 18_000 },
  ],
  [
    'seven-day window',
    { usedPercentage: 1, resetsAt: 604_800, windowSeconds: 604_800 },
  ],
])(
  'omits the expected usage line at the start of the %s',
  (_description, window) => {
    expect(getExpectedUsagePercentage(window, 0)).toEqual(null)
  },
)

it.each([
  [
    'five-hour window',
    { usedPercentage: 1, resetsAt: 17_999, windowSeconds: 18_000 },
    18_000,
  ],
  [
    'seven-day window',
    { usedPercentage: 1, resetsAt: 604_799, windowSeconds: 604_800 },
    604_800,
  ],
])(
  'calculates the landing estimate from the start of the %s',
  (_description, window, expected) => {
    expect(getUsageLandingEstimate(window, 0)).toEqual({
      percentage: expected,
      status: 'danger',
    })
  },
)

it.each([
  [
    'five-hour window',
    { usedPercentage: 1, resetsAt: 18_000, windowSeconds: 18_000 },
  ],
  [
    'seven-day window',
    { usedPercentage: 1, resetsAt: 604_800, windowSeconds: 604_800 },
  ],
])(
  'omits the landing estimate at the start of the %s',
  (_description, window) => {
    expect(getUsageLandingEstimate(window, 0)).toEqual(null)
  },
)

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

it.each([
  [
    'the window has reset',
    { usedPercentage: 88, resetsAt: 1_794_831_900, windowSeconds: 18_000 },
  ],
  [
    'there is no reset time',
    { usedPercentage: 88, resetsAt: null, windowSeconds: 18_000 },
  ],
])('omits the expected usage line when %s', (_description, window) => {
  expect(getExpectedUsagePercentage(window, 1_794_832_000)).toEqual(null)
})

it.each([
  ['normal', 52, 'normal'],
  ['warning', 82, 'warning'],
  ['danger', 97, 'danger'],
])('classifies %s quota usage', (_description, used, expected) => {
  expect(getUsageSeverity(used)).toEqual(expected)
})

it('validates complete usage payloads received over the event stream', () => {
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

  const incomplete = {
    sessions: [],
    quotas: { claude: null },
  }

  const result = JSON.stringify({
    complete: isAgentUsage(complete),
    incomplete: isAgentUsage(incomplete),
  })

  expect(result).toEqual('{"complete":true,"incomplete":false}')
})
