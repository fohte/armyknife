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

it.each([
  ['profiler rotation succeeds', false],
  ['replacement profiler construction fails', true],
] as const)(
  'sends a long task report when %s',
  async (_, failOnSecondProfiler) => {
    const taskStart = performance.now()
    const expectedReport: LongTaskReport = {
      startedAtMs: Math.round(performance.timeOrigin + taskStart),
      durationMs: 1_000,
      functions: [
        {
          name: 'slowTask',
          file: '/assets/app.js',
          line: 12,
          percentage: 100,
        },
      ],
    }
    const {
      deliverLongTask,
      fetchMock,
      fetchStarted,
      observerOptions,
      profilerCreationError,
      triggerRotation,
    } = installProfilerEnvironment(taskStart + 500, failOnSecondProfiler)
    const profilerWarning = vi
      .spyOn(console, 'warn')
      .mockImplementation(() => undefined)
    const cleanup = startFreezeProfiler()

    expect(observerOptions()).toEqual({ type: 'longtask', buffered: true })
    deliverLongTask({
      duration: 1_000,
      entryType: 'longtask',
      name: 'self',
      startTime: taskStart,
      toJSON: () => ({}),
    })
    if (failOnSecondProfiler) triggerRotation()
    await fetchStarted

    expect(fetchMock.mock.calls).toEqual([
      [
        '/api/long-tasks',
        {
          method: 'POST',
          headers: { 'content-type': 'application/json' },
          body: JSON.stringify(expectedReport),
          keepalive: true,
        },
      ],
    ])
    expect(profilerWarning.mock.calls).toEqual(
      failOnSecondProfiler
        ? [
            [
              'Failed to rotate watch-web JavaScript profiler',
              profilerCreationError,
            ],
          ]
        : [],
    )

    cleanup()
  },
)

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

function installProfilerEnvironment(
  sampleTimestamp: number,
  failOnSecondProfiler: boolean,
) {
  let profilerConstructions = 0
  let observerOptions: PerformanceObserverInit | undefined
  let rotateOnTimer = () => {}
  const triggerRotation = () => {
    rotateOnTimer()
  }
  const profilerCreationError = new Error('Profiler unavailable')
  let notifyLongTask: (entry: PerformanceEntry) => void = () => {}
  const deliverLongTask = (entry: PerformanceEntry) => {
    notifyLongTask(entry)
  }
  class TestProfiler {
    constructor() {
      profilerConstructions += 1
      if (failOnSecondProfiler && profilerConstructions === 2) {
        throw profilerCreationError
      }
    }

    stop() {
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
      notifyLongTask = (entry) => {
        const list: PerformanceObserverEntryList = {
          getEntries: () => [entry],
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
    deliverLongTask,
    fetchMock,
    fetchStarted,
    observerOptions: () => observerOptions,
    profilerCreationError,
    triggerRotation,
  }
}
