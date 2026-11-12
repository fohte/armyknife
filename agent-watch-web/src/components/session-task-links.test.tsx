import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'

import { SessionTaskLinks } from '#components/session-task-links'
import type { Session, SessionTask } from '#types'

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
  const summary = container.querySelector('summary')
  const menu = document.body.querySelector<HTMLElement>(
    '.session-task-links-menu',
  )
  const visibleMenu = menu?.hidden === false ? menu : null

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
    disclosureOwnsMenu:
      container
        .querySelector('.session-task-links')
        ?.getAttribute('aria-owns') === menu?.id,
    summaryControlsMenu: summary?.getAttribute('aria-controls') === menu?.id,
    menuRole: visibleMenu?.getAttribute('role') ?? null,
    menuLabel: visibleMenu?.getAttribute('aria-label') ?? null,
    menuHidden: menu?.hidden ?? null,
    menuPosition: visibleMenu
      ? {
          left: visibleMenu.style.left,
          top: visibleMenu.style.top,
          width: visibleMenu.style.width,
        }
      : null,
    heading: visibleMenu?.querySelector('.session-task-links-heading')
      ?.textContent,
    completedToggle: (() => {
      const toggle = visibleMenu?.querySelector<HTMLButtonElement>(
        '.session-task-links-completed-toggle',
      )
      return toggle === null || toggle === undefined
        ? null
        : {
            text: toggle.textContent,
            expanded: toggle.getAttribute('aria-expanded'),
          }
    })(),
    linkedTasks: Array.from(visibleMenu?.querySelectorAll('li') ?? []).map(
      (item) => ({
        href: item.querySelector('a')?.getAttribute('href'),
        id: item.querySelector('.session-task-link-id')?.textContent,
        text: item.querySelector('a')?.textContent,
        statuses: Array.from(
          item.querySelectorAll(
            '.session-task-links-primary, .session-task-completed',
          ),
        ).map((status) => status.textContent),
        summary: item.querySelector('.session-task-links-summary')?.textContent,
        action: item.querySelector('button')?.textContent ?? null,
      }),
    ),
  }
}

function makeSession(
  category: Session['category'],
  tasks: SessionTask[],
): Pick<Session, 'category'> & Pick<Session, 'task' | 'tasks'> {
  return {
    category,
    task: tasks[0] ?? null,
    tasks,
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

it('keeps completed secondary tasks collapsed until requested', async () => {
  const task = {
    id: 'open-task',
    number: 2048,
    title: 'Improve request reliability',
    priority: 1,
    isClosed: true,
  }
  const secondaryTask = {
    id: 'secondary-task',
    number: 2046,
    title: 'Add request tracing',
    priority: 4,
    isClosed: false,
  }
  const completedTask = {
    id: 'completed-task',
    number: 2047,
    title: 'Document retry behavior',
    priority: null,
    isClosed: true,
  }
  root = createRoot(container)
  act(() => {
    root?.render(
      <SessionTaskLinks
        task={task}
        tasks={[task, secondaryTask, completedTask]}
        sessions={[
          makeSession('needs_action', [task]),
          makeSession('needs_action', [task]),
          makeSession('in_progress', [task, secondaryTask]),
          makeSession('done', [task, secondaryTask]),
          makeSession('later', [completedTask]),
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

  const snapshots = [snapshot()]

  await act(async () => {
    document.body
      .querySelector<HTMLButtonElement>('.session-task-links-completed-toggle')
      ?.click()
    await new Promise<void>((resolve) => window.setTimeout(resolve, 0))
  })

  snapshots.push(snapshot())

  await act(async () => {
    summary?.click()
    await new Promise<void>((resolve) => window.setTimeout(resolve, 0))
  })

  snapshots.push(snapshot())

  expect(snapshots).toEqual([
    {
      expanded: true,
      primary: {
        href: 'https://tq.fohte.net/tasks/open-task',
        id: 'P1 #2048',
        text: 'P1 #2048 Improve request reliability',
      },
      count: '+1 tasks ▾',
      disclosureOwnsMenu: true,
      summaryControlsMenu: true,
      menuRole: 'region',
      menuLabel: 'Linked tq tasks',
      menuHidden: false,
      menuPosition: {
        left: '30px',
        top: '82px',
        width: '280px',
      },
      heading: 'Linked tq tasks · Inbox and Board use the primary one',
      completedToggle: { text: 'Show 1 completed', expanded: 'false' },
      linkedTasks: [
        {
          href: 'https://tq.fohte.net/tasks/open-task',
          id: 'P1 #2048',
          text: 'P1 #2048 Improve request reliability',
          statuses: ['Primary', 'Completed'],
          summary: '4 sessions · 2 need action',
          action: null,
        },
        {
          href: 'https://tq.fohte.net/tasks/secondary-task',
          id: 'P4 #2046',
          text: 'P4 #2046 Add request tracing',
          statuses: [],
          summary: '2 sessions',
          action: 'Make primary',
        },
      ],
    },
    {
      expanded: true,
      primary: {
        href: 'https://tq.fohte.net/tasks/open-task',
        id: 'P1 #2048',
        text: 'P1 #2048 Improve request reliability',
      },
      count: '+1 tasks ▾',
      disclosureOwnsMenu: true,
      summaryControlsMenu: true,
      menuRole: 'region',
      menuLabel: 'Linked tq tasks',
      menuHidden: false,
      menuPosition: {
        left: '30px',
        top: '82px',
        width: '280px',
      },
      heading: 'Linked tq tasks · Inbox and Board use the primary one',
      completedToggle: { text: 'Hide 1 completed', expanded: 'true' },
      linkedTasks: [
        {
          href: 'https://tq.fohte.net/tasks/open-task',
          id: 'P1 #2048',
          text: 'P1 #2048 Improve request reliability',
          statuses: ['Primary', 'Completed'],
          summary: '4 sessions · 2 need action',
          action: null,
        },
        {
          href: 'https://tq.fohte.net/tasks/secondary-task',
          id: 'P4 #2046',
          text: 'P4 #2046 Add request tracing',
          statuses: [],
          summary: '2 sessions',
          action: 'Make primary',
        },
        {
          href: 'https://tq.fohte.net/tasks/completed-task',
          id: '#2047',
          text: '#2047 Document retry behavior',
          statuses: ['Completed'],
          summary: '1 session · Later',
          action: 'Make primary',
        },
      ],
    },
    {
      expanded: false,
      primary: {
        href: 'https://tq.fohte.net/tasks/open-task',
        id: 'P1 #2048',
        text: 'P1 #2048 Improve request reliability',
      },
      count: '+1 tasks ▾',
      disclosureOwnsMenu: true,
      summaryControlsMenu: true,
      menuRole: null,
      menuLabel: null,
      menuHidden: true,
      menuPosition: null,
      heading: undefined,
      completedToggle: null,
      linkedTasks: [],
    },
  ])
})

it('keeps a completed primary visible when every secondary task is completed', async () => {
  const task = {
    id: 'primary-task',
    number: 2048,
    title: 'Improve request reliability',
    priority: 1,
    isClosed: true,
  }
  const completedTask = {
    id: 'completed-task',
    number: 2047,
    title: 'Document retry behavior',
    priority: null,
    isClosed: true,
  }
  root = createRoot(container)
  act(() => {
    root?.render(
      <SessionTaskLinks
        task={task}
        tasks={[task, completedTask]}
        taskListOpen
      />,
    )
  })

  const snapshots = [
    {
      count: container.querySelector('summary')?.textContent,
      menuTasks: Array.from(
        document.body.querySelectorAll('.session-task-links-menu li'),
      ).map((item) => item.querySelector('a')?.textContent),
      toggle: document.body.querySelector<HTMLButtonElement>(
        '.session-task-links-completed-toggle',
      )?.textContent,
    },
  ]

  await act(async () => {
    document.body
      .querySelector<HTMLButtonElement>('.session-task-links-completed-toggle')
      ?.click()
    await new Promise<void>((resolve) => window.setTimeout(resolve, 0))
  })

  snapshots.push({
    count: container.querySelector('summary')?.textContent,
    menuTasks: Array.from(
      document.body.querySelectorAll('.session-task-links-menu li'),
    ).map((item) => item.querySelector('a')?.textContent),
    toggle: document.body.querySelector<HTMLButtonElement>(
      '.session-task-links-completed-toggle',
    )?.textContent,
  })

  expect(snapshots).toEqual([
    {
      count: '+0 tasks · 1 completed ▾',
      menuTasks: ['P1 #2048 Improve request reliability'],
      toggle: 'Show 1 completed',
    },
    {
      count: '+0 tasks · 1 completed ▾',
      menuTasks: [
        'P1 #2048 Improve request reliability',
        '#2047 Document retry behavior',
      ],
      toggle: 'Hide 1 completed',
    },
  ])
})

it('makes a linked task primary from the menu', async () => {
  const task = {
    id: 'primary-task',
    number: 2048,
    title: 'Improve request reliability',
    priority: 1,
  }
  const secondaryTask = {
    id: 'secondary-task',
    number: 2046,
    title: 'Add request tracing',
    priority: 4,
  }
  const onMakePrimary = vi.fn(() => Promise.resolve(true))
  root = createRoot(container)
  act(() => {
    root?.render(
      <SessionTaskLinks
        task={task}
        tasks={[task, secondaryTask]}
        onMakePrimary={onMakePrimary}
        taskListOpen
      />,
    )
  })

  await act(async () => {
    document.body
      .querySelector<HTMLButtonElement>('.session-task-links-make-primary')
      ?.click()
    await new Promise<void>((resolve) => window.setTimeout(resolve, 0))
  })

  expect(onMakePrimary.mock.calls).toEqual([['secondary-task']])
})
