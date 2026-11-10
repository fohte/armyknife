import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'

import { SessionRow } from '#components/session-row'
import type { Session } from '#types'

let container: HTMLDivElement
let root: Root | null = null

const session: Session = {
  id: 'paused-session',
  category: 'done',
  isPaused: true,
  kind: null,
  title: 'Prepare the next deployment window',
  preview: null,
  elapsedSeconds: 120,
  unread: false,
  task: null,
  tasks: [],
  repo: 'example/service',
  project: 'sample project',
  parent: null,
  ancestorSessionIds: [],
  pullRequest: null,
}

function renderRow(viewSession: Session) {
  root = createRoot(container)
  act(() => {
    root?.render(
      <ul>
        <SessionRow
          session={viewSession}
          selected={false}
          onSelect={() => undefined}
        />
      </ul>,
    )
  })
}

function snapshot() {
  return {
    badge: container.querySelector('[data-slot="chip"]')?.textContent,
    age: container.querySelector('.session-age')?.textContent,
    task: container.querySelector('.session-task')?.textContent,
  }
}

beforeEach(() => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  container = document.createElement('div')
  document.body.append(container)
})

afterEach(() => {
  if (root !== null) act(() => root?.unmount())
  root = null
  container.remove()
  vi.unstubAllGlobals()
})

it('labels a paused session while keeping its elapsed time visible', () => {
  renderRow(session)

  expect(snapshot()).toEqual({
    badge: 'Paused',
    age: '2m',
    task: '— No task',
  })
})

it('keeps the merged pull request label on a paused row', () => {
  renderRow({
    ...session,
    pullRequest: { number: 2049, state: 'Merged' },
  })

  expect(snapshot()).toEqual({
    badge: 'Paused',
    age: 'Merged #2049 · 2m',
    task: '— No task',
  })
})

it('shows the primary tq task and the number of other linked tasks', () => {
  renderRow({
    ...session,
    task: {
      id: 'open-task',
      number: 2048,
      title: 'Improve request reliability',
      priority: 1,
      isClosed: false,
    },
    tasks: [
      {
        id: 'open-task',
        number: 2048,
        title: 'Improve request reliability',
        priority: 1,
        isClosed: false,
      },
      {
        id: 'completed-task',
        number: 2047,
        title: 'Document retry behavior',
        priority: null,
        isClosed: true,
      },
    ],
  })

  expect(container.querySelector('.session-task')?.textContent).toBe(
    'P1 #2048 · Improve request reliability +1',
  )
})
