import { act, type ComponentProps } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { userEvent } from 'vitest/browser'

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
  branch: 'feature/notification-preferences',
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
        onOpenSession={() => undefined}
        onLater={() => undefined}
        onOpenInTerminal={() => undefined}
        onOpenProject={() => undefined}
        onRunCommand={() => undefined}
        {...props}
      />,
    )
  })
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

  expect(
    container.querySelector('.command-palette-item-subtitle')?.textContent,
  ).toBe(
    'P1 #42 Improve notifications +1 · feature/notification-preferences · example/web-app',
  )
})

it.each([
  ['needs action', 'needs_action', 'Question', 300, '◆', 'Question', '5m'],
  ['error', 'needs_action', 'Error', 720, '×', 'Error', '12m'],
  ['in progress', 'in_progress', null, 30, '●', 'Working', '30s'],
  ['later', 'later', null, 90, '◷', 'Later', ''],
  ['done', 'done', null, 360, '✓', 'Done', ''],
  ['short session', 'needs_action', 'Question', 8, '◆', 'Question', '8s'],
] as const)(
  'renders the %s session icon, kind, and elapsed time',
  (
    _name,
    category,
    kind,
    elapsedSeconds,
    icon,
    expectedKind,
    expectedDetail,
  ) => {
    renderPalette({
      sessions: [{ ...session, category, kind, elapsedSeconds }],
      initialQuery: 'Review notification settings',
    })

    const item = container.querySelector('.command-palette-item')
    const snapshot = () => ({
      detail: item?.querySelector('.command-palette-item-detail')?.textContent,
      icon: item?.querySelector('.command-palette-status')?.textContent,
      kind: item?.querySelector('.command-palette-item-kind')?.textContent,
      title: item?.querySelector('.command-palette-item-heading strong')
        ?.textContent,
    })
    expect(snapshot()).toEqual({
      detail: expectedDetail,
      icon,
      kind: expectedKind,
      title: 'Review notification settings',
    })
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
  ).toBe('P1 · 1 sessions · 1 need action · example/web-app')
})

it.each([
  ['switch board', '>Switch to Board', '⇧B'],
  ['switch inbox', '>Switch to Inbox', ''],
  ['archive done', '>Archive all done', ''],
] as const)(
  'shows only the assigned shortcut for the %s command',
  (_name, query, expectedShortcut) => {
    renderPalette({ initialQuery: query })

    expect(
      container.querySelector('.command-palette-item-detail')?.textContent,
    ).toBe(expectedShortcut)
  },
)

it('renders every result detail with monospace styling', () => {
  renderPalette({ initialQuery: 'notification' })

  const actual = Array.from(
    container.querySelectorAll('.command-palette-item-detail'),
  ).map((item) => item.className)
  expect(actual).toEqual([
    'command-palette-item-detail is-monospace',
    'command-palette-item-detail is-monospace',
    'command-palette-item-detail is-monospace',
  ])
})

it.each([
  [
    'results',
    undefined,
    ['↵ Open', '→ Actions', 'Tab Scope', '# task · @ project · > command'],
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
        id: 'switch-inbox',
        label: 'Switch to Inbox',
        description: 'Show the session inbox',
      },
    ],
  ])
})

it('matches a session by branch name and shows its task, branch, and repository', () => {
  renderPalette({ initialQuery: 'feature/notification-preferences' })

  expect(container.querySelector('.command-palette-item')?.textContent).toEqual(
    '◆Review notification settingsQuestionP1 #42 Improve notifications · feature/notification-preferences · example/web-app5m',
  )
})

it.each([
  ['omitted', undefined],
  ['null', null],
  ['blank', '   '],
] as const)('omits the branch label for %s', (_, branch) => {
  const sessionWithoutBranch: Session = { ...session }
  if (branch === undefined) {
    delete sessionWithoutBranch.branch
  } else {
    sessionWithoutBranch.branch = branch
  }
  renderPalette({ sessions: [sessionWithoutBranch] })

  expect(
    container.querySelector('.command-palette-item-subtitle')?.textContent,
  ).toEqual('P1 #42 Improve notifications · example/web-app')
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
  ).toEqual(['Open session', 'Later', 'Copy session ID', 'Open in terminal'])
})

it('opens the highlighted session actions with ArrowRight and runs Later', async () => {
  const onLater = vi.fn<(id: string) => void>()
  renderPalette({ onLater })

  await act(async () => {
    await userEvent.keyboard('{ArrowRight}{ArrowDown}{Enter}')
  })

  expect(onLater.mock.calls).toEqual([['session-alerts']])
})

it('opens the highlighted session in its terminal from the actions submenu', async () => {
  const onOpenInTerminal = vi.fn<(id: string) => void>()
  renderPalette({ onOpenInTerminal })

  await act(async () => {
    await userEvent.keyboard(
      '{ArrowRight}{ArrowDown}{ArrowDown}{ArrowDown}{Enter}',
    )
  })

  expect(onOpenInTerminal.mock.calls).toEqual([['session-alerts']])
})

it('copies the session ID from the actions submenu with Enter', async () => {
  const writeText = vi
    .fn<(text: string) => Promise<void>>()
    .mockResolvedValue(undefined)
  vi.stubGlobal('navigator', { clipboard: { writeText } })
  renderPalette()

  await act(async () => {
    await userEvent.keyboard('{ArrowRight}{ArrowDown}{ArrowDown}{Enter}')
  })

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
  ).toEqual(['Open session', 'Later', 'Copy session ID'])
})
