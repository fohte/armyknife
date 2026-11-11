import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { userEvent } from 'vitest/browser'

import { RunningTasksPanel } from '#components/running-tasks-panel'
import type { BackgroundTask } from '#types'

let container: HTMLDivElement
let root: Root | null

const tasks: BackgroundTask[] = [
  {
    id: 'fixture-subagent',
    description: 'Inspect the fictional module',
    elapsedSeconds: 10,
    kind: 'subagent',
    agentType: 'FixtureExplorer',
  },
  {
    id: 'fixture-background-task',
    description: 'cargo test -p fictional-crate',
    elapsedSeconds: 9,
    kind: 'background_task',
  },
]

beforeEach(() => {
  container = document.createElement('div')
  document.body.append(container)
  root = createRoot(container)
})

afterEach(() => {
  if (root !== null) act(() => root?.unmount())
  container.remove()
  vi.unstubAllGlobals()
})

it('shows and refreshes output only for a background task', async () => {
  const urls: string[] = []
  const fetcher = vi.fn<typeof fetch>((input) => {
    urls.push(
      typeof input === 'string'
        ? input
        : input instanceof URL
          ? input.href
          : input.url,
    )
    return Promise.resolve(new Response(`output ${String(urls.length)}`))
  })
  vi.stubGlobal('fetch', fetcher)

  act(() =>
    root?.render(
      <RunningTasksPanel sessionId="fixture-session" tasks={tasks} />,
    ),
  )
  const outputButton = container.querySelector('button')
  expect(
    Array.from(
      container.querySelectorAll('button'),
      (button) => button.textContent,
    ),
  ).toEqual(['Output'])

  await act(async () => {
    if (outputButton !== null) await userEvent.click(outputButton)
    await new Promise((resolve) => window.setTimeout(resolve, 0))
  })
  const opened = () => ({
    expanded: container.querySelector('button')?.getAttribute('aria-expanded'),
    output: container.querySelector('pre')?.textContent,
    urls,
  })
  expect(opened()).toEqual({
    expanded: 'true',
    output: 'output 1',
    urls: [
      '/api/sessions/fixture-session/background-tasks/fixture-background-task/output',
    ],
  })

  await act(async () => {
    await new Promise((resolve) => window.setTimeout(resolve, 1_050))
  })
  expect(opened()).toEqual({
    expanded: 'true',
    output: 'output 2',
    urls: [
      '/api/sessions/fixture-session/background-tasks/fixture-background-task/output',
      '/api/sessions/fixture-session/background-tasks/fixture-background-task/output',
    ],
  })
})
