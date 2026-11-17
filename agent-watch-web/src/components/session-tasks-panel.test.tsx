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
    updatedAt: new Date(Date.now() - (index + 1) * 3_600_000).toISOString(),
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
  vi.restoreAllMocks()
})

it('groups open and completed linked tasks and reveals the remaining completed tasks', () => {
  act(() => {
    root?.render(
      <SessionTasksPanel
        tasks={tasks}
        repo="example/service"
        primaryTaskId="example-open-task"
      />,
    )
  })

  const before = () => ({
    headings: Array.from(
      container.querySelectorAll('.session-tasks-panel-heading'),
      (heading) => heading.textContent,
    ),
    openTasks: Array.from(
      container.querySelectorAll('.session-tasks-panel-item--open'),
      (task) => ({ href: task.getAttribute('href'), text: task.textContent }),
    ),
    completedTasks: Array.from(
      container.querySelectorAll('.session-tasks-panel-item--completed'),
      (task) => ({ href: task.getAttribute('href'), text: task.textContent }),
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
      openTasks: [
        {
          href: 'https://tq.fohte.net/tasks/example-open-task',
          text: '●#2048 Improve request reliabilityP1 · service',
        },
      ],
      completedTasks: [
        {
          href: 'https://tq.fohte.net/tasks/example-completed-task-1',
          text: '✓#2047 Document request behavior 1 1h',
        },
        {
          href: 'https://tq.fohte.net/tasks/example-completed-task-2',
          text: '✓#2046 Document request behavior 2 2h',
        },
        {
          href: 'https://tq.fohte.net/tasks/example-completed-task-3',
          text: '✓#2045 Document request behavior 3 3h',
        },
        {
          href: 'https://tq.fohte.net/tasks/example-completed-task-4',
          text: '✓#2044 Document request behavior 4 4h',
        },
      ],
      showMore: 'Show 1 more',
    },
    afterClick: {
      headings: ['IN PROGRESS', 'COMPLETED 5'],
      openTasks: [
        {
          href: 'https://tq.fohte.net/tasks/example-open-task',
          text: '●#2048 Improve request reliabilityP1 · service',
        },
      ],
      completedTasks: [
        {
          href: 'https://tq.fohte.net/tasks/example-completed-task-1',
          text: '✓#2047 Document request behavior 1 1h',
        },
        {
          href: 'https://tq.fohte.net/tasks/example-completed-task-2',
          text: '✓#2046 Document request behavior 2 2h',
        },
        {
          href: 'https://tq.fohte.net/tasks/example-completed-task-3',
          text: '✓#2045 Document request behavior 3 3h',
        },
        {
          href: 'https://tq.fohte.net/tasks/example-completed-task-4',
          text: '✓#2044 Document request behavior 4 4h',
        },
        {
          href: 'https://tq.fohte.net/tasks/example-completed-task-5',
          text: '✓#2043 Document request behavior 5 5h',
        },
      ],
      showMore: undefined,
    },
  })
})

it('formats completed task ages in minutes and days', () => {
  const now = Date.parse('2026-11-17T00:00:00.000Z')
  vi.spyOn(Date, 'now').mockReturnValue(now)
  const ageTasks: SessionTask[] = [
    {
      id: 'example-minute-task',
      number: 2047,
      title: 'Document request behavior',
      priority: null,
      isClosed: true,
      updatedAt: new Date(now - 35 * 60_000).toISOString(),
    },
    {
      id: 'example-day-task',
      number: 2046,
      title: 'Add a retry example',
      priority: null,
      isClosed: true,
      updatedAt: new Date(now - 2 * 24 * 60 * 60_000).toISOString(),
    },
  ]

  act(() => {
    root?.render(
      <SessionTasksPanel
        tasks={ageTasks}
        repo="example/service"
        primaryTaskId={null}
      />,
    )
  })

  expect(
    Array.from(
      container.querySelectorAll('.session-tasks-panel-item--completed'),
      (task) => task.textContent,
    ),
  ).toEqual([
    '✓#2047 Document request behavior 35m',
    '✓#2046 Add a retry example 2d',
  ])
})

it('makes a completed secondary task primary from the Tasks tab', async () => {
  const onMakePrimary = vi.fn(() => Promise.resolve(true))
  act(() => {
    root?.render(
      <SessionTasksPanel
        tasks={tasks}
        repo="example/service"
        primaryTaskId="example-open-task"
        onMakePrimary={onMakePrimary}
      />,
    )
  })

  await act(async () => {
    container
      .querySelector<HTMLButtonElement>('.session-tasks-panel-primary-action')
      ?.click()
    await new Promise<void>((resolve) => window.setTimeout(resolve, 0))
  })

  expect(onMakePrimary.mock.calls).toEqual([['example-completed-task-1']])
})
