import { Result } from 'neverthrow'

import {
  aggregateSamples,
  type LongTaskFunction,
  type ProfilerTrace,
} from '#freeze-profiler-aggregation'
import { postLongTaskReport } from '#freeze-profiler-http'

const SAMPLE_INTERVAL_MS = 10
const MAX_BUFFER_SIZE = 10_000
const ROTATION_INTERVAL_MS = 30_000
const RETAINED_PROFILE_MS = 120_000
const MIN_LONG_TASK_MS = 1_000
const MAX_PENDING_LONG_TASKS = 10

type ProfilerInstance = {
  stop(): Promise<ProfilerTrace>
}

type ProfilerConstructor = new (options: {
  sampleInterval: number
  maxBufferSize: number
}) => ProfilerInstance

type ProfileSegment = {
  start: number
  end: number
  trace: Promise<ProfilerTrace | null>
}

export type LongTaskReport = {
  startedAtMs: number
  durationMs: number
  functions: LongTaskFunction[]
}

export function startFreezeProfiler() {
  if (
    typeof window === 'undefined' ||
    typeof PerformanceObserver === 'undefined'
  ) {
    return () => undefined
  }

  const Profiler = (window as Window & { Profiler?: ProfilerConstructor })
    .Profiler
  if (Profiler === undefined) return () => undefined

  const segments: ProfileSegment[] = []
  const initialProfiler = Result.fromThrowable(
    () => createProfiler(Profiler),
    toError,
  )()
  if (initialProfiler.isErr()) return () => undefined

  let activeProfiler: ProfilerInstance | null = initialProfiler.value
  let activeStartedAt = performance.now()
  const pendingLongTasks: PerformanceEntry[] = []

  const queueLongTasks = (tasks: PerformanceEntry[]) => {
    const retainedAfter = performance.now() - RETAINED_PROFILE_MS
    let droppedCount = 0
    for (let index = pendingLongTasks.length - 1; index >= 0; index--) {
      const task = pendingLongTasks[index]
      if (
        task !== undefined &&
        task.startTime + task.duration < retainedAfter
      ) {
        pendingLongTasks.splice(index, 1)
        droppedCount += 1
      }
    }

    const recentTasks = tasks.filter(
      (task) => task.startTime + task.duration >= retainedAfter,
    )
    droppedCount += tasks.length - recentTasks.length
    const overflow = Math.max(
      0,
      pendingLongTasks.length + recentTasks.length - MAX_PENDING_LONG_TASKS,
    )
    if (overflow > 0) {
      const pendingOverflow = Math.min(overflow, pendingLongTasks.length)
      pendingLongTasks.splice(0, pendingOverflow)
      recentTasks.splice(0, overflow - pendingOverflow)
      droppedCount += overflow
    }

    if (droppedCount > 0) {
      console.warn(
        `Dropped ${String(droppedCount)} stale or excess watch-web long-task profiles`,
      )
    }
    pendingLongTasks.push(...recentTasks)
  }

  const sendPendingReports = () => {
    const tasks = pendingLongTasks.splice(0)
    const availableSegments = [...segments]
    for (const task of tasks) {
      const start = task.startTime
      const end = start + task.duration
      const taskSegments = availableSegments.filter(
        (segment) => segment.start <= end && segment.end >= start,
      )
      if (taskSegments.length === 0) {
        console.warn('No retained JavaScript profile for watch-web long task')
        continue
      }
      void Promise.all(taskSegments.map((segment) => segment.trace)).then(
        (resolvedTraces) => {
          const report: LongTaskReport = {
            startedAtMs: Math.round(performance.timeOrigin + start),
            durationMs: task.duration,
            functions: aggregateSamples(
              resolvedTraces.filter(
                (trace): trace is ProfilerTrace => trace !== null,
              ),
              start,
              end,
            ),
          }
          return postLongTaskReport(report)
        },
      )
    }
  }

  const rotate = () => {
    const previous = activeProfiler
    if (previous === null) return

    const startedAt = activeStartedAt
    const nextProfiler = Result.fromThrowable(
      () => createProfiler(Profiler),
      toError,
    )()
    if (nextProfiler.isErr()) {
      console.warn(
        'Failed to rotate watch-web JavaScript profiler',
        nextProfiler.error,
      )
      return
    }

    const endedAt = performance.now()
    activeProfiler = nextProfiler.value
    activeStartedAt = endedAt
    const trace = stopProfiler(previous)
    segments.push({ start: startedAt, end: endedAt, trace })
    while (segments.length > 0) {
      const oldestSegment = segments[0]
      if (
        oldestSegment === undefined ||
        oldestSegment.end >= endedAt - RETAINED_PROFILE_MS
      ) {
        break
      }
      segments.shift()
    }
    sendPendingReports()
  }

  const reportLongTasks = (entries: PerformanceEntry[]) => {
    const longTasks = entries.filter(
      (entry) => entry.duration >= MIN_LONG_TASK_MS,
    )
    if (longTasks.length === 0) return

    queueLongTasks(longTasks)
    rotate()
  }

  const observerResult = Result.fromThrowable(() => {
    const observer = new PerformanceObserver((list) => {
      reportLongTasks(list.getEntries())
    })
    observer.observe({ type: 'longtask', buffered: true })
    return observer
  }, toError)()
  if (observerResult.isErr()) {
    void stopProfiler(activeProfiler)
    return () => undefined
  }
  const observer = observerResult.value

  const rotationTimer = window.setInterval(rotate, ROTATION_INTERVAL_MS)
  return () => {
    window.clearInterval(rotationTimer)
    observer.disconnect()
    const previous = activeProfiler
    if (previous === null) return
    activeProfiler = null
    const endedAt = performance.now()
    const trace = stopProfiler(previous)
    segments.push({ start: activeStartedAt, end: endedAt, trace })
    sendPendingReports()
  }
}

function createProfiler(Profiler: ProfilerConstructor) {
  return new Profiler({
    sampleInterval: SAMPLE_INTERVAL_MS,
    maxBufferSize: MAX_BUFFER_SIZE,
  })
}

function toError(error: unknown) {
  return error instanceof Error
    ? error
    : new Error('JavaScript profiling is unavailable', { cause: error })
}

function stopProfiler(profiler: ProfilerInstance) {
  return profiler.stop().then(
    (trace) => trace,
    (error: unknown) => {
      console.warn('Failed to stop watch-web JavaScript profiler', error)
      return null
    },
  )
}
