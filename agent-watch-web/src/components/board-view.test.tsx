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

function renderBoard(
  sessions: Session[],
  onHideQuietProjectsChange: (checked: boolean) => void = () => undefined,
) {
  root = createRoot(container)
  act(() => {
    root?.render(
      <BoardView
        projects={buildBoardProjects(sessions)}
        loaded
        error={null}
        hideQuietProjects={false}
        onHideQuietProjectsChange={onHideQuietProjectsChange}
        onNewSession={() => undefined}
        onSelectSession={() => undefined}
      />,
    )
  })
}

function snapshot() {
  return {
    group: container
      .querySelector('.board-group.done')
      ?.getAttribute('aria-label'),
    heading: container.querySelector('.board-group.done h3')?.textContent,
    badge: container.querySelector('.board-done-summary [data-slot="chip"]')
      ?.textContent,
    summary: container.querySelector('.board-done-summary')?.textContent,
  }
}

function moreButtonSnapshot(more: HTMLButtonElement | null) {
  return {
    type: more?.type,
    text: more?.textContent,
    buttonCount: container.querySelectorAll('.board-more').length,
    groupLabel: container
      .querySelector('.board-group.in-progress')
      ?.getAttribute('aria-label'),
    groupHeading: container.querySelector('.board-group.in-progress h3')
      ?.textContent,
  }
}

function quietToggleSnapshot(
  input: HTMLInputElement | null,
  onChange: ReturnType<typeof vi.fn>,
) {
  return {
    type: input?.type,
    checked: input?.checked,
    label: input?.parentElement?.textContent,
    changeCalls: onChange.mock.calls,
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
    heading: undefined,
    badge: 'Paused',
    summary: '✓ Done · PausedPrepare the next deployment window',
  })
})

it('shows paused sessions beside the count of other Done sessions', () => {
  renderBoard([pausedSession, completedSession])

  expect(snapshot()).toEqual({
    group: 'Done (2)',
    heading: undefined,
    badge: 'Paused',
    summary: '✓ Done · PausedPrepare the next deployment window and 1 more',
  })
})

it('renders the overflow summary as a button without expanding sessions', () => {
  renderBoard(
    Array.from({ length: 4 }, (_, index) => ({
      ...completedSession,
      id: `working-session-${String(index)}`,
      category: 'in_progress' as const,
    })),
  )

  const more = container.querySelector<HTMLButtonElement>('.board-more')
  act(() => more?.click())

  expect(moreButtonSnapshot(more)).toEqual({
    type: 'button',
    text: '1 more',
    buttonCount: 1,
    groupLabel: 'In progress (4)',
    groupHeading: 'In progress',
  })
})

it('keeps the quiet-project control as a labeled native checkbox', () => {
  const onHideQuietProjectsChange = vi.fn()
  renderBoard([completedSession], onHideQuietProjectsChange)

  const input = container.querySelector<HTMLInputElement>(
    '.board-quiet-toggle input',
  )
  act(() => input?.click())

  expect(quietToggleSnapshot(input, onHideQuietProjectsChange)).toEqual({
    type: 'checkbox',
    checked: false,
    label: 'Hide quiet projects (1)',
    changeCalls: [[true]],
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

  const rowSnapshot = () => ({
    title: container.querySelector('.board-card .session-title')?.textContent,
    task: container.querySelector('.board-card .session-task')?.textContent,
    repo: container.querySelector('.board-card .session-repo')?.textContent,
  })
  expect(rowSnapshot()).toEqual({
    title: 'Document the deployment procedure',
    task: 'P1 #42 Improve delivery reliability +1',
    repo: 'example/service',
  })
})

it('shows the task placeholder on cards without an assigned task', () => {
  renderBoard([
    {
      ...completedSession,
      category: 'needs_action',
      kind: 'Plan',
      task: null,
    },
  ])

  const rowSnapshot = () => ({
    task: container.querySelector('.board-card .session-task')?.textContent,
    repo: container.querySelector('.board-card .session-repo')?.textContent,
  })
  expect(rowSnapshot()).toEqual({ task: '— No task', repo: 'example/service' })
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
