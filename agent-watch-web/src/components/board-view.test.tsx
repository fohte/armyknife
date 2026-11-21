import { act, useEffect } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { page } from 'vitest/browser'

import {
  AsyncActionFeedbackProvider,
  useAsyncActionStarter,
} from '#async-action-feedback'
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
  isMobile = false,
) {
  root = createRoot(container)
  act(() => {
    root?.render(
      <BoardView
        projects={buildBoardProjects(sessions)}
        loaded
        error={null}
        hideQuietProjects={false}
        isMobile={isMobile}
        onHideQuietProjectsChange={onHideQuietProjectsChange}
        onNewSession={() => undefined}
        onSelectSession={() => undefined}
      />,
    )
  })
}

function renderPhoneBoardWithActivity() {
  function PhoneBoardWithActivity() {
    const startAsyncAction = useAsyncActionStarter(() => undefined)

    useEffect(() => {
      startAsyncAction({
        kind: 'merge',
        sessionId: 'example-session',
        sessionTitle: 'Example session',
        pullRequestNumber: 42,
        run: () => new Promise(() => undefined),
      })
    }, [startAsyncAction])

    return (
      <BoardView
        projects={buildBoardProjects([completedSession])}
        loaded
        error={null}
        hideQuietProjects={false}
        isMobile
        onHideQuietProjectsChange={() => undefined}
        onNewSession={() => undefined}
        onSelectSession={() => undefined}
      />
    )
  }

  root = createRoot(container)
  act(() => {
    root?.render(
      <AsyncActionFeedbackProvider>
        <PhoneBoardWithActivity />
      </AsyncActionFeedbackProvider>,
    )
  })
}

function paginationSnapshot() {
  return {
    ariaHidden: container
      .querySelector('.board-pagination-dots')
      ?.getAttribute('aria-hidden'),
    dots: Array.from(
      container.querySelectorAll<HTMLElement>('.board-pagination-dot'),
    ).map((dot) => dot.dataset['current']),
  }
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

function overflowSummarySnapshot() {
  const more = container.querySelector<HTMLElement>('.board-more')

  return {
    tagName: more?.tagName,
    text: more?.textContent,
    role: more?.getAttribute('role'),
    tabIndex: more?.tabIndex,
    buttonCount: container.querySelectorAll('button.board-more').length,
    cardCount: container.querySelectorAll('.board-card').length,
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
  renderBoard([completedSession, pausedSession])

  expect(snapshot()).toEqual({
    group: 'Done (2)',
    heading: undefined,
    badge: 'Paused',
    summary: '✓ Done · PausedPrepare the next deployment window and 1 more',
  })
})

it('shows the first completed session when no Done session is paused', () => {
  renderBoard([completedSession])

  expect(snapshot()).toEqual({
    group: 'Done (1)',
    heading: undefined,
    badge: undefined,
    summary: '✓ Done · Document the deployment procedure',
  })
})

it('renders the overflow summary as non-interactive text', () => {
  renderBoard(
    Array.from({ length: 4 }, (_, index) => ({
      ...completedSession,
      id: `working-session-${String(index)}`,
      category: 'in_progress' as const,
    })),
  )

  expect(overflowSummarySnapshot()).toEqual({
    tagName: 'SPAN',
    text: '1 more',
    role: null,
    tabIndex: -1,
    buttonCount: 0,
    cardCount: 3,
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

it('keeps the phone Board title and quiet-project control apart', async () => {
  await page.viewport(375, 844)
  container.style.cssText = 'display: flex; width: 375px; height: 844px'
  renderPhoneBoardWithActivity()

  const title = container.querySelector<HTMLElement>('.board-header-title')
  const quietToggle = container.querySelector<HTMLElement>(
    '.board-quiet-toggle',
  )
  const activityButton = container.querySelector(
    '.async-action-activity-button',
  )
  if (title === null || quietToggle === null || activityButton === null) {
    throw new Error('Expected the phone Board heading and filter to render')
  }

  const titleRect = title.getBoundingClientRect()
  const quietToggleRect = quietToggle.getBoundingClientRect()
  const headingRect = title.querySelector('h1')?.getBoundingClientRect()
  if (headingRect === undefined) {
    throw new Error('Expected the phone Board title to render')
  }

  const layoutSnapshot = () => ({
    headingAndFilterDoNotOverlap:
      headingRect.right <= quietToggleRect.left ||
      headingRect.bottom <= quietToggleRect.top ||
      quietToggleRect.right <= headingRect.left ||
      quietToggleRect.bottom <= headingRect.top,
    filterWrapsBelowTitle: quietToggleRect.top >= titleRect.bottom,
  })
  expect(layoutSnapshot()).toEqual({
    headingAndFilterDoNotOverlap: true,
    filterWrapsBelowTitle: true,
  })
  await page.viewport(1280, 800)
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

it('shows one active page dot for a single phone project', () => {
  renderBoard([completedSession], undefined, true)

  expect(paginationSnapshot()).toEqual({
    ariaHidden: 'true',
    dots: ['true'],
  })
})

it('updates the active page dot as phone projects scroll', () => {
  renderBoard(
    [
      { ...completedSession, id: 'first-project', project: 'first project' },
      { ...completedSession, id: 'second-project', project: 'second project' },
    ],
    undefined,
    true,
  )

  const scroller = container.querySelector<HTMLElement>(
    '.board-projects-scroll',
  )
  const columns = Array.from(
    container.querySelectorAll<HTMLElement>('.board-project'),
  )
  if (scroller === null || columns.length !== 2) {
    throw new Error('Expected two phone project columns')
  }

  Object.defineProperties(scroller, {
    clientWidth: { configurable: true, value: 390 },
    scrollLeft: { configurable: true, value: 310 },
  })
  const scrollerRect = new DOMRect(0, 0, 390, 844)
  vi.spyOn(scroller, 'getBoundingClientRect').mockReturnValue(scrollerRect)
  columns.forEach((column, index) => {
    vi.spyOn(column, 'getBoundingClientRect').mockReturnValue(
      new DOMRect(index === 0 ? -294 : 16, 0, 300, 700),
    )
  })
  act(() => {
    scroller.dispatchEvent(new Event('scroll'))
  })

  expect(paginationSnapshot()).toEqual({
    ariaHidden: 'true',
    dots: ['false', 'true'],
  })
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
