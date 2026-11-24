import { act, type ComponentProps } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { userEvent } from 'vitest/browser'

import {
  getCommandPaletteModifierLabel,
  getCommandPaletteShortcut,
} from '#command-palette-shortcut'
import { CommandPalette } from '#components/command-palette'
import type { Session } from '#types'

let container: HTMLDivElement
let root: Root | null = null

const session: Session = {
  id: 'session-alerts',
  category: 'needs_action',
  isPaused: false,
  kind: 'Question',
  title: 'Review notification settings',
  preview: null,
  elapsedSeconds: 300,
  unread: true,
  task: {
    id: 'task-alerts',
    number: 42,
    title: 'Improve notifications',
    priority: 1,
  },
  repo: 'example/web-app',
  branch: 'feature/sample-change',
  canOpenInTerminal: true,
  project: 'notification refresh',
  parent: null,
  ancestorSessionIds: [],
  pullRequest: null,
}

function renderPalette(
  props: Partial<ComponentProps<typeof CommandPalette>> = {},
) {
  root = createRoot(container)
  act(() => {
    root?.render(
      <CommandPalette
        sessions={[session]}
        recentSessionIds={[]}
        onClose={() => undefined}
        onCloseSession={() => undefined}
        onOpenSession={() => undefined}
        onOpenSessionBeside={() => undefined}
        onLater={() => undefined}
        onOpenInTerminal={() => undefined}
        onOpenProject={() => undefined}
        onRunCommand={() => undefined}
        {...props}
      />,
    )
  })
}

async function pressKeys(...keys: string[]) {
  for (const key of keys) {
    await act(async () => {
      await userEvent.keyboard(key)
    })
  }
}

async function openActionsMenu() {
  await pressKeys('{ArrowRight}')
  await vi.waitFor(() => {
    if (container.querySelector('[role="menu"]') === null) {
      throw new Error('Actions menu was not rendered')
    }
  })
}

async function hoverAt(x: number, y: number) {
  const target = document.createElement('div')
  target.style.position = 'fixed'
  target.style.left = `${String(x)}px`
  target.style.top = `${String(y)}px`
  target.style.width = '2px'
  target.style.height = '2px'
  target.style.zIndex = '1000'
  document.body.append(target)
  await act(async () => {
    await userEvent.hover(target)
  })
  target.remove()
}

const openBesideModifier = getCommandPaletteModifierLabel(
  getCommandPaletteShortcut(navigator.platform),
)

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

it('opens the project for the active result with a prefix and Enter', async () => {
  const onOpenProject = vi.fn<(name: string) => void>()
  renderPalette({ onOpenProject })
  const input = container.querySelector<HTMLInputElement>(
    '[aria-label="Jump to a session, tq task or project, or type > for commands"]',
  )
  if (input === null) throw new Error('Palette input was not rendered')

  await act(async () => {
    await userEvent.type(input, '#notification')
    await userEvent.keyboard('{Enter}')
  })

  expect(onOpenProject.mock.calls).toEqual([['notification refresh']])
})

it.each([
  ['macOS', 'MacIntel', { metaKey: true }],
  ['other platforms', 'Linux x86_64', { ctrlKey: true }],
] as const)(
  'opens the selected session beside with Enter on %s',
  (_name, platform, modifiers) => {
    vi.spyOn(navigator, 'platform', 'get').mockReturnValue(platform)
    const onOpenSession = vi.fn<(id: string) => void>()
    const onOpenSessionBeside = vi.fn<(id: string) => void>()
    renderPalette({ onOpenSession, onOpenSessionBeside })
    const input = container.querySelector<HTMLInputElement>(
      '[aria-label="Jump to a session, tq task or project, or type > for commands"]',
    )
    if (input === null) throw new Error('Palette input was not rendered')
    const event = new KeyboardEvent('keydown', {
      key: 'Enter',
      bubbles: true,
      cancelable: true,
      ...modifiers,
    })

    act(() => {
      input.dispatchEvent(event)
    })

    const getActual = () => ({
      defaultPrevented: event.defaultPrevented,
      besideCalls: onOpenSessionBeside.mock.calls,
      openCalls: onOpenSession.mock.calls,
    })
    expect(getActual()).toEqual({
      defaultPrevented: true,
      besideCalls: [['session-alerts']],
      openCalls: [],
    })
  },
)

it('shows the primary task and the count of other linked tasks for a session', () => {
  renderPalette({
    sessions: [
      {
        ...session,
        tasks: [
          {
            id: 'task-alerts',
            number: 42,
            title: 'Improve notifications',
            priority: 1,
          },
          {
            id: 'task-delivery',
            number: 43,
            title: 'Document delivery behavior',
            priority: null,
          },
        ],
      },
    ],
  })

  const snapshot = () => ({
    task: container.querySelector('.session-task')?.textContent,
    repo: container.querySelector('.session-repo')?.textContent,
  })
  expect(snapshot()).toEqual({
    task: 'P1 #42 Improve notifications +1',
    repo: 'example/web-app',
  })
})

it.each([
  [
    'needs action',
    'needs_action',
    'Question',
    300,
    'Question',
    '5m',
    true,
    false,
  ],
  [
    'needs action without a kind',
    'needs_action',
    null,
    300,
    'Stopped',
    '5m',
    true,
    false,
  ],
  ['error', 'needs_action', 'Error', 720, 'Error', '12m', true, false],
  ['in progress', 'in_progress', null, 30, 'Working', '30s', false, true],
  ['later', 'later', null, 90, 'Stopped', '1m', true, false],
  ['done', 'done', null, 360, 'Done', '6m', false, false],
  [
    'short session',
    'needs_action',
    'Question',
    8,
    'Question',
    '8s',
    true,
    false,
  ],
  [
    '59 seconds',
    'needs_action',
    'Question',
    59,
    'Question',
    '59s',
    true,
    false,
  ],
  ['60 seconds', 'needs_action', 'Question', 60, 'Question', '1m', true, false],
  [
    '59 minutes',
    'needs_action',
    'Question',
    3_540,
    'Question',
    '59m',
    true,
    false,
  ],
  [
    '60 minutes',
    'needs_action',
    'Question',
    3_600,
    'Question',
    '1h',
    true,
    false,
  ],
  [
    '90 minutes',
    'needs_action',
    'Question',
    5_400,
    'Question',
    '1h',
    true,
    false,
  ],
  [
    '24 hours',
    'needs_action',
    'Question',
    86_400,
    'Question',
    '1d',
    true,
    false,
  ],
  ['unknown category', 'blocked', null, 60, 'Stopped', '1m', true, false],
] as const)(
  'renders the %s status icon and elapsed time',
  (
    _name,
    category,
    kind,
    elapsedSeconds,
    expectedStatus,
    expectedAge,
    hasKindIcon,
    hasWorkingDot,
  ) => {
    renderPalette({
      sessions: [{ ...session, category, kind, elapsedSeconds }],
      initialQuery: 'Review notification settings',
    })

    const item = container.querySelector('.command-palette-item')
    const snapshot = () => ({
      status: item
        ?.querySelector('.session-row-status')
        ?.getAttribute('aria-label'),
      hasKindIcon: item?.querySelector('.session-row-kind-icon') !== null,
      hasWorkingDot: item?.querySelector('.session-row-working-dot') !== null,
      age: item?.querySelector('.session-age')?.textContent,
      title: item?.querySelector('.session-title')?.textContent,
    })
    expect(snapshot()).toEqual({
      status: expectedStatus,
      hasKindIcon,
      hasWorkingDot,
      age: expectedAge,
      title: 'Review notification settings',
    })
  },
)

it.each([
  ['needs action', 'needs_action', 'Question', 'needs-action'],
  ['in progress', 'in_progress', null, 'in-progress'],
  ['stopped in progress', 'in_progress', 'Stopped', 'in-progress'],
  ['Error overrides its category', 'in_progress', 'Error', 'error'],
] as const)(
  'uses the shared %s session tone in the command palette',
  (_label, category, kind, tone) => {
    renderPalette({
      sessions: [{ ...session, category, kind }],
      initialQuery: 'Review notification settings',
    })

    expect(
      container
        .querySelector('.command-palette-item')
        ?.getAttribute('data-tone'),
    ).toEqual(tone)
  },
)

it.each([
  ['task', '#notification', '#', '#42 Improve notifications'],
  ['project', '@notification', '▤', 'notification refresh'],
  ['command', '>Switch to Board', '›', 'Switch to Board'],
] as const)(
  'renders a %s result with its marker and title',
  (_type, query, icon, title) => {
    renderPalette({ initialQuery: query })

    const item = container.querySelector('.command-palette-item')
    const actualIcon =
      item?.querySelector('.command-palette-status')?.textContent ?? ''
    const actualTitle =
      item?.querySelector('.command-palette-item-heading strong')
        ?.textContent ?? ''
    const actual = `${actualIcon}${actualTitle}`
    expect(actual).toEqual(`${icon}${title}`)
  },
)

it('adds the task number to result rows but not action headings', async () => {
  renderPalette({ initialQuery: '#notification' })

  expect(
    container.querySelector('.command-palette-item-heading strong')
      ?.textContent,
  ).toBe('#42 Improve notifications')

  await act(async () => {
    await userEvent.keyboard('{ArrowRight}')
  })

  const snapshot = () => ({
    actionHeading: container.querySelector('[role="menu"] h2')?.textContent,
    actionLabel: container
      .querySelector('[role="menu"]')
      ?.getAttribute('aria-label'),
  })
  expect(snapshot()).toEqual({
    actionHeading: 'Improve notifications',
    actionLabel: 'Improve notifications actions',
  })
})

it('shows task priority before the task result summary', () => {
  renderPalette({ initialQuery: '#notification' })

  expect(
    container.querySelector('.command-palette-item-subtitle')?.textContent,
  ).toBe('P1 · 1 session')
})

it('shows an em dash when a task has no priority', () => {
  renderPalette({
    initialQuery: '#57',
    sessions: [
      {
        ...session,
        task: {
          id: 'task-rollout',
          number: 57,
          title: 'Prepare a rollout plan',
          priority: null,
        },
      },
    ],
  })

  expect(
    container.querySelector('.command-palette-item-subtitle')?.textContent,
  ).toBe('P— · 1 session')
})

it('shows the three designed commands without subtitles or trailing details', () => {
  renderPalette()

  const getActual = () => ({
    commands: Array.from(
      container.querySelectorAll('.command-palette-item[data-tone="command"]'),
    ).map((item) => ({
      icon: item.querySelector('.command-palette-status')?.textContent,
      title: item.querySelector('.command-palette-item-heading strong')
        ?.textContent,
      subtitle: item.querySelector('.command-palette-item-subtitle')
        ?.textContent,
      detail: item.querySelector('.command-palette-item-detail')?.textContent,
    })),
    scopes: Array.from(container.querySelectorAll('[role="tab"]')).map(
      (tab) => tab.textContent,
    ),
  })
  expect(getActual()).toEqual({
    commands: [
      {
        icon: '+',
        title: 'New session',
        subtitle: undefined,
        detail: undefined,
      },
      {
        icon: '›',
        title: 'Switch to Board',
        subtitle: undefined,
        detail: undefined,
      },
      {
        icon: '›',
        title: 'Close all done',
        subtitle: undefined,
        detail: undefined,
      },
    ],
    scopes: ['All', 'Sessions', 'tq tasks', 'Projects', 'Commands'],
  })
})

it('keeps Tab keyboard scope switching without showing scope markers', async () => {
  renderPalette()

  await act(async () => {
    await userEvent.keyboard('{Tab}{Tab}')
  })

  expect(
    Array.from(container.querySelectorAll('[role="tab"]')).map((tab) => ({
      label: tab.textContent,
      selected: tab.getAttribute('aria-selected'),
    })),
  ).toEqual([
    { label: 'All', selected: 'false' },
    { label: 'Sessions', selected: 'false' },
    { label: 'tq tasks', selected: 'true' },
    { label: 'Projects', selected: 'false' },
    { label: 'Commands', selected: 'false' },
  ])
})

it('does not show Show in Board on task result rows', () => {
  renderPalette({ initialQuery: '#notification' })

  const getActual = () => ({
    subtitle: container.querySelector('.command-palette-item-subtitle')
      ?.textContent,
    detail: container.querySelector('.command-palette-item-detail')
      ?.textContent,
  })
  expect(getActual()).toEqual({ subtitle: 'P1 · 1 session', detail: undefined })
})

it('keeps Show in Board on project result rows', () => {
  renderPalette({ initialQuery: '@notification' })

  const getActual = () =>
    Array.from(container.querySelectorAll('.command-palette-item')).map(
      (item) => ({
        icon: item.querySelector('.command-palette-status')?.textContent,
        title: item.querySelector('.command-palette-item-heading strong')
          ?.textContent,
        subtitle: item.querySelector('.command-palette-item-subtitle')
          ?.textContent,
        detail: item.querySelector('.command-palette-item-detail')?.textContent,
      }),
    )
  expect(getActual()).toEqual([
    {
      icon: '▤',
      title: 'notification refresh',
      subtitle: '1 sessions · 1 need action · example/web-app',
      detail: 'Show in Board',
    },
  ])
})

it.each([
  [
    'results',
    undefined,
    [
      '↵ Open',
      `${openBesideModifier}↵ Open beside`,
      '→ Actions',
      'Tab Scope',
      '# task · @ project · > command',
    ],
    'command-palette-hint-legend',
  ],
  ['actions', 'session-alerts', ['↵ Select', '← Back', '↑↓ Navigate'], ''],
] as const)(
  'renders %s footer hints with the right alignment only on the legend',
  (_name, initialActionItemId, expectedHints, expectedLegendClass) => {
    renderPalette(
      initialActionItemId === undefined ? {} : { initialActionItemId },
    )

    const snapshot = () => {
      const hints = container.querySelectorAll('.command-palette-hints > span')
      return {
        labels: Array.from(hints).map((hint) => hint.textContent),
        lastClassName: hints[hints.length - 1]?.className ?? '',
      }
    }
    expect(snapshot()).toEqual({
      labels: expectedHints,
      lastClassName: expectedLegendClass,
    })
  },
)

it('opens a matching project from the project scope with Enter', async () => {
  const onOpenProject = vi.fn<(name: string) => void>()
  renderPalette({ onOpenProject })
  const input = container.querySelector<HTMLInputElement>(
    '[aria-label="Jump to a session, tq task or project, or type > for commands"]',
  )
  if (input === null) throw new Error('Palette input was not rendered')

  await act(async () => {
    await userEvent.type(input, '@notification')
    await userEvent.keyboard('{Enter}')
  })

  expect(onOpenProject.mock.calls).toEqual([['notification refresh']])
})

it('runs the selected command with Ctrl-n and Enter', async () => {
  const onRunCommand =
    vi.fn<NonNullable<ComponentProps<typeof CommandPalette>['onRunCommand']>>()
  renderPalette({ initialQuery: '>Switch to', onRunCommand })
  const input = container.querySelector<HTMLInputElement>(
    '[aria-label="Jump to a session, tq task or project, or type > for commands"]',
  )
  if (input === null) throw new Error('Palette input was not rendered')

  await act(async () => {
    await userEvent.keyboard('{Control>}n{/Control}')
    await userEvent.keyboard('{Enter}')
  })

  expect(onRunCommand.mock.calls).toEqual([
    [
      {
        type: 'command',
        id: 'switch-board',
        label: 'Switch to Board',
        description: 'Show projects and sessions',
      },
    ],
  ])
})

it('runs New session from the command palette', async () => {
  const onRunCommand =
    vi.fn<NonNullable<ComponentProps<typeof CommandPalette>['onRunCommand']>>()
  renderPalette({ initialQuery: '>New session', onRunCommand })

  await act(async () => {
    await userEvent.keyboard('{Enter}')
  })

  expect(onRunCommand.mock.calls).toEqual([
    [
      {
        type: 'command',
        id: 'new-session',
        label: 'New session',
        description: 'Start Claude Code or Codex in a directory',
      },
    ],
  ])
})

it('matches a session by branch name and renders the shared session row', () => {
  renderPalette({ initialQuery: 'feature/sample-change' })

  const snapshot = () => ({
    title: container.querySelector('.session-title')?.textContent,
    task: container.querySelector('.session-task')?.textContent,
    repo: container.querySelector('.session-repo')?.textContent,
    branch: container.querySelector('.session-branch')?.textContent,
  })
  expect(snapshot()).toEqual({
    title: 'Review notification settings',
    task: 'P1 #42 Improve notifications',
    repo: 'example/web-app',
    branch: 'feature/sample-change',
  })
})

it.each([
  ['omitted', undefined],
  ['null', null],
  ['blank', '   '],
] as const)(
  'keeps branch metadata out of the common row when it is %s',
  (_, branch) => {
    const sessionWithoutBranch: Session = { ...session }
    if (branch === undefined) {
      delete sessionWithoutBranch.branch
    } else {
      sessionWithoutBranch.branch = branch
    }
    renderPalette({ sessions: [sessionWithoutBranch] })

    const snapshot = () => ({
      task: container.querySelector('.session-task')?.textContent,
      repo: container.querySelector('.session-repo')?.textContent,
      branch: container.querySelector('.session-branch')?.textContent ?? null,
    })
    expect(snapshot()).toEqual({
      task: 'P1 #42 Improve notifications',
      repo: 'example/web-app',
      branch: null,
    })
  },
)

it('keeps the branch after the task when the repository is blank', () => {
  renderPalette({
    sessions: [
      {
        ...session,
        repo: '   ',
        branch: 'feature/sample-change',
      },
    ],
  })

  const snapshot = () => ({
    task: container.querySelector('.session-task')?.textContent,
    repo: container.querySelector('.session-repo')?.textContent,
    branch: container.querySelector('.session-branch')?.textContent,
  })
  expect(snapshot()).toEqual({
    task: 'P1 #42 Improve notifications',
    repo: '—',
    branch: 'feature/sample-change',
  })
})

it.each([
  ['caret before the query end', 2, 2, 3],
  ['a selection before the query end', 2, 6, 6],
] as const)(
  'keeps ArrowRight in the input for %s',
  async (_, start, end, expectedCaret) => {
    renderPalette({ initialQuery: 'notification' })
    const input = container.querySelector<HTMLInputElement>(
      '[aria-label="Jump to a session, tq task or project, or type > for commands"]',
    )
    if (input === null) throw new Error('Palette input was not rendered')
    input.setSelectionRange(start, end)

    await act(async () => {
      await userEvent.keyboard('{ArrowRight}')
    })

    const snapshot = () => ({
      actionMenu: container.querySelector('[role="menu"]') !== null,
      selectionEnd: input.selectionEnd,
      selectionStart: input.selectionStart,
    })

    expect(snapshot()).toEqual({
      actionMenu: false,
      selectionEnd: expectedCaret,
      selectionStart: expectedCaret,
    })
  },
)

it('opens the highlighted actions when the query caret is at the end', async () => {
  renderPalette({ initialQuery: 'Review' })
  const input = container.querySelector<HTMLInputElement>(
    '[aria-label="Jump to a session, tq task or project, or type > for commands"]',
  )
  if (input === null) throw new Error('Palette input was not rendered')
  input.setSelectionRange(input.value.length, input.value.length)

  await act(async () => {
    await userEvent.keyboard('{ArrowRight}')
  })

  expect(
    Array.from(container.querySelectorAll('[role="menuitem"]')).map(
      (item) => item.textContent,
    ),
  ).toEqual([
    'Open session',
    'Open beside',
    'Later',
    'Copy session ID',
    'Open in terminal',
    'Close this session',
  ])
})

it('opens the highlighted session actions with ArrowRight and runs Later', async () => {
  const onLater = vi.fn<(id: string) => void>()
  renderPalette({ onLater })

  await openActionsMenu()
  await pressKeys('{ArrowDown}', '{ArrowDown}', '{Enter}')

  expect(onLater.mock.calls).toEqual([['session-alerts']])
})

it('requests closing the highlighted session from its actions submenu', async () => {
  const onCloseSession = vi.fn<(id: string) => void>()
  renderPalette({ onCloseSession })

  await openActionsMenu()
  await pressKeys(
    '{ArrowDown}',
    '{ArrowDown}',
    '{ArrowDown}',
    '{ArrowDown}',
    '{ArrowDown}',
    '{Enter}',
  )

  expect(onCloseSession.mock.calls).toEqual([['session-alerts']])
})

it('opens the highlighted session beside from the actions submenu', async () => {
  const onOpenSessionBeside = vi.fn<(id: string) => void>()
  renderPalette({ onOpenSessionBeside })

  await openActionsMenu()
  await pressKeys('{ArrowDown}', '{Enter}')

  expect(onOpenSessionBeside.mock.calls).toEqual([['session-alerts']])
})

it('opens the highlighted session in its terminal from the actions submenu', async () => {
  const onOpenInTerminal = vi.fn<(id: string) => void>()
  renderPalette({ onOpenInTerminal, initialActionItemId: session.id })
  const later = container.querySelectorAll('[role="menuitem"]')[2]
  if (later === undefined) throw new Error('Later action was not rendered')
  const { x, y, width, height } = later.getBoundingClientRect()

  await pressKeys('{ArrowLeft}')
  await hoverAt(x + width / 2, y + height / 2)
  await pressKeys('{ArrowRight}')
  await vi.waitFor(() => {
    if (container.querySelector('[role="menu"]') === null) {
      throw new Error('Actions menu was not rendered')
    }
  })
  await pressKeys(
    '{ArrowDown}',
    '{ArrowDown}',
    '{ArrowDown}',
    '{ArrowDown}',
    '{Enter}',
  )

  const snapshot = () => ({
    calls: onOpenInTerminal.mock.calls,
    submenuVisible: container.querySelector('[role="menu"]') !== null,
  })
  expect(snapshot()).toEqual({
    calls: [['session-alerts']],
    submenuVisible: true,
  })
})

it('highlights an action when the pointer moves over it', async () => {
  renderPalette({ initialActionItemId: session.id })
  const later = container.querySelectorAll('[role="menuitem"]')[2]
  if (later === undefined) throw new Error('Later action was not rendered')

  await act(async () => {
    await userEvent.hover(later)
  })

  expect(
    Array.from(container.querySelectorAll('[role="menuitem"]')).map((item) =>
      item
        .closest('.command-palette-action')
        ?.getAttribute('data-palette-action-active'),
    ),
  ).toEqual(['false', 'false', 'true', 'false', 'false', 'false'])
})

it('keeps the keyboard-selected result when results change under a stationary pointer', async () => {
  const sessions = [
    {
      ...session,
      id: 'session-first',
      title: 'Review first notification',
      task: {
        id: 'task-first',
        number: 51,
        title: 'First notification task',
        priority: 1,
      },
    },
    {
      ...session,
      id: 'session-second',
      title: 'Review second notification',
      task: {
        id: 'task-second',
        number: 52,
        title: 'Second notification task',
        priority: 2,
      },
    },
    {
      ...session,
      id: 'session-third',
      title: 'Review third notification',
      task: {
        id: 'task-third',
        number: 53,
        title: 'Third notification task',
        priority: 3,
      },
    },
  ] satisfies Session[]
  renderPalette({ sessions, initialQuery: 'Review' })
  const input = container.querySelector<HTMLInputElement>(
    '[aria-label="Jump to a session, tq task or project, or type > for commands"]',
  )
  if (input === null) throw new Error('Palette input was not rendered')
  input.setSelectionRange(0, input.value.length)
  await act(async () => {
    await userEvent.keyboard('#')
  })
  const thirdTask = container.querySelectorAll('[role="option"]')[2]
  if (thirdTask === undefined) throw new Error('Third task was not rendered')
  const { x, y, width, height } = thirdTask.getBoundingClientRect()
  await act(async () => {
    await userEvent.fill(input, 'Review')
  })
  await hoverAt(x + width / 2, y + height / 2)

  input.setSelectionRange(0, input.value.length)
  await act(async () => {
    await userEvent.keyboard('#')
  })

  expect(
    Array.from(container.querySelectorAll('[role="option"]')).map((item) => ({
      title: item.querySelector('.command-palette-item-heading strong')
        ?.textContent,
      selected: item.getAttribute('aria-selected'),
    })),
  ).toEqual([
    { title: '#51 First notification task', selected: 'true' },
    { title: '#52 Second notification task', selected: 'false' },
    { title: '#53 Third notification task', selected: 'false' },
  ])
})

it('highlights a result when the pointer moves over it', async () => {
  const sessions = [
    {
      ...session,
      id: 'session-first',
      title: 'Review first notification',
      task: {
        id: 'task-first',
        number: 51,
        title: 'First notification task',
        priority: 1,
      },
    },
    {
      ...session,
      id: 'session-second',
      title: 'Review second notification',
      task: {
        id: 'task-second',
        number: 52,
        title: 'Second notification task',
        priority: 2,
      },
    },
  ] satisfies Session[]
  renderPalette({ sessions, initialQuery: 'Review' })
  const secondResult = container.querySelectorAll('[role="option"]')[1]
  if (secondResult === undefined)
    throw new Error('Second result was not rendered')

  await act(async () => {
    await userEvent.hover(secondResult)
  })

  expect(
    Array.from(container.querySelectorAll('[role="option"]')).map((item) =>
      item.getAttribute('aria-selected'),
    ),
  ).toEqual(['false', 'true'])
})

it('shows terminal focus progress in the actions submenu', () => {
  renderPalette({
    initialActionItemId: session.id,
    pendingFocusSessionId: session.id,
  })
  const button = Array.from(
    container.querySelectorAll<HTMLButtonElement>('[role="menuitem"]'),
  ).find((item) => item.textContent === 'Opening terminal…')
  if (button === undefined) throw new Error('Expected terminal focus action')

  const snapshot = () => ({
    label: button.textContent,
    disabled: button.disabled,
    busy: button.getAttribute('aria-busy'),
    submenuVisible: container.querySelector('[role="menu"]') !== null,
  })
  expect(snapshot()).toEqual({
    label: 'Opening terminal…',
    disabled: true,
    busy: 'true',
    submenuVisible: true,
  })
})

it('copies the session ID from the actions submenu with Enter', async () => {
  const writeText = vi
    .fn<(text: string) => Promise<void>>()
    .mockResolvedValue(undefined)
  vi.stubGlobal('navigator', {
    clipboard: { writeText },
    platform: 'Linux x86_64',
  })
  renderPalette()

  await openActionsMenu()
  await pressKeys('{ArrowDown}', '{ArrowDown}', '{ArrowDown}', '{Enter}')

  const snapshot = () => ({
    clipboardCalls: writeText.mock.calls,
    activeActionLabel: container.querySelector(
      '[data-palette-action-active="true"] button',
    )?.textContent,
  })

  expect(snapshot()).toEqual({
    clipboardCalls: [['session-alerts']],
    activeActionLabel: 'Copied',
  })
})

it('hides terminal action when the session has no terminal capability', () => {
  const unavailableSession = { ...session, canOpenInTerminal: false }
  renderPalette({
    initialActionItemId: unavailableSession.id,
    sessions: [unavailableSession],
  })

  expect(
    Array.from(container.querySelectorAll('[role="menuitem"]')).map(
      (item) => item.textContent,
    ),
  ).toEqual([
    'Open session',
    'Open beside',
    'Later',
    'Copy session ID',
    'Close this session',
  ])
})
