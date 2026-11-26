import { afterEach, expect, it, vi } from 'vitest'

import { type LongTaskReport, startFreezeProfiler } from '#freeze-profiler'
import { aggregateSamples } from '#freeze-profiler-aggregation'
import { postLongTaskReport } from '#freeze-profiler-http'

afterEach(() => {
  vi.unstubAllGlobals()
  vi.restoreAllMocks()
})

const report: LongTaskReport = {
  startedAtMs: 1_795_737_600_000,
  durationMs: 8_200,
  functions: [
    {
      name: 'renderList',
      file: '/assets/app.js',
      line: 42,
      percentage: 75,
    },
  ],
}

it('aggregates samples by the active function within the long task interval', () => {
  expect(
    aggregateSamples(
      [
        {
          resources: ['/assets/app.js'],
          frames: [
            { name: 'renderList', resourceId: 0, line: 42 },
            { name: 'updateRow', resourceId: 0, line: 8 },
          ],
          stacks: [{ frameId: 0 }, { frameId: 1 }],
          samples: [
            { timestamp: 9, stackId: 0 },
            { timestamp: 10, stackId: 0 },
            { timestamp: 15, stackId: 1 },
            { timestamp: 20, stackId: 1 },
            { timestamp: 21, stackId: 0 },
          ],
        },
      ],
      10,
      20,
    ),
  ).toEqual([
    {
      name: 'updateRow',
      file: '/assets/app.js',
      line: 8,
      percentage: 66.7,
    },
    {
      name: 'renderList',
      file: '/assets/app.js',
      line: 42,
      percentage: 33.3,
    },
  ])
})

it('labels anonymous samples without a source line', () => {
  expect(
    aggregateSamples(
      [
        {
          resources: [''],
          frames: [{ name: '', resourceId: 0, line: 0 }],
          stacks: [{ frameId: 0 }],
          samples: [{ timestamp: 10, stackId: 0 }],
        },
      ],
      10,
      10,
    ),
  ).toEqual([
    {
      name: '(anonymous)',
      file: 'unknown',
      line: null,
      percentage: 100,
    },
  ])
})

it('sends the long task report to the watch-web endpoint', async () => {
  const fetchMock = vi
    .fn<typeof fetch>()
    .mockResolvedValue(new Response(null, { status: 204 }))
  vi.stubGlobal('fetch', fetchMock)

  await postLongTaskReport(report)

  expect(fetchMock.mock.calls).toEqual([
    [
      '/api/long-tasks',
      {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify(report),
        keepalive: true,
      },
    ],
  ])
})

const longTaskScenarios = [
  {
    name: 'profiler rotation succeeds',
    failOnSecondProfiler: false,
    failOnFirstStop: false,
    flush: 'event',
  },
  {
    name: 'replacement profiler construction is retried',
    failOnSecondProfiler: true,
    failOnFirstStop: false,
    flush: 'timer',
  },
  {
    name: 'cleanup flushes a pending report after rotation fails',
    failOnSecondProfiler: true,
    failOnFirstStop: false,
    flush: 'cleanup',
  },
  {
    name: 'profiler stop fails',
    failOnSecondProfiler: false,
    failOnFirstStop: true,
    flush: 'event',
  },
] as const

it.each(longTaskScenarios)(
  'sends a long task report when $name',
  async ({ failOnSecondProfiler, failOnFirstStop, flush }) => {
    const taskStart = performance.now()
    const expectedReport: LongTaskReport = {
      startedAtMs: Math.round(performance.timeOrigin + taskStart),
      durationMs: 1_000,
      functions: failOnFirstStop
        ? []
        : [
            {
              name: 'slowTask',
              file: '/assets/app.js',
              line: 12,
              percentage: 100,
            },
          ],
    }
    const {
      deliverLongTasks,
      fetchMock,
      fetchStarted,
      observerOptions,
      profilerCreationError,
      profilerStopError,
      triggerRotation,
    } = installProfilerEnvironment(taskStart + 500, {
      failOnFirstStop,
      failOnSecondProfiler,
    })
    const profilerWarning = vi
      .spyOn(console, 'warn')
      .mockImplementation(() => undefined)
    const cleanup = startFreezeProfiler()

    deliverLongTasks([longTaskEntry(taskStart)])
    if (flush === 'timer') triggerRotation()
    if (flush === 'cleanup') cleanup()
    await fetchStarted

    const expectedWarnings: unknown[][] = []
    if (failOnSecondProfiler) {
      expectedWarnings.push([
        'Failed to rotate watch-web JavaScript profiler',
        profilerCreationError,
      ])
    }
    if (failOnFirstStop) {
      expectedWarnings.push([
        'Failed to stop watch-web JavaScript profiler',
        profilerStopError,
      ])
    }

    const actual = profilerSnapshot(
      observerOptions(),
      fetchMock.mock.calls,
      profilerWarning.mock.calls,
    )
    const expected = profilerSnapshot(
      { type: 'longtask', buffered: true },
      [
        [
          '/api/long-tasks',
          {
            method: 'POST',
            headers: { 'content-type': 'application/json' },
            body: JSON.stringify(expectedReport),
            keepalive: true,
          },
        ],
      ],
      expectedWarnings,
    )
    expect(actual).toEqual(expected)

    if (flush !== 'cleanup') cleanup()
  },
)

it('caps pending reports across callbacks and retains the newest task', async () => {
  const taskStart = performance.now()
  const {
    deliverLongTasks,
    fetchMock,
    fetchStarted,
    observerOptions,
    profilerCreationError,
  } = installProfilerEnvironment(taskStart + 500, {
    failOnSecondProfiler: true,
  })
  const warning = vi.spyOn(console, 'warn').mockImplementation(() => undefined)
  startFreezeProfiler()

  deliverLongTasks(
    Array.from({ length: 10 }, (_, index) =>
      longTaskEntry(taskStart - 100 + index),
    ),
  )
  deliverLongTasks([longTaskEntry(taskStart - 50)])
  await fetchStarted
  await new Promise<void>((resolve) => setTimeout(resolve, 0))

  const reportFor = (startTime: number): LongTaskReport => ({
    startedAtMs: Math.round(performance.timeOrigin + startTime),
    durationMs: 1_000,
    functions: [
      {
        name: 'slowTask',
        file: '/assets/app.js',
        line: 12,
        percentage: 100,
      },
    ],
  })
  const expectedFetchCalls = [
    ...Array.from({ length: 9 }, (_, index) => -99 + index),
    -50,
  ].map((offset) => [
    '/api/long-tasks',
    {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify(reportFor(taskStart + offset)),
      keepalive: true,
    },
  ])

  const actual = profilerSnapshot(
    observerOptions(),
    fetchMock.mock.calls,
    warning.mock.calls,
  )
  const expected = profilerSnapshot(
    { type: 'longtask', buffered: true },
    expectedFetchCalls,
    [
      ['Failed to rotate watch-web JavaScript profiler', profilerCreationError],
      ['Dropped 1 stale or excess watch-web long-task profiles'],
    ],
  )
  expect(actual).toEqual(expected)
})

it('warns when the long task report cannot be sent', async () => {
  const error = new Error('Network unavailable')
  const fetchMock = vi.fn<typeof fetch>().mockRejectedValue(error)
  const warn = vi.spyOn(console, 'warn').mockImplementation(() => undefined)
  vi.stubGlobal('fetch', fetchMock)

  await postLongTaskReport(report)

  expect(warn.mock.calls).toEqual([
    ['Failed to send watch-web long-task profile', error],
  ])
})

it('does not create an observer when the browser does not expose Profiler', () => {
  let observerConstructions = 0
  class TestPerformanceObserver {
    constructor() {
      observerConstructions += 1
    }
    observe() {}
    disconnect() {}
    takeRecords(): PerformanceEntry[] {
      return []
    }
  }

  vi.stubGlobal('window', { Profiler: undefined })
  vi.stubGlobal('PerformanceObserver', TestPerformanceObserver)

  startFreezeProfiler()

  expect(observerConstructions).toEqual(0)
})

function longTaskEntry(startTime: number): PerformanceEntry {
  return {
    duration: 1_000,
    entryType: 'longtask',
    name: 'self',
    startTime,
    toJSON: () => ({}),
  }
}

function profilerSnapshot<ObserverOptions, FetchCalls, Warnings>(
  observerOptions: ObserverOptions,
  fetchCalls: FetchCalls,
  warnings: Warnings,
) {
  return { observerOptions, fetchCalls, warnings }
}

type ProfilerEnvironmentOptions = {
  failOnSecondProfiler?: boolean
  failOnFirstStop?: boolean
}

function installProfilerEnvironment(
  sampleTimestamp: number,
  options: ProfilerEnvironmentOptions,
) {
  let profilerConstructions = 0
  let profilerStops = 0
  let observerOptions: PerformanceObserverInit | undefined
  let rotateOnTimer = () => {}
  const triggerRotation = () => {
    rotateOnTimer()
  }
  const profilerCreationError = new Error('Profiler unavailable')
  const profilerStopError = new Error('Profiler trace unavailable')
  let notifyLongTasks: (entries: PerformanceEntry[]) => void = () => {}
  const deliverLongTasks = (entries: PerformanceEntry[]) => {
    if (
      observerOptions?.type !== 'longtask' ||
      observerOptions.buffered !== true
    ) {
      throw new Error('The long-task observer was not registered')
    }
    notifyLongTasks(entries)
  }
  class TestProfiler {
    constructor() {
      profilerConstructions += 1
      if (
        options.failOnSecondProfiler === true &&
        profilerConstructions === 2
      ) {
        throw profilerCreationError
      }
    }

    stop() {
      profilerStops += 1
      if (options.failOnFirstStop === true && profilerStops === 1) {
        return Promise.reject(profilerStopError)
      }
      return Promise.resolve({
        resources: ['/assets/app.js'],
        frames: [{ name: 'slowTask', resourceId: 0, line: 12 }],
        stacks: [{ frameId: 0 }],
        samples: [{ timestamp: sampleTimestamp, stackId: 0 }],
      })
    }
  }

  class TestPerformanceObserver implements PerformanceObserver {
    constructor(callback: PerformanceObserverCallback) {
      notifyLongTasks = (entries) => {
        const list: PerformanceObserverEntryList = {
          getEntries: () => entries,
          getEntriesByName: () => [],
          getEntriesByType: () => [],
        }
        callback(list, this)
      }
    }

    observe(options?: PerformanceObserverInit) {
      observerOptions = options
    }
    disconnect() {}
    takeRecords(): PerformanceEntry[] {
      return []
    }
  }

  let resolveFetchStarted!: () => void
  const fetchStarted = new Promise<void>((resolve) => {
    resolveFetchStarted = resolve
  })
  const fetchMock = vi.fn<typeof fetch>().mockImplementation(() => {
    resolveFetchStarted()
    return Promise.resolve(new Response(null, { status: 204 }))
  })
  const setInterval = vi.fn((callback: () => void) => {
    rotateOnTimer = callback
    return 1
  })

  vi.stubGlobal('window', {
    Profiler: TestProfiler,
    setInterval,
    clearInterval: vi.fn(),
  })
  vi.stubGlobal('PerformanceObserver', TestPerformanceObserver)
  vi.stubGlobal('fetch', fetchMock)

  return {
    deliverLongTasks,
    fetchMock,
    fetchStarted,
    observerOptions: () => observerOptions,
    profilerCreationError,
    profilerStopError,
    triggerRotation,
  }
}
