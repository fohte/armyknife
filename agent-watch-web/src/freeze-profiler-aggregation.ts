type ProfilerFrame = {
  name: string
  resourceId?: number
  line?: number
  column?: number
}

type ProfilerStack = {
  frameId: number
  parentId?: number
}

type ProfilerSample = {
  timestamp: number
  stackId?: number
}

export type ProfilerTrace = {
  resources: string[]
  frames: ProfilerFrame[]
  stacks: ProfilerStack[]
  samples: ProfilerSample[]
}

export type LongTaskFunction = {
  name: string
  file: string
  line: number | null
  column?: number
  percentage: number
}

const MAX_REPORTED_FUNCTIONS = 10
const MAX_FUNCTION_NAME_BYTES = 256
const MAX_FUNCTION_FILE_BYTES = 2_048
const UTF8_ENCODER = new TextEncoder()

export function aggregateSamples(
  traces: ProfilerTrace[],
  start: number,
  end: number,
): LongTaskFunction[] {
  const counts = new Map<
    string,
    {
      name: string
      file: string
      line: number | null
      column: number | undefined
      samples: number
    }
  >()
  let totalSamples = 0

  for (const trace of traces) {
    for (const sample of trace.samples) {
      if (
        sample.timestamp < start ||
        sample.timestamp > end ||
        sample.stackId === undefined
      ) {
        continue
      }

      let stackId: number | undefined = sample.stackId
      let frame: ProfilerFrame | undefined
      for (
        let depth = 0;
        stackId !== undefined && depth < trace.stacks.length;
        depth++
      ) {
        const stack: ProfilerStack | undefined = trace.stacks[stackId]
        if (stack === undefined) break
        frame = trace.frames[stack.frameId]
        if (frame !== undefined) break
        stackId = stack.parentId
      }
      if (frame === undefined) continue

      const file =
        frame.resourceId === undefined
          ? 'unknown'
          : (trace.resources[frame.resourceId] ?? 'unknown')
      const name = frame.name || '(anonymous)'
      const source = file || 'unknown'
      const line =
        frame.line !== undefined && frame.line > 0 ? frame.line : null
      const column =
        frame.column !== undefined && frame.column > 0
          ? frame.column
          : undefined
      const lineKey = line === null ? '' : String(line)
      const columnKey = column === undefined ? '' : String(column)
      const key = `${name}\0${source}\0${lineKey}\0${columnKey}`
      const existing = counts.get(key)
      if (existing === undefined) {
        counts.set(key, { name, file: source, line, column, samples: 1 })
      } else {
        existing.samples += 1
      }
      totalSamples += 1
    }
  }

  if (totalSamples === 0) return []

  return [...counts.values()]
    .map(({ name, file, line, column, samples }) => ({
      name,
      file,
      line,
      column,
      percentage: Math.round((samples / totalSamples) * 1_000) / 10,
      sampleCount: samples,
    }))
    .sort(
      (left, right) =>
        right.sampleCount - left.sampleCount ||
        left.name.localeCompare(right.name) ||
        left.file.localeCompare(right.file) ||
        (left.line ?? 0) - (right.line ?? 0) ||
        (left.column ?? 0) - (right.column ?? 0),
    )
    .slice(0, MAX_REPORTED_FUNCTIONS)
    .map(({ name, file, line, column, percentage }) => ({
      name: truncateUtf8(name, MAX_FUNCTION_NAME_BYTES),
      file: truncateUtf8(file, MAX_FUNCTION_FILE_BYTES),
      line,
      ...(column === undefined ? {} : { column }),
      percentage,
    }))
}

function truncateUtf8(value: string, maxBytes: number) {
  const buffer = new Uint8Array(maxBytes)
  const { read } = UTF8_ENCODER.encodeInto(value, buffer)
  return value.slice(0, read)
}
