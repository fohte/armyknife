import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'

import { SessionRow } from '#components/session-row'
import type { Session } from '#types'

let container: HTMLDivElement
let root: Root | null = null

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
    repo: 'example/service',
    project: 'sample project',
    parent: null,
    ancestorSessionIds: [],
    pullRequest: null,
  }

  root = createRoot(container)
  act(() => {
    root?.render(
      <ul>
        <SessionRow
          session={session}
          selected={false}
          onSelect={() => undefined}
        />
      </ul>,
    )
  })

  const snapshot = () => ({
    badge: container.querySelector('[data-slot="chip"]')?.textContent,
    age: container.querySelector('.session-age')?.textContent,
  })
  expect(snapshot()).toEqual({ badge: 'Paused', age: '2m' })
})
