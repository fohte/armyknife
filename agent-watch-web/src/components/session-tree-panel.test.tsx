import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'

import { SessionTreePanel } from '#components/session-tree-panel'
import type { SessionTreeNode } from '#types'

const sessions: SessionTreeNode[] = [
  {
    id: 'root',
    parentId: null,
    engine: 'claude',
    workType: null,
    category: 'done',
    isActive: true,
    kind: null,
    title: 'Root session',
    preview: null,
    elapsedSeconds: 300,
    task: null,
    repo: 'sample/repo',
    vibeMode: { mode: 'enabled' },
    decisionCount: 0,
    pullRequest: null,
  },
  {
    id: 'active-child',
    parentId: 'root',
    engine: 'codex',
    workType: 'development',
    category: 'needs_action',
    isActive: true,
    kind: 'Question',
    title: 'Active child',
    preview: 'Needs an answer',
    elapsedSeconds: 8,
    task: {
      id: 'task-id',
      number: 17,
      title: 'Sample task',
      priority: null,
    },
    repo: 'sample/other-repo',
    vibeMode: { mode: 'inherited', ancestorSessionId: 'root' },
    decisionCount: 0,
    pullRequest: null,
  },
  {
    id: 'ended-child',
    parentId: 'active-child',
    engine: 'claude',
    workType: 'PM',
    category: 'done',
    isActive: false,
    kind: null,
    title: 'Ended child',
    preview: null,
    elapsedSeconds: 3_600,
    task: null,
    repo: 'sample/other-repo',
    vibeMode: { mode: 'disabled' },
    decisionCount: 0,
    pullRequest: null,
  },
]

let container: HTMLDivElement
let root: Root | null = null

function renderTree(
  onSelectSession = vi.fn(),
  size: 'desktop' | 'phone' = 'desktop',
) {
  root = createRoot(container)
  act(() => {
    root?.render(
      <SessionTreePanel
        sessions={sessions}
        size={size}
        selectedSessionId="root"
        onSelectSession={onSelectSession}
      />,
    )
  })
  return onSelectSession
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
  vi.restoreAllMocks()
  vi.useRealTimers()
  vi.unstubAllGlobals()
})

it('advances elapsed time without a panel interval', async () => {
  vi.useFakeTimers()
  vi.setSystemTime(new Date('2026-11-10T00:00:00.000Z'))
  const intervalSpy = vi.spyOn(window, 'setInterval')
  renderTree()

  const before = Array.from(
    container.querySelectorAll<HTMLElement>('.session-age'),
    (element) => element.textContent,
  )
  await act(async () => {
    await vi.advanceTimersByTimeAsync(1_000)
  })
  const after = Array.from(
    container.querySelectorAll<HTMLElement>('.session-age'),
    (element) => element.textContent,
  )

  const snapshot = () => ({
    before,
    after,
    intervalCount: intervalSpy.mock.calls.length,
  })
  expect(snapshot()).toEqual({
    before: ['5m', '8s', '1h'],
    after: ['5m', '9s', '1h'],
    intervalCount: 1,
  })
})

it('clears the shared interval when the tree releases its clock subscription', () => {
  vi.useFakeTimers()
  const intervalSpy = vi.spyOn(window, 'setInterval')
  renderTree()
  const activeTimersBeforeUnmount = vi.getTimerCount()

  act(() => {
    root?.unmount()
  })
  root = null

  const snapshot = () => ({
    createdIntervals: intervalSpy.mock.calls.length,
    activeTimersBeforeUnmount,
    activeTimersAfterUnmount: vi.getTimerCount(),
  })
  expect(snapshot()).toEqual({
    createdIntervals: 1,
    activeTimersBeforeUnmount: 1,
    activeTimersAfterUnmount: 0,
  })
})

it('shows every tree session with the shared row and keeps ended sessions visible', () => {
  renderTree()

  expect(
    Array.from(container.querySelectorAll('[role="treeitem"]'), (row) => ({
      title: row.querySelector('.session-title')?.textContent,
      role: row.querySelector('.session-role-icon')?.getAttribute('aria-label'),
      engine: row.querySelector('.session-engine')?.textContent ?? null,
      selected: row.getAttribute('aria-selected'),
      task: row.querySelector('.session-task')?.textContent ?? null,
      repo: row.querySelector('.session-repo')?.textContent ?? null,
      vibeMode:
        row.querySelector('.vibe-indicator')?.getAttribute('aria-label') ??
        null,
      ended: row.classList.contains('is-ended'),
    })),
  ).toEqual([
    {
      title: 'Root session',
      role: 'No role',
      engine: 'claude',
      selected: 'true',
      task: '— No task',
      repo: 'sample/repo',
      vibeMode: 'Vibe on',
      ended: false,
    },
    {
      title: 'Active child',
      role: 'development',
      engine: 'codex',
      selected: 'false',
      task: '— #17 Sample task',
      repo: 'sample/other-repo',
      vibeMode: 'Vibe on · inherited',
      ended: false,
    },
    {
      title: 'Ended child',
      role: 'PM',
      engine: 'claude',
      selected: 'false',
      task: '— No task',
      repo: 'sample/other-repo',
      vibeMode: null,
      ended: true,
    },
  ])
})

it.each([
  { size: 'desktop', density: 'density-compact' },
  { size: 'phone', density: 'density-compact-phone' },
] as const)(
  'keeps each tree row on three lines at $size size',
  ({ size, density }) => {
    renderTree(vi.fn(), size)

    expect(
      Array.from(
        container.querySelectorAll<HTMLElement>('[role="treeitem"]'),
        (row) => ({
          density: Array.from(
            row.querySelector('.session-row-content')?.classList ?? [],
          ).find((className) => className.startsWith('density-')),
          detailLines: Array.from(
            row.querySelector('.session-content')?.children ?? [],
            (line) => line.className,
          ),
        }),
      ),
    ).toEqual([
      {
        density,
        detailLines: [
          'session-title-line',
          'session-meta-line',
          'session-repo-line',
        ],
      },
      {
        density,
        detailLines: [
          'session-title-line',
          'session-meta-line',
          'session-repo-line',
        ],
      },
      {
        density,
        detailLines: [
          'session-title-line',
          'session-meta-line',
          'session-repo-line',
        ],
      },
    ])
  },
)

it.each([
  { size: 'desktop', expectedMarginTop: '5px' },
  { size: 'phone', expectedMarginTop: '10px' },
] as const)(
  'aligns the tree caret with the title row at $size size',
  ({ size, expectedMarginTop }) => {
    renderTree(vi.fn(), size)

    const row = container.querySelector('.session-tree-main-line')
    const caret = container.querySelector('.session-tree-caret')
    const snapshot = () => ({
      rowAlignment: row === null ? null : getComputedStyle(row).alignItems,
      caretMarginTop: caret === null ? null : getComputedStyle(caret).marginTop,
    })
    expect(snapshot()).toEqual({
      rowAlignment: 'flow-start',
      caretMarginTop: expectedMarginTop,
    })
  },
)

it('opens an active row with Enter and does not open an ended row by click', () => {
  const onSelectSession = renderTree()
  const tree = container.querySelector<HTMLDivElement>('[role="tree"]')
  const endedRow = container.querySelector<HTMLElement>(
    '#session-tree-row-ended-child',
  )

  act(() => {
    tree?.focus()
    tree?.dispatchEvent(
      new KeyboardEvent('keydown', { key: 'j', bubbles: true }),
    )
  })
  act(() => {
    tree?.dispatchEvent(
      new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }),
    )
  })
  act(() => endedRow?.click())

  expect(onSelectSession.mock.calls).toEqual([['active-child']])
})
