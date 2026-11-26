import { afterEach, expect, it, vi } from 'vitest'

import { type LongTaskReport, startFreezeProfiler } from '#freeze-profiler'
import { aggregateSamples } from '#freeze-profiler-aggregation'
import { postLongTaskReport } from '#freeze-profiler-http'

afterEach(() => {
  vi.unstubAllGlobals()
})

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

it('sends the aggregated long task report to the watch-web endpoint', async () => {
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

it('leaves profiling disabled when the browser does not expose Profiler', () => {
  vi.stubGlobal('Profiler', undefined)

  expect(typeof startFreezeProfiler()).toEqual('function')
})
