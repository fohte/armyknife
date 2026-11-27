import '#index.css'

import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'

import { RunningTasksPanel } from '#components/running-tasks-panel'
import type { BackgroundTask } from '#types'

class FakeEventSource {
  static readonly CONNECTING = 0
  static readonly OPEN = 1
  static readonly CLOSED = 2

  private listeners = new Map<string, EventListener>()
  closed = false
  readyState = FakeEventSource.CONNECTING

  constructor(readonly url: string) {}

  addEventListener(type: string, listener: EventListener) {
    this.listeners.set(type, listener)
  }

  removeEventListener(type: string, listener: EventListener) {
    if (this.listeners.get(type) === listener) this.listeners.delete(type)
  }

  close() {
    this.closed = true
  }

  emit(type: string, data?: string) {
    if (type === 'open') this.readyState = FakeEventSource.OPEN
    const event =
      data === undefined ? new Event(type) : new MessageEvent(type, { data })
    this.listeners.get(type)?.(event)
  }
}

const tasks: BackgroundTask[] = [
  {
    id: 'fixture-subagent',
    description: 'Inspect a sample module',
    elapsedSeconds: 2,
    kind: 'subagent',
    agentType: 'FixtureAgent',
  },
  {
    id: 'fixture-background-task',
    description: 'Run a fixture command',
    elapsedSeconds: 1,
    kind: 'background_task',
  },
]

function getOutputButtonCounts(container: HTMLElement) {
  return {
    subagentRows: container.querySelectorAll(
      'section[aria-label="Subagents"] .running-task',
    ).length,
    subagentOutputButtons: container.querySelectorAll(
      'section[aria-label="Subagents"] .running-task-output-button',
    ).length,
    backgroundTaskOutputButtons: container.querySelectorAll(
      'section[aria-label="Background tasks"] .running-task-output-button',
    ).length,
  }
}

let container: HTMLDivElement
let root: Root | null = null
let eventSources: FakeEventSource[]

beforeEach(() => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  eventSources = []
  vi.stubGlobal(
    'EventSource',
    class extends FakeEventSource {
      constructor(url: string) {
        super(url)
        eventSources.push(this)
      }
    },
  )
  container = document.createElement('div')
  document.body.append(container)
  root = createRoot(container)
})

afterEach(() => {
  if (root !== null) act(() => root?.unmount())
  root = null
  container.remove()
  vi.unstubAllGlobals()
  vi.useRealTimers()
})

it('shows an output button only on background task rows', () => {
  act(() => {
    root?.render(
      <RunningTasksPanel sessionId="fixture-session" tasks={tasks} />,
    )
  })

  expect(getOutputButtonCounts(container)).toEqual({
    subagentRows: 1,
    subagentOutputButtons: 0,
    backgroundTaskOutputButtons: 1,
  })
})

it('opens one event stream without polling and closes it with the row', () => {
  vi.useFakeTimers()
  const fetcher = vi.fn<typeof fetch>(() => Promise.resolve(new Response('')))
  vi.stubGlobal('fetch', fetcher)
  act(() => {
    root?.render(
      <RunningTasksPanel sessionId="fixture-session" tasks={tasks} />,
    )
  })

  const outputButton = container.querySelector<HTMLButtonElement>(
    '.running-task-output-button',
  )
  const beforeOpen = {
    sourceCountBeforeOpen: eventSources.length,
    buttonExpanded: outputButton?.getAttribute('aria-expanded'),
  }

  act(() => outputButton?.click())
  const source = eventSources[0]
  const afterOpen = {
    sourceCount: eventSources.length,
    sourceUrl: source?.url,
    buttonExpanded: outputButton?.getAttribute('aria-expanded'),
  }

  act(() => {
    vi.advanceTimersByTime(2_000)
  })
  act(() => outputButton?.click())
  const snapshot = () => ({
    beforeOpen,
    afterOpen,
    fetchCalls: fetcher.mock.calls.length,
    sourceClosed: source?.closed,
    outputRemoved: container.querySelector('.running-task-output') === null,
  })
  expect(snapshot()).toEqual({
    beforeOpen: { sourceCountBeforeOpen: 0, buttonExpanded: 'false' },
    afterOpen: {
      sourceCount: 1,
      sourceUrl:
        '/api/sessions/fixture-session/background-tasks/fixture-background-task/output/events',
      buttonExpanded: 'true',
    },
    fetchCalls: 0,
    sourceClosed: true,
    outputRemoved: true,
  })
})

it('renders the pre output pane without a border', () => {
  act(() => {
    root?.render(
      <RunningTasksPanel sessionId="fixture-session" tasks={tasks} />,
    )
  })
  const outputButton = container.querySelector<HTMLButtonElement>(
    '.running-task-output-button',
  )
  act(() => outputButton?.click())
  const output = container.querySelector<HTMLElement>('pre')
  expect(output === null ? null : getComputedStyle(output).borderTopWidth).toBe(
    '0px',
  )
})

it('renders each output update received from the event stream', () => {
  act(() => {
    root?.render(
      <RunningTasksPanel sessionId="fixture-session" tasks={tasks} />,
    )
  })
  const outputButton = container.querySelector<HTMLButtonElement>(
    '.running-task-output-button',
  )
  act(() => outputButton?.click())
  const source = eventSources[0]
  act(() =>
    source?.emit('message', JSON.stringify({ output: 'first output\n' })),
  )
  const firstOutput = container.querySelector(
    '.running-task-output',
  )?.textContent
  act(() =>
    source?.emit(
      'message',
      JSON.stringify({ output: 'first output\nsecond output\n' }),
    ),
  )
  const secondOutput = container.querySelector(
    '.running-task-output',
  )?.textContent
  const snapshot = () => ({
    sourceUrl: source?.url,
    firstOutput,
    secondOutput,
  })
  expect(snapshot()).toEqual({
    sourceUrl:
      '/api/sessions/fixture-session/background-tasks/fixture-background-task/output/events',
    firstOutput: 'first output\n',
    secondOutput: 'first output\nsecond output\n',
  })
})

it('renders an empty output received from the event stream', () => {
  act(() => {
    root?.render(
      <RunningTasksPanel sessionId="fixture-session" tasks={tasks} />,
    )
  })
  const outputButton = container.querySelector<HTMLButtonElement>(
    '.running-task-output-button',
  )
  act(() => outputButton?.click())
  const source = eventSources[0]
  act(() => source?.emit('message', JSON.stringify({ output: '' })))

  expect(
    container.querySelector('.running-task-output')?.textContent ?? null,
  ).toBe('')
})

it('shows a failure when the event stream is permanently closed', () => {
  act(() => {
    root?.render(
      <RunningTasksPanel sessionId="fixture-session" tasks={tasks} />,
    )
  })
  const outputButton = container.querySelector<HTMLButtonElement>(
    '.running-task-output-button',
  )
  act(() => outputButton?.click())
  const source = eventSources[0]
  if (source !== undefined) {
    source.readyState = FakeEventSource.CLOSED
    act(() => {
      source.emit('error')
    })
  }

  expect(
    container.querySelector('.running-task-output')?.textContent ?? null,
  ).toBe('Unable to load task output.')
})
