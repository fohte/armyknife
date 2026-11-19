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
    summaryLabel: summary?.getAttribute('aria-label'),
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

it('keeps closed secondary tasks out of the header menu', async () => {
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
      summaryLabel: '1 more linked tq tasks',
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
      expanded: false,
      primary: {
        href: 'https://tq.fohte.net/tasks/open-task',
        id: 'P1 #2048',
        text: 'P1 #2048 Improve request reliability',
      },
      count: '+1 tasks ▾',
      summaryLabel: '1 more linked tq tasks',
      disclosureOwnsMenu: true,
      summaryControlsMenu: true,
      menuRole: null,
      menuLabel: null,
      menuHidden: true,
      menuPosition: null,
      heading: undefined,
      linkedTasks: [],
    },
  ])
})

it('shows a completed primary without a menu when every secondary task is closed', () => {
  const task = {
    id: 'primary-task',
    number: 2048,
    title: 'Improve request reliability',
    priority: 1,
    isClosed: true,
  }
  const completedTasks = [
    {
      id: 'completed-task-one',
      number: 2047,
      title: 'Document retry behavior',
      priority: null,
      isClosed: true,
    },
    {
      id: 'completed-task-two',
      number: 2046,
      title: 'Review request metrics',
      priority: 3,
      isClosed: true,
    },
    {
      id: 'completed-task-three',
      number: 2045,
      title: 'Update retry guide',
      priority: 4,
      isClosed: true,
    },
  ]
  root = createRoot(container)
  act(() => {
    root?.render(
      <SessionTaskLinks
        task={task}
        tasks={[task, ...completedTasks]}
        taskListOpen
      />,
    )
  })

  const snapshot = () => ({
    primary: container.querySelector('.session-task-line > a')?.textContent,
    disclosure: container.querySelector('.session-task-links summary')
      ?.textContent,
    menuItems: document.body.querySelectorAll('.session-task-links-menu li')
      .length,
  })

  expect(snapshot()).toEqual({
    primary: 'P1 #2048 Improve request reliability',
    disclosure: undefined,
    menuItems: 0,
  })
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
      />,
    )
  })

  await act(async () => {
    container.querySelector('summary')?.click()
    await new Promise<void>((resolve) => window.setTimeout(resolve, 0))
  })

  const makePrimaryButton = document.body.querySelector<HTMLButtonElement>(
    '.session-task-links-make-primary',
  )
  if (makePrimaryButton === null)
    throw new Error('Expected make-primary action')
  await act(async () => {
    makePrimaryButton.dispatchEvent(
      new PointerEvent('pointerdown', { bubbles: true }),
    )
    makePrimaryButton.click()
    await new Promise<void>((resolve) => window.setTimeout(resolve, 0))
  })

  const snapshot = () => ({
    calls: onMakePrimary.mock.calls,
    menuOpen:
      document.body.querySelector<HTMLElement>('.session-task-links-menu')
        ?.hidden === false,
  })
  expect(snapshot()).toEqual({
    calls: [['secondary-task']],
    menuOpen: true,
  })
})

it.each([
  { description: 'an outside pointer press', key: false },
  { description: 'Escape', key: true },
])('closes the linked task menu on $description', async ({ key }) => {
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
  root = createRoot(container)
  act(() => {
    root?.render(<SessionTaskLinks task={task} tasks={[task, secondaryTask]} />)
  })
  const summary = container.querySelector('summary')
  await act(async () => {
    summary?.click()
    await new Promise<void>((resolve) => window.setTimeout(resolve, 0))
  })

  act(() => {
    if (key) {
      document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }))
    } else {
      document.body.dispatchEvent(
        new PointerEvent('pointerdown', { bubbles: true }),
      )
    }
  })
  await act(async () => {
    await new Promise<void>((resolve) => window.setTimeout(resolve, 0))
  })

  const snapshot = () => ({
    expanded: container.querySelector('details')?.open,
    menuHidden: document.body.querySelector<HTMLElement>(
      '.session-task-links-menu',
    )?.hidden,
  })
  expect(snapshot()).toEqual({ expanded: false, menuHidden: true })
})
