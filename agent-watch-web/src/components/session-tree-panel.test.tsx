import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'

import { SessionTreePanel } from '#components/session-tree-panel'
import type { SessionTreeNode } from '#types'

const sessions: SessionTreeNode[] = [
  {
    id: 'root',
    parentId: null,
    category: 'done',
    isActive: true,
    kind: null,
    title: 'Root session',
    preview: null,
    elapsedSeconds: 300,
    task: null,
    repo: 'sample/repo',
    decisionCount: 0,
    pullRequest: null,
  },
  {
    id: 'active-child',
    parentId: 'root',
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
    decisionCount: 0,
    pullRequest: null,
  },
  {
    id: 'ended-child',
    parentId: 'active-child',
    category: 'done',
    isActive: false,
    kind: null,
    title: 'Ended child',
    preview: null,
    elapsedSeconds: 3_600,
    task: null,
    repo: 'sample/other-repo',
    decisionCount: 0,
    pullRequest: null,
  },
]

let container: HTMLDivElement
let root: Root | null = null

function renderTree(onSelectSession = vi.fn()) {
  root = createRoot(container)
  act(() => {
    root?.render(
      <SessionTreePanel
        sessions={sessions}
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
  vi.unstubAllGlobals()
})

it('shows the current path, task and repository differences, and ended sessions', () => {
  renderTree()

  expect(
    Array.from(container.querySelectorAll('[role="treeitem"]'), (row) => ({
      title: row.querySelector('.session-tree-title')?.textContent,
      selected: row.getAttribute('aria-selected'),
      task: row.querySelector('.session-tree-task')?.textContent ?? null,
      repo: row.querySelector('.session-tree-repo')?.textContent ?? null,
      ended: row.classList.contains('is-ended'),
    })),
  ).toEqual([
    {
      title: 'Root session',
      selected: 'true',
      task: null,
      repo: 'sample/repo',
      ended: false,
    },
    {
      title: 'Active child',
      selected: 'false',
      task: '17 · Sample task',
      repo: 'sample/other-repo',
      ended: false,
    },
    {
      title: 'Ended child',
      selected: 'false',
      task: null,
      repo: null,
      ended: true,
    },
  ])
})

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
