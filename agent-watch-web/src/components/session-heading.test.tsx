import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'

import { SessionHeading } from '#components/session-heading'

let container: HTMLDivElement
let root: Root | null = null

function snapshot() {
  return {
    primary: {
      href: container
        .querySelector('.session-heading-task-line > a')
        ?.getAttribute('href'),
      text: container.querySelector('.session-heading-task-line > a')
        ?.textContent,
    },
    count: container.querySelector('.session-task-links summary')?.textContent,
    linkedTasks: Array.from(
      container.querySelectorAll('.session-task-links li'),
    ).map((item) => ({
      href: item.querySelector('a')?.getAttribute('href'),
      text: item.querySelector('a')?.textContent,
      status:
        item.querySelector('.session-task-completed')?.textContent ?? null,
    })),
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

it('lists all linked tasks and marks completed tasks', () => {
  const task = {
    id: 'open-task',
    number: 2048,
    title: 'Improve request reliability',
    priority: 1,
    isClosed: false,
  }
  root = createRoot(container)
  act(() => {
    root?.render(
      <SessionHeading
        title="Investigate a failing build"
        isPaused={false}
        task={task}
        tasks={[
          task,
          {
            id: 'completed-task',
            number: 2047,
            title: 'Document retry behavior',
            priority: null,
            isClosed: true,
          },
        ]}
        taskListOpen
      />,
    )
  })

  expect(snapshot()).toEqual({
    primary: {
      href: 'https://tq.fohte.net/tasks/open-task',
      text: 'P1 #2048 Improve request reliability',
    },
    count: '+1',
    linkedTasks: [
      {
        href: 'https://tq.fohte.net/tasks/open-task',
        text: 'P1 #2048 Improve request reliability',
        status: null,
      },
      {
        href: 'https://tq.fohte.net/tasks/completed-task',
        text: '#2047 Document retry behavior',
        status: 'Completed',
      },
    ],
  })
})
