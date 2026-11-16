import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'

import { SessionTasksPanel } from '#components/session-tasks-panel'
import type { SessionTask } from '#types'

const tasks: SessionTask[] = [
  {
    id: 'example-open-task',
    number: 2048,
    title: 'Improve request reliability',
    priority: 1,
  },
  ...Array.from({ length: 5 }, (_, index) => ({
    id: `example-completed-task-${String(index + 1)}`,
    number: 2047 - index,
    title: `Document request behavior ${String(index + 1)}`,
    priority: null,
    isClosed: true,
  })),
]

let container: HTMLDivElement
let root: Root | null = null

beforeEach(() => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  container = document.createElement('div')
  document.body.append(container)
  root = createRoot(container)
})

afterEach(() => {
  if (root !== null) act(() => root?.unmount())
  root = null
  container.remove()
  vi.unstubAllGlobals()
})

it('groups open and completed linked tasks and reveals the remaining completed tasks', () => {
  act(() => {
    root?.render(<SessionTasksPanel tasks={tasks} repo="example/service" />)
  })

  const before = () => ({
    headings: Array.from(
      container.querySelectorAll('.session-tasks-panel-heading'),
      (heading) => heading.textContent,
    ),
    openTasks: Array.from(
      container.querySelectorAll('.session-tasks-panel-item--open'),
      (task) => task.textContent,
    ),
    completedTasks: Array.from(
      container.querySelectorAll('.session-tasks-panel-item--completed'),
      (task) => task.textContent,
    ),
    showMore: container.querySelector('.session-tasks-panel-show-more')
      ?.textContent,
  })
  const beforeClick = before()

  act(() => {
    container
      .querySelector<HTMLButtonElement>('.session-tasks-panel-show-more')
      ?.click()
  })

  const snapshot = () => ({ beforeClick, afterClick: before() })
  expect(snapshot()).toEqual({
    beforeClick: {
      headings: ['IN PROGRESS', 'COMPLETED 5'],
      openTasks: ['●#2048 Improve request reliabilityP1 · example/service'],
      completedTasks: [
        '✓#2047 Document request behavior 1',
        '✓#2046 Document request behavior 2',
        '✓#2045 Document request behavior 3',
        '✓#2044 Document request behavior 4',
      ],
      showMore: 'Show 1 more',
    },
    afterClick: {
      headings: ['IN PROGRESS', 'COMPLETED 5'],
      openTasks: ['●#2048 Improve request reliabilityP1 · example/service'],
      completedTasks: [
        '✓#2047 Document request behavior 1',
        '✓#2046 Document request behavior 2',
        '✓#2045 Document request behavior 3',
        '✓#2044 Document request behavior 4',
        '✓#2043 Document request behavior 5',
      ],
      showMore: undefined,
    },
  })
})
