import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'

import { SessionTaskLinks } from '#components/session-task-links'

let container: HTMLDivElement
let root: Root | null = null

function makeRect({
  left,
  top,
  width,
  height,
}: {
  left: number
  top: number
  width: number
  height: number
}): DOMRect {
  return {
    x: left,
    y: top,
    left,
    top,
    right: left + width,
    bottom: top + height,
    width,
    height,
    toJSON: () => ({}),
  }
}

function snapshot() {
  const menu = document.body.querySelector<HTMLElement>(
    '.session-task-links-menu',
  )

  return {
    expanded: container.querySelector('details')?.open,
    primary: {
      href: container
        .querySelector('.session-task-line > a')
        ?.getAttribute('href'),
      id: container.querySelector(
        '.session-task-line > a .session-task-link-id',
      )?.textContent,
      text: container.querySelector('.session-task-line > a')?.textContent,
    },
    count: container.querySelector('.session-task-links summary')?.textContent,
    menuRole: menu?.getAttribute('role') ?? null,
    menuPosition: menu
      ? {
          left: menu.style.left,
          top: menu.style.top,
          width: menu.style.width,
        }
      : null,
    heading: menu?.querySelector('.session-task-links-heading')?.textContent,
    linkedTasks: Array.from(menu?.querySelectorAll('li') ?? []).map((item) => ({
      href: item.querySelector('a')?.getAttribute('href'),
      id: item.querySelector('.session-task-link-id')?.textContent,
      text: item.querySelector('a')?.textContent,
      statuses: Array.from(
        item.querySelectorAll(
          '.session-task-links-primary, .session-task-completed',
        ),
      ).map((status) => status.textContent),
    })),
  }
}

beforeEach(() => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  container = document.createElement('div')
  container.className = 'conversation-pane'
  vi.spyOn(container, 'getBoundingClientRect').mockReturnValue(
    makeRect({ left: 30, top: 0, width: 280, height: 140 }),
  )
  document.body.append(container)
})

afterEach(() => {
  if (root !== null) act(() => root?.unmount())
  root = null
  container.remove()
  vi.unstubAllGlobals()
})

it('lists all linked tasks and marks completed tasks', async () => {
  const task = {
    id: 'open-task',
    number: 2048,
    title: 'Improve request reliability',
    priority: 1,
    isClosed: true,
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
  const summary = container.querySelector('summary')
  if (summary !== null) {
    vi.spyOn(summary, 'getBoundingClientRect').mockReturnValue(
      makeRect({ left: 250, top: 52, width: 48, height: 22 }),
    )
  }
  await act(async () => {
    summary?.click()
    await new Promise<void>((resolve) => window.setTimeout(resolve, 0))
  })

  expect(snapshot()).toEqual({
    expanded: true,
    primary: {
      href: 'https://tq.fohte.net/tasks/open-task',
      id: 'P1 #2048',
      text: 'P1 #2048 Improve request reliability',
    },
    count: '+2 tasks ▾',
    menuRole: null,
    menuPosition: {
      left: '30px',
      top: '82px',
      width: '280px',
    },
    heading: 'Linked tq tasks · Inbox and Board use the primary one',
    linkedTasks: [
      {
        href: 'https://tq.fohte.net/tasks/open-task',
        id: 'P1 #2048',
        text: 'P1 #2048 Improve request reliability',
        statuses: ['Primary', 'Completed'],
      },
      {
        href: 'https://tq.fohte.net/tasks/secondary-task',
        id: 'P4 #2046',
        text: 'P4 #2046 Add request tracing',
        statuses: [],
      },
      {
        href: 'https://tq.fohte.net/tasks/completed-task',
        id: '#2047',
        text: '#2047 Document retry behavior',
        statuses: ['Completed'],
      },
    ],
  })
})
