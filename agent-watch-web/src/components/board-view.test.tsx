import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'

import { buildBoardProjects } from '#board-model'
import { BoardView } from '#components/board-view'
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

it('identifies paused sessions in the Done group', () => {
  const session: Session = {
    id: 'paused-session',
    category: 'done',
    isPaused: true,
    kind: null,
    title: 'Prepare the next deployment window',
    preview: null,
    elapsedSeconds: 240,
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
      <BoardView
        projects={buildBoardProjects([session])}
        loaded
        error={null}
        hideQuietProjects={false}
        onHideQuietProjectsChange={() => undefined}
        onSelectSession={() => undefined}
      />,
    )
  })

  const snapshot = () => ({
    group: container.querySelector('.board-group')?.getAttribute('aria-label'),
    badge: container.querySelector('.board-done-summary [data-slot="chip"]')
      ?.textContent,
    title: container.querySelector('.board-done-paused-session span:last-child')
      ?.textContent,
  })
  expect(snapshot()).toEqual({
    group: 'Done (1)',
    badge: 'Paused',
    title: 'Prepare the next deployment window',
  })
})
