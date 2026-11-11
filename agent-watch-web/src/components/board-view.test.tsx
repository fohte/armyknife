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

function overflowHintSnapshot(wrapper: HTMLElement, scroller: HTMLElement) {
  const hint = wrapper.querySelector<HTMLElement>('.board-overflow-hint')
  return {
    hintText: hint?.textContent,
    hintIsOutsideScroller: hint !== null && !scroller.contains(hint),
    wrapperPosition: getComputedStyle(wrapper).position,
    hintPosition: hint === null ? null : getComputedStyle(hint).position,
    hintIsAlignedWithWrapperRight:
      hint?.getBoundingClientRect().right ===
      wrapper.getBoundingClientRect().right,
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

it('shows the primary task and the count of other linked tasks on a card', () => {
  renderBoard([
    {
      ...completedSession,
      category: 'needs_action',
      kind: 'Question',
      task: {
        id: 'primary-task',
        number: 42,
        title: 'Improve delivery reliability',
        priority: 1,
      },
      tasks: [
        {
          id: 'primary-task',
          number: 42,
          title: 'Improve delivery reliability',
          priority: 1,
        },
        {
          id: 'linked-task',
          number: 43,
          title: 'Document delivery behavior',
          priority: null,
        },
      ],
    },
  ])

  expect(container.querySelector('.board-card-task')?.textContent).toBe(
    'P1 #42 Improve delivery reliability +1',
  )
})

it('keeps the overflow hint at the visible edge while projects are scrolled', async () => {
  container.style.cssText = 'display: flex; width: 320px; height: 320px'
  renderBoard(
    [
      'sample project 1',
      'sample project 2',
      'sample project 3',
      'sample project 4',
    ].map((project, index) => ({
      ...completedSession,
      id: `overflow-session-${String(index)}`,
      project,
    })),
  )

  const wrapper = container.querySelector<HTMLElement>(
    '.board-projects-container',
  )
  const scroller = container.querySelector<HTMLElement>(
    '.board-projects-scroll',
  )
  const projects = Array.from(
    container.querySelectorAll<HTMLElement>('.board-project'),
  )
  if (wrapper === null || scroller === null) {
    throw new Error('Expected the board projects viewport to render')
  }

  Object.defineProperties(scroller, {
    clientWidth: { configurable: true, value: 300 },
    scrollLeft: { configurable: true, value: 300 },
  })
  projects.forEach((project, index) => {
    Object.defineProperties(project, {
      offsetLeft: { configurable: true, value: index * 300 },
      clientWidth: { configurable: true, value: 280 },
    })
  })
  await act(() => Promise.resolve(scroller.dispatchEvent(new Event('scroll'))))

  expect(overflowHintSnapshot(wrapper, scroller)).toEqual({
    hintText: '2 more →',
    hintIsOutsideScroller: true,
    wrapperPosition: 'relative',
    hintPosition: 'absolute',
    hintIsAlignedWithWrapperRight: true,
  })
})
