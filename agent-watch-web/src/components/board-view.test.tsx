import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'

import { buildBoardProjects } from '#board-model'
import { BoardView } from '#components/board-view'
import type { Session } from '#types'

let container: HTMLDivElement
let root: Root | null = null

const pausedSession: Session = {
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

const completedSession: Session = {
  ...pausedSession,
  id: 'completed-session',
  isPaused: false,
  title: 'Document the deployment procedure',
}

function renderBoard(sessions: Session[]) {
  root = createRoot(container)
  act(() => {
    root?.render(
      <BoardView
        projects={buildBoardProjects(sessions)}
        loaded
        error={null}
        hideQuietProjects={false}
        onHideQuietProjectsChange={() => undefined}
        onSelectSession={() => undefined}
      />,
    )
  })
}

function snapshot() {
  return {
    group: container.querySelector('.board-group')?.getAttribute('aria-label'),
    badge: container.querySelector('.board-done-summary [data-slot="chip"]')
      ?.textContent,
    pausedTitle: container.querySelector(
      '.board-done-paused-session span:last-child',
    )?.textContent,
    summary: container.querySelector('.board-done-summary')?.textContent,
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

it('identifies paused sessions in the Done group', () => {
  renderBoard([pausedSession])

  expect(snapshot()).toEqual({
    group: 'Done (1)',
    badge: 'Paused',
    pausedTitle: 'Prepare the next deployment window',
    summary: 'PausedPrepare the next deployment window',
  })
})

it('shows paused sessions beside the count of other Done sessions', () => {
  renderBoard([pausedSession, completedSession])

  expect(snapshot()).toEqual({
    group: 'Done (2)',
    badge: 'Paused',
    pausedTitle: 'Prepare the next deployment window',
    summary: 'PausedPrepare the next deployment window · 1 other done',
  })
})
