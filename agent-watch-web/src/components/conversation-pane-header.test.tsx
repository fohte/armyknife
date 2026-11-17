import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'

import { ConversationPaneHeader } from '#components/conversation-pane-header'
import type { Session, SessionActions } from '#types'

const { copySessionIdMock } = vi.hoisted(() => ({
  copySessionIdMock: vi.fn(),
}))

vi.mock('#components/copy-session-id-control', () => ({
  CopySessionIdControl: () => null,
}))

vi.mock('#components/use-copy-session-id', () => ({
  useCopySessionId: () => ({
    status: 'idle',
    copySessionId: copySessionIdMock,
  }),
}))

let container: HTMLDivElement
let root: Root | null = null

const session: Session = {
  id: 'fixture-session',
  engine: 'claude',
  category: 'needs_action',
  isPaused: false,
  kind: 'Message',
  title: 'A fixture title',
  preview: null,
  elapsedSeconds: 0,
  unread: false,
  task: null,
  repo: 'example/project',
  project: 'project',
  parent: null,
  ancestorSessionIds: [],
  pullRequest: null,
}

const actions: SessionActions = {
  message: true,
  answerQuestion: false,
  permission: { approve: false, reject: false },
  interrupt: false,
  openInTerminal: false,
  rewind: false,
}

function requireElement<T extends Element>(element: T | null): T {
  if (element === null) throw new Error('Expected element was not rendered')
  return element
}

function renderHeader(
  isMobile: boolean,
  onAction: (
    action: string,
    payload?: Record<string, unknown>,
  ) => Promise<boolean>,
  currentSession = session,
  options: {
    onLater?: () => void
    onClose?: () => void
    onToggleToday?: (isToday: boolean) => void
    onToggleStatusPanel?: () => void
    mobileActionsMenuOpen?: boolean
    isToday?: boolean
  } = {},
) {
  act(() => {
    root?.render(
      <ConversationPaneHeader
        key={currentSession.id}
        session={currentSession}
        sessions={[]}
        actions={actions}
        isMobile={isMobile}
        statusPanelOpen={false}
        backgroundTasks={[]}
        openInTerminalStatus={null}
        isToday={options.isToday ?? false}
        {...(options.mobileActionsMenuOpen === undefined
          ? {}
          : { mobileActionsMenuOpen: options.mobileActionsMenuOpen })}
        onAction={onAction}
        onOpenInTerminal={() => undefined}
        onSelectSession={() => undefined}
        onToggleStatusPanel={options.onToggleStatusPanel ?? (() => undefined)}
        {...(options.onLater === undefined ? {} : { onLater: options.onLater })}
        {...(options.onClose === undefined ? {} : { onClose: options.onClose })}
        {...(options.onToggleToday === undefined
          ? {}
          : { onToggleToday: options.onToggleToday })}
      />,
    )
  })
}

function setInputValue(input: HTMLInputElement, value: string) {
  const valueSetter = Reflect.getOwnPropertyDescriptor(
    input.constructor.prototype,
    'value',
  )?.set
  if (valueSetter !== undefined) Reflect.apply(valueSetter, input, [value])
  input.dispatchEvent(new Event('input', { bubbles: true }))
}

function mobileActionMenuItems() {
  return Array.from(
    document.body.querySelectorAll<HTMLElement>('[role="menuitem"]'),
  )
}

function mobileActionMenuItem(label: string) {
  return requireElement(
    mobileActionMenuItems().find((item) => item.textContent === label) ?? null,
  )
}

async function clickMobileActionMenuItem(label: string) {
  await act(async () => {
    requireElement(
      container.querySelector<HTMLButtonElement>(
        'button[aria-label="More actions"]',
      ),
    ).click()
    await new Promise((resolve) => window.setTimeout(resolve, 0))
  })
  await act(async () => {
    mobileActionMenuItem(label).click()
    await new Promise((resolve) => window.setTimeout(resolve, 0))
  })
}

function isMobileActionMenuClosed() {
  return (
    container
      .querySelector('button[aria-label="More actions"]')
      ?.getAttribute('aria-expanded') === 'false'
  )
}

beforeEach(() => {
  vi.clearAllMocks()
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

it.each([
  {
    description: 'desktop header saves a nonempty title',
    isMobile: false,
    title: 'Updated fixture title',
    saved: true,
    expectedDraft: null,
  },
  {
    description: 'mobile header clears the title',
    isMobile: true,
    title: '',
    saved: true,
    expectedDraft: null,
  },
  {
    description: 'desktop header keeps an unsaved title after failure',
    isMobile: false,
    title: 'Uncommitted fixture title',
    saved: false,
    expectedDraft: 'Uncommitted fixture title',
  },
])('$description', async ({ isMobile, title, saved, expectedDraft }) => {
  let completeSave: (result: boolean) => void = () => undefined
  const saveResult = new Promise<boolean>((resolve) => {
    completeSave = resolve
  })
  const onAction = vi.fn(
    (action: string, payload?: Record<string, unknown>): Promise<boolean> => {
      return action === 'title' && typeof payload?.['title'] === 'string'
        ? saveResult
        : Promise.resolve(false)
    },
  )
  renderHeader(
    isMobile,
    onAction,
    session,
    isMobile ? { mobileActionsMenuOpen: true } : {},
  )

  act(() => {
    if (isMobile) {
      requireElement(
        document.body.querySelector<HTMLElement>(
          '[role="menuitem"]:last-child',
        ),
      ).click()
      return
    }
    requireElement(
      container.querySelector<HTMLButtonElement>(
        '[aria-label="Edit session title"]',
      ),
    ).click()
  })
  const input = requireElement(
    container.querySelector<HTMLInputElement>(
      'input[aria-label="Session title"]',
    ),
  )
  act(() => {
    setInputValue(input, title)
  })
  act(() => {
    requireElement(
      container.querySelector<HTMLButtonElement>('button[type="submit"]'),
    ).click()
  })
  const inputDisabledDuringSave = input.disabled
  const saveButtonTextDuringSave = requireElement(
    container.querySelector<HTMLButtonElement>('button[type="submit"]'),
  ).textContent
  await act(async () => {
    completeSave(saved)
    await saveResult
  })

  const snapshot = () => ({
    action: onAction.mock.calls,
    inputDisabledDuringSave,
    saveButtonTextDuringSave,
    editor: (() => {
      const editor = container.querySelector<HTMLInputElement>(
        'input[aria-label="Session title"]',
      )
      return editor === null
        ? { visible: false, draft: null }
        : { visible: true, draft: editor.value }
    })(),
  })
  expect(snapshot()).toEqual({
    action: [['title', { title }]],
    inputDisabledDuringSave: true,
    saveButtonTextDuringSave: 'Saving…',
    editor:
      expectedDraft === null
        ? { visible: false, draft: null }
        : { visible: true, draft: expectedDraft },
  })
})

it('starts a fresh editor when the selected session changes', () => {
  const onAction = vi.fn(() => Promise.resolve(false))
  renderHeader(false, onAction)

  act(() => {
    requireElement(
      container.querySelector<HTMLButtonElement>(
        '[aria-label="Edit session title"]',
      ),
    ).click()
  })
  const input = requireElement(
    container.querySelector<HTMLInputElement>(
      'input[aria-label="Session title"]',
    ),
  )
  act(() => {
    setInputValue(input, 'Uncommitted fixture title')
  })

  renderHeader(false, onAction, {
    ...session,
    id: 'next-fixture-session',
    title: 'Another fixture title',
  })

  const snapshot = () => ({
    action: onAction.mock.calls,
    editButtonVisible:
      container.querySelector('[aria-label="Edit session title"]') !== null,
    editorVisible:
      container.querySelector('input[aria-label="Session title"]') !== null,
  })
  expect(snapshot()).toEqual({
    action: [],
    editButtonVisible: true,
    editorVisible: false,
  })
})

it('keeps secondary controls out of the mobile header', () => {
  renderHeader(
    true,
    vi.fn(() => Promise.resolve(true)),
    session,
    {
      isToday: true,
    },
  )

  const snapshot = () => ({
    headerButtons: Array.from(
      container.querySelectorAll<HTMLButtonElement>(
        '.mobile-conversation-back, .mobile-conversation-status-toggle, .mobile-conversation-more',
      ),
      (button) => button.getAttribute('aria-label'),
    ),
    titleCopyButtonVisible:
      container.querySelector('.conversation-copy-session-id') !== null,
    titleEditButtonVisible:
      container.querySelector('[aria-label="Edit session title"]') !== null,
  })
  expect(snapshot()).toEqual({
    headerButtons: [
      'Back to Inbox',
      'Open session status panel',
      'More actions',
    ],
    titleCopyButtonVisible: false,
    titleEditButtonVisible: false,
  })
})

it('places the mobile Starred marker before the session title', () => {
  renderHeader(
    true,
    vi.fn(() => Promise.resolve(true)),
    session,
    {
      isToday: true,
    },
  )

  const snapshot = () =>
    Array.from(
      requireElement(container.querySelector('.mobile-conversation-title'))
        .children,
      (element) =>
        element.classList.contains('mobile-conversation-starred')
          ? 'star'
          : 'title',
    )
  expect(snapshot()).toEqual(['star', 'title'])
})

it('opens the mobile status panel without opening the actions menu', () => {
  const onToggleStatusPanel = vi.fn()
  renderHeader(
    true,
    vi.fn(() => Promise.resolve(true)),
    session,
    {
      onToggleStatusPanel,
    },
  )

  act(() => {
    requireElement(
      container.querySelector<HTMLButtonElement>(
        '[aria-label="Open session status panel"]',
      ),
    ).click()
  })

  const snapshot = () => ({
    statusPanelCalls: onToggleStatusPanel.mock.calls,
    actionsMenuClosed: isMobileActionMenuClosed(),
  })
  expect(snapshot()).toEqual({
    statusPanelCalls: [[]],
    actionsMenuClosed: true,
  })
})

it('hides Close for an active mobile session even when a callback is provided', () => {
  renderHeader(
    true,
    vi.fn(() => Promise.resolve(true)),
    session,
    {
      mobileActionsMenuOpen: true,
      onClose: vi.fn(),
    },
  )

  const snapshot = () => mobileActionMenuItems().map((item) => item.textContent)
  expect(snapshot()).toEqual([
    'Later',
    'Add to Starred',
    'Copy session ID',
    'Edit title',
  ])
})

it('runs Later from the mobile actions menu', async () => {
  const onLater = vi.fn()
  renderHeader(
    true,
    vi.fn(() => Promise.resolve(true)),
    session,
    { onLater },
  )
  await clickMobileActionMenuItem('Later')

  const snapshot = () => ({
    calls: onLater.mock.calls,
    menuClosed: isMobileActionMenuClosed(),
  })
  expect(snapshot()).toEqual({ calls: [[]], menuClosed: true })
})

it('runs Close from the mobile actions menu for a Done session', async () => {
  const onClose = vi.fn()
  renderHeader(
    true,
    vi.fn(() => Promise.resolve(true)),
    { ...session, category: 'done' },
    { onClose },
  )
  await clickMobileActionMenuItem('Close')

  const snapshot = () => ({
    calls: onClose.mock.calls,
    menuClosed: isMobileActionMenuClosed(),
  })
  expect(snapshot()).toEqual({ calls: [[]], menuClosed: true })
})

it.each([
  {
    description: 'adds an unstarred session to Starred',
    isToday: false,
    label: 'Add to Starred',
    nextValue: true,
  },
  {
    description: 'removes a starred session from Starred',
    isToday: true,
    label: 'Remove from Starred',
    nextValue: false,
  },
])(
  '$description from the mobile actions menu',
  async ({ isToday, label, nextValue }) => {
    const onToggleToday = vi.fn()
    renderHeader(
      true,
      vi.fn(() => Promise.resolve(true)),
      session,
      { isToday, onToggleToday },
    )
    await clickMobileActionMenuItem(label)

    const snapshot = () => ({
      calls: onToggleToday.mock.calls,
      menuClosed: isMobileActionMenuClosed(),
    })
    expect(snapshot()).toEqual({
      calls: [[nextValue]],
      menuClosed: true,
    })
  },
)

it('copies the session ID from the mobile actions menu', async () => {
  renderHeader(
    true,
    vi.fn(() => Promise.resolve(true)),
  )
  await clickMobileActionMenuItem('Copy session ID')

  const snapshot = () => ({
    calls: copySessionIdMock.mock.calls,
    menuClosed: isMobileActionMenuClosed(),
  })
  expect(snapshot()).toEqual({ calls: [[]], menuClosed: true })
})

it('opens the title editor from the mobile actions menu', async () => {
  renderHeader(
    true,
    vi.fn(() => Promise.resolve(true)),
  )
  await clickMobileActionMenuItem('Edit title')

  const snapshot = () => ({
    editorVisible:
      container.querySelector('input[aria-label="Session title"]') !== null,
    menuClosed: isMobileActionMenuClosed(),
  })
  expect(snapshot()).toEqual({ editorVisible: true, menuClosed: true })
})

it('summarizes every pull request and opens the full list', () => {
  renderHeader(
    false,
    vi.fn(() => Promise.resolve(true)),
    {
      ...session,
      pullRequest: {
        number: 42,
        state: 'Open',
        owner: 'example-org',
        repo: 'example-repo',
        url: 'https://github.com/example-org/example-repo/pull/42',
        headSha: 'sample-head-sha',
        ciState: 'InProgress',
        isCurrentBranch: true,
      },
      pullRequests: [
        {
          number: 42,
          state: 'Open',
          owner: 'example-org',
          repo: 'example-repo',
          url: 'https://github.com/example-org/example-repo/pull/42',
          headSha: 'sample-head-sha',
          ciState: 'InProgress',
          isCurrentBranch: true,
        },
        {
          number: 43,
          state: 'Open',
          owner: 'example-org',
          repo: 'example-repo',
          url: 'https://github.com/example-org/example-repo/pull/43',
          headSha: 'other-head-sha',
          ciState: 'Successful',
          isCurrentBranch: false,
        },
      ],
    },
  )

  const trigger = requireElement(
    container.querySelector<HTMLButtonElement>('.session-pr-trigger'),
  )
  act(() => {
    trigger.click()
  })

  const snapshot = () => ({
    triggerLabel: trigger.getAttribute('aria-label'),
    expanded: trigger.getAttribute('aria-expanded'),
    pullRequests: Array.from(
      container.querySelectorAll<HTMLAnchorElement>('.session-pr-row'),
      (link) => ({
        number: link.querySelector('.session-pr-number')?.textContent,
        href: link.getAttribute('href'),
      }),
    ),
  })
  expect(snapshot()).toEqual({
    triggerLabel: 'PR #42, open, CI running. 1 more PR from this session',
    expanded: 'true',
    pullRequests: [
      {
        number: '#42',
        href: 'https://github.com/example-org/example-repo/pull/42',
      },
      {
        number: '#43',
        href: 'https://github.com/example-org/example-repo/pull/43',
      },
    ],
  })
})

it('merges an open pull request when an earlier session PR is closed', async () => {
  const fetchMock = vi.fn<typeof fetch>(() =>
    Promise.resolve(Response.json({ result: 'merged' })),
  )
  vi.stubGlobal('fetch', fetchMock)
  renderHeader(
    false,
    vi.fn(() => Promise.resolve(true)),
    {
      ...session,
      pullRequests: [
        {
          number: 41,
          state: 'Closed',
          owner: 'example-org',
          repo: 'example-repo',
          url: 'https://github.com/example-org/example-repo/pull/41',
          ciState: 'Unknown',
          isCurrentBranch: false,
        },
        {
          number: 42,
          state: 'Open',
          owner: 'example-org',
          repo: 'example-repo',
          url: 'https://github.com/example-org/example-repo/pull/42',
          headSha: 'fixture-head-sha',
          ciState: 'Successful',
          isCurrentBranch: false,
        },
      ],
    },
  )

  const trigger = requireElement(
    container.querySelector<HTMLButtonElement>('.session-pr-trigger'),
  )
  act(() => {
    requireElement(
      container.querySelector<HTMLButtonElement>(
        '.conversation-pr-merge-button',
      ),
    ).click()
  })
  const confirmation = requireElement(
    container.querySelector<HTMLElement>('[role="alertdialog"]'),
  )

  await act(async () => {
    requireElement(
      [...confirmation.querySelectorAll<HTMLButtonElement>('button')].find(
        (button) => button.textContent === 'Confirm merge',
      ) ?? null,
    ).click()
    for (let attempt = 0; attempt < 20; attempt += 1) {
      if (container.querySelector('[role="status"]') !== null) return
      await new Promise((resolve) => window.setTimeout(resolve, 0))
    }
  })

  const mergeState = () => ({
    summary: trigger.getAttribute('aria-label'),
    request: fetchMock.mock.calls.map(([url]) => url),
    feedback: container.querySelector('[role="status"]')?.textContent,
  })
  expect(mergeState()).toEqual({
    summary: 'PR #42, open, CI passed. 1 more PR from this session',
    request: ['/api/sessions/fixture-session/pull-requests/42/merge'],
    feedback: 'PR #42 merged.',
  })
})

it('keeps merge feedback scoped to the repository when PR numbers match', async () => {
  const fetchMock = vi.fn<typeof fetch>(() =>
    Promise.resolve(Response.json({ result: 'merged' })),
  )
  vi.stubGlobal('fetch', fetchMock)
  const firstRepositoryPullRequest = {
    number: 42,
    state: 'Open' as const,
    owner: 'example-org',
    repo: 'example-repository',
    url: 'https://github.com/example-org/example-repository/pull/42',
    headSha: 'fixture-first-head',
    ciState: 'Successful' as const,
    isCurrentBranch: false,
  }
  const secondRepositoryPullRequest = {
    number: 42,
    state: 'Open' as const,
    owner: 'example-org',
    repo: 'example-infrastructure',
    url: 'https://github.com/example-org/example-infrastructure/pull/42',
    headSha: 'fixture-second-head',
    ciState: 'Successful' as const,
    isCurrentBranch: false,
  }
  renderHeader(
    false,
    vi.fn(() => Promise.resolve(true)),
    {
      ...session,
      pullRequests: [firstRepositoryPullRequest, secondRepositoryPullRequest],
    },
  )

  act(() => {
    requireElement(
      container.querySelector<HTMLButtonElement>(
        '.conversation-pr-merge-button',
      ),
    ).click()
  })
  const confirmation = requireElement(
    container.querySelector<HTMLElement>('[role="alertdialog"]'),
  )
  await act(async () => {
    requireElement(
      [...confirmation.querySelectorAll<HTMLButtonElement>('button')].find(
        (button) => button.textContent === 'Confirm merge',
      ) ?? null,
    ).click()
    for (let attempt = 0; attempt < 20; attempt += 1) {
      if (container.querySelector('[role="status"]') !== null) return
      await new Promise((resolve) => window.setTimeout(resolve, 0))
    }
  })

  renderHeader(
    false,
    vi.fn(() => Promise.resolve(true)),
    {
      ...session,
      pullRequests: [
        { ...firstRepositoryPullRequest, state: 'Merged', headSha: null },
        secondRepositoryPullRequest,
      ],
    },
  )

  const mergeState = () => ({
    summary: container
      .querySelector('.session-pr-trigger')
      ?.getAttribute('aria-label'),
    mergeButton:
      container.querySelector('.conversation-pr-merge-button') !== null,
    feedback:
      container.querySelector('.conversation-pr-merge-feedback')?.textContent ??
      null,
  })
  expect(mergeState()).toEqual({
    summary: 'PR #42, open, CI passed. 1 more PR from this session',
    mergeButton: true,
    feedback: null,
  })
})

it('shows mobile merge feedback after confirming from the pull request sheet', async () => {
  const fetchMock = vi.fn<typeof fetch>(() =>
    Promise.resolve(Response.json({ result: 'merged' })),
  )
  vi.stubGlobal('fetch', fetchMock)
  renderHeader(
    true,
    vi.fn(() => Promise.resolve(true)),
    {
      ...session,
      pullRequest: {
        number: 42,
        state: 'Open',
        owner: 'example-org',
        repo: 'example-repo',
        url: 'https://github.com/example-org/example-repo/pull/42',
        headSha: 'fixture-head-sha',
        ciState: 'Successful',
        isCurrentBranch: true,
      },
    },
  )

  act(() => {
    requireElement(
      container.querySelector<HTMLButtonElement>('.session-pr-trigger--mobile'),
    ).click()
  })
  const sheet = requireElement(
    container.querySelector<HTMLElement>('.session-pr-list--mobile'),
  )
  act(() => {
    requireElement(
      [...sheet.querySelectorAll<HTMLButtonElement>('button')].find(
        (button) => button.textContent === 'Merge',
      ) ?? null,
    ).click()
  })
  const confirmation = requireElement(
    container.querySelector<HTMLElement>('[role="alertdialog"]'),
  )

  await act(async () => {
    requireElement(
      [...confirmation.querySelectorAll<HTMLButtonElement>('button')].find(
        (button) => button.textContent === 'Confirm merge',
      ) ?? null,
    ).click()
    for (let attempt = 0; attempt < 20; attempt += 1) {
      if (container.querySelector('.conversation-pr-merge-feedback') !== null) {
        return
      }
      await new Promise((resolve) => window.setTimeout(resolve, 0))
    }
  })

  const mergeState = () => ({
    request: fetchMock.mock.calls.map(([url]) => url),
    confirmationOpen: container.querySelector('[role="alertdialog"]') !== null,
    feedback: container.querySelector('.conversation-pr-merge-feedback')
      ?.textContent,
    feedbackRole: container
      .querySelector('.conversation-pr-merge-feedback')
      ?.getAttribute('role'),
    headerHasFeedback: container
      .querySelector('.conversation-header')
      ?.classList.contains('has-session-pr-feedback'),
  })
  expect(mergeState()).toEqual({
    request: ['/api/sessions/fixture-session/pull-requests/42/merge'],
    confirmationOpen: false,
    feedback: 'PR #42 merged.',
    feedbackRole: 'status',
    headerHasFeedback: true,
  })
})

it('merges the session pull request after confirming its head SHA', async () => {
  let resolveMerge: (response: Response) => void = () => undefined
  const mergeResponse = new Promise<Response>((resolve) => {
    resolveMerge = resolve
  })
  const fetchMock = vi.fn<typeof fetch>(() => mergeResponse)
  vi.stubGlobal('fetch', fetchMock)
  renderHeader(
    false,
    vi.fn(() => Promise.resolve(true)),
    {
      ...session,
      pullRequest: {
        number: 42,
        state: 'Open',
        owner: 'example-org',
        repo: 'example-repo',
        url: 'https://github.com/example-org/example-repo/pull/42',
        headSha: 'sample-head-sha',
      },
    },
  )

  act(() => {
    requireElement(
      [...container.querySelectorAll<HTMLButtonElement>('button')].find(
        (button) => button.textContent === 'Merge',
      ) ?? null,
    ).click()
  })
  const confirmation = requireElement(
    container.querySelector<HTMLElement>('[role="alertdialog"]'),
  )
  const confirmedSha = confirmation.querySelector('code')?.textContent
  act(() => {
    requireElement(
      [...confirmation.querySelectorAll<HTMLButtonElement>('button')].find(
        (button) => button.textContent === 'Confirm merge',
      ) ?? null,
    ).click()
  })

  const pendingMergeState = () => ({
    confirmationOpen: container.querySelector('[role="alertdialog"]') !== null,
    mergeButton: (() => {
      const button = container.querySelector<HTMLButtonElement>(
        '.conversation-pr-merge-button',
      )
      return button === null
        ? null
        : { text: button.textContent, disabled: button.disabled }
    })(),
    feedback: container.querySelector('[role="status"]')?.textContent ?? null,
  })

  const pendingState = pendingMergeState()

  await act(async () => {
    resolveMerge(Response.json({ result: 'merged' }))
    for (let attempt = 0; attempt < 20; attempt += 1) {
      if (container.querySelector('[role="status"]') !== null) return
      await new Promise((resolve) => window.setTimeout(resolve, 0))
    }
  })

  const mergeState = () => ({
    confirmedSha,
    pendingState,
    request: fetchMock.mock.calls.map(([url, init]) => ({
      url,
      method: init?.method,
      body: init?.body,
    })),
    confirmationOpen: container.querySelector('[role="alertdialog"]') !== null,
    mergeButton:
      container.querySelector('.conversation-pr-merge-button') !== null,
    feedback: container.querySelector('[role="status"]')?.textContent,
  })
  expect(mergeState()).toEqual({
    confirmedSha: 'sample-head-sha',
    pendingState: {
      confirmationOpen: false,
      mergeButton: { text: 'Merging…', disabled: true },
      feedback: null,
    },
    request: [
      {
        url: '/api/sessions/fixture-session/pull-requests/42/merge',
        method: 'POST',
        body: JSON.stringify({
          owner: 'example-org',
          repo: 'example-repo',
          expectedHeadSha: 'sample-head-sha',
        }),
      },
    ],
    confirmationOpen: false,
    mergeButton: false,
    feedback: 'PR #42 merged.',
  })
})

it('keeps the merge action available when the session pull request is queued', async () => {
  const fetchMock = vi.fn<typeof fetch>(() =>
    Promise.resolve(
      Response.json({ result: 'enqueued', message: 'Waiting for checks.' }),
    ),
  )
  vi.stubGlobal('fetch', fetchMock)
  renderHeader(
    false,
    vi.fn(() => Promise.resolve(true)),
    {
      ...session,
      pullRequest: {
        number: 42,
        state: 'Open',
        owner: 'example-org',
        repo: 'example-repo',
        url: 'https://github.com/example-org/example-repo/pull/42',
        headSha: 'sample-head-sha',
      },
    },
  )

  act(() => {
    requireElement(
      [...container.querySelectorAll<HTMLButtonElement>('button')].find(
        (button) => button.textContent === 'Merge',
      ) ?? null,
    ).click()
  })
  const confirmation = requireElement(
    container.querySelector<HTMLElement>('[role="alertdialog"]'),
  )
  await act(async () => {
    requireElement(
      [...confirmation.querySelectorAll<HTMLButtonElement>('button')].find(
        (button) => button.textContent === 'Confirm merge',
      ) ?? null,
    ).click()
    for (let attempt = 0; attempt < 20; attempt += 1) {
      if (container.querySelector('[role="status"]') !== null) return
      await new Promise((resolve) => window.setTimeout(resolve, 0))
    }
  })

  const mergeState = () => ({
    request: fetchMock.mock.calls.map(([url, init]) => ({
      url,
      method: init?.method,
      body: init?.body,
    })),
    confirmationOpen: container.querySelector('[role="alertdialog"]') !== null,
    mergeButton:
      container.querySelector('.conversation-pr-merge-button') !== null,
    feedback: container.querySelector('[role="status"]')?.textContent,
  })
  expect(mergeState()).toEqual({
    request: [
      {
        url: '/api/sessions/fixture-session/pull-requests/42/merge',
        method: 'POST',
        body: JSON.stringify({
          owner: 'example-org',
          repo: 'example-repo',
          expectedHeadSha: 'sample-head-sha',
        }),
      },
    ],
    confirmationOpen: false,
    mergeButton: true,
    feedback: 'PR #42 was added to the merge queue: Waiting for checks.',
  })
})

it('keeps the merge action available after a stale-head merge failure', async () => {
  const fetchMock = vi.fn<typeof fetch>(() =>
    Promise.resolve(
      Response.json(
        { error: 'The pull request head changed.' },
        { status: 409 },
      ),
    ),
  )
  vi.stubGlobal('fetch', fetchMock)
  renderHeader(
    false,
    vi.fn(() => Promise.resolve(true)),
    {
      ...session,
      pullRequest: {
        number: 42,
        state: 'Open',
        owner: 'example-org',
        repo: 'example-repo',
        url: 'https://github.com/example-org/example-repo/pull/42',
        headSha: 'sample-head-sha',
      },
    },
  )

  act(() => {
    requireElement(
      [...container.querySelectorAll<HTMLButtonElement>('button')].find(
        (button) => button.textContent === 'Merge',
      ) ?? null,
    ).click()
  })
  const confirmation = requireElement(
    container.querySelector<HTMLElement>('[role="alertdialog"]'),
  )
  await act(async () => {
    requireElement(
      [...confirmation.querySelectorAll<HTMLButtonElement>('button')].find(
        (button) => button.textContent === 'Confirm merge',
      ) ?? null,
    ).click()
    for (let attempt = 0; attempt < 20; attempt += 1) {
      if (container.querySelector('[role="alert"]') !== null) return
      await new Promise((resolve) => window.setTimeout(resolve, 0))
    }
  })

  const mergeState = () => ({
    request: fetchMock.mock.calls.map(([url, init]) => ({
      url,
      method: init?.method,
      body: init?.body,
    })),
    confirmationOpen: container.querySelector('[role="alertdialog"]') !== null,
    mergeButton:
      container.querySelector('.conversation-pr-merge-button') !== null,
    feedback: container.querySelector('[role="alert"]')?.textContent,
  })
  expect(mergeState()).toEqual({
    request: [
      {
        url: '/api/sessions/fixture-session/pull-requests/42/merge',
        method: 'POST',
        body: JSON.stringify({
          owner: 'example-org',
          repo: 'example-repo',
          expectedHeadSha: 'sample-head-sha',
        }),
      },
    ],
    confirmationOpen: false,
    mergeButton: true,
    feedback: 'The pull request head changed.',
  })
})

it('hides the merge action when the session pull request has no head SHA', () => {
  renderHeader(
    false,
    vi.fn(() => Promise.resolve(true)),
    {
      ...session,
      pullRequest: {
        number: 42,
        state: 'Open',
        owner: 'example-org',
        repo: 'example-repo',
        headSha: null,
      },
    },
  )

  const headerState = () => ({
    pullRequestLabel: container
      .querySelector('.session-pr-trigger')
      ?.getAttribute('aria-label'),
    mergeButton: [
      ...container.querySelectorAll<HTMLButtonElement>('button'),
    ].some((button) => button.textContent === 'Merge'),
  })
  expect(headerState()).toEqual({
    pullRequestLabel: 'PR #42, open, CI status unknown',
    mergeButton: false,
  })
})
