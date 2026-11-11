import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'

import { SessionTaskLinks } from '#components/session-task-links'

let container: HTMLDivElement
let root: Root | null = null

function snapshot() {
  return {
    expanded: container.querySelector('details')?.open,
    primary: {
      href: container
        .querySelector('.session-task-line > a')
        ?.getAttribute('href'),
      text: container.querySelector('.session-task-line > a')?.textContent,
    },
    count: container.querySelector('.session-task-links summary')?.textContent,
    heading: container.querySelector('.session-task-links-heading')
      ?.textContent,
    linkedTasks: Array.from(
      container.querySelectorAll('.session-task-links li'),
    ).map((item) => ({
      href: item.querySelector('a')?.getAttribute('href'),
      text: item.querySelector('a')?.textContent,
      status:
        item.querySelector(
          '.session-task-completed, .session-task-links-primary',
        )?.textContent ?? null,
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
      <SessionTaskLinks
        task={task}
        tasks={[
          task,
          {
            id: 'secondary-task',
            number: 2046,
            title: 'Add request tracing',
            priority: 4,
            isClosed: false,
          },
          {
            id: 'completed-task',
            number: 2047,
            title: 'Document retry behavior',
            priority: null,
            isClosed: true,
          },
        ]}
      />,
    )
  })
  act(() => {
    container.querySelector('summary')?.click()
  })

  expect(snapshot()).toEqual({
    expanded: true,
    primary: {
      href: 'https://tq.fohte.net/tasks/open-task',
      text: 'P1 #2048 Improve request reliability',
    },
    count: '+2 tasks ▾',
    heading: 'Linked tq tasks · Inbox and Board use the primary one',
    linkedTasks: [
      {
        href: 'https://tq.fohte.net/tasks/open-task',
        text: 'P1 #2048 Improve request reliability',
        status: 'Primary',
      },
      {
        href: 'https://tq.fohte.net/tasks/secondary-task',
        text: 'P4 #2046 Add request tracing',
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
