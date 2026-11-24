import { act, useState } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'

import { AsyncActionFeedbackProvider } from '#async-action-feedback'
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

function HeaderWithMergeBandTarget(
  props: Parameters<typeof ConversationPaneHeader>[0],
) {
  const [target, setTarget] = useState<HTMLDivElement | null>(null)
  const [pullRequestTarget, setPullRequestTarget] =
    useState<HTMLDivElement | null>(null)
  const [statusPanelOpen, setStatusPanelOpen] = useState(props.statusPanelOpen)
  const toggleStatusPanel = () => {
    props.onToggleStatusPanel()
    setStatusPanelOpen((open) => !open)
  }
  return (
    <>
      <ConversationPaneHeader
        {...props}
        statusPanelOpen={statusPanelOpen}
        onToggleStatusPanel={toggleStatusPanel}
        mergeBandTarget={target}
        pullRequestListTarget={props.isMobile ? pullRequestTarget : null}
      />
      <div ref={setTarget} />
      {props.isMobile && statusPanelOpen ? (
        <div ref={setPullRequestTarget} />
      ) : null}
    </>
  )
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
      <AsyncActionFeedbackProvider>
        <HeaderWithMergeBandTarget
          key={currentSession.id}
          session={currentSession}
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
          {...(options.onLater === undefined
            ? {}
            : { onLater: options.onLater })}
          {...(options.onClose === undefined
            ? {}
            : { onClose: options.onClose })}
          {...(options.onToggleToday === undefined
            ? {}
            : { onToggleToday: options.onToggleToday })}
        />
      </AsyncActionFeedbackProvider>,
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
    mobileActionMenuItems().find((item) => item.textContent.includes(label)) ??
      null,
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
    activityButtonVisible:
      container.querySelector('.async-action-activity-button') !== null,
  })
  expect(snapshot()).toEqual({
    headerButtons: [
      'Back to Inbox',
      'Open session status panel',
      'More actions',
    ],
    titleCopyButtonVisible: false,
    titleEditButtonVisible: false,
    activityButtonVisible: false,
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

it('shows Close session as the final mobile action for an active session', () => {
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
    'Close session…',
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

it('opens the inherited vibe sheet from the first mobile menu item', async () => {
  renderHeader(
    true,
    vi.fn(() => Promise.resolve(true)),
    {
      ...session,
      vibeMode: { mode: 'inherited', ancestorSessionId: 'fixture-parent' },
    },
  )
  await act(async () => {
    requireElement(
      container.querySelector<HTMLButtonElement>(
        'button[aria-label="More actions"]',
      ),
    ).click()
    await new Promise((resolve) => window.setTimeout(resolve, 0))
  })
  const firstItem = mobileActionMenuItems()[0]?.textContent
  await act(async () => {
    mobileActionMenuItem('Vibe mode').click()
    await new Promise((resolve) => window.setTimeout(resolve, 0))
  })

  const snapshot = () => ({
    firstItem,
    menuClosed: isMobileActionMenuClosed(),
    sheetTitle: document.body.querySelector('.vibe-panel-title')?.textContent,
    preview: document.body.querySelector('.vibe-panel-message'),
  })
  expect(snapshot()).toEqual({
    firstItem: 'Vibe modeOn · inherited',
    menuClosed: true,
    sheetTitle: 'Vibe on · inherited',
    preview: null,
  })
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
      container.querySelectorAll<HTMLElement>('.session-pr-row'),
      (row) => ({
        number: row.querySelector('.session-pr-number')?.textContent,
        href: row.querySelector('a')?.getAttribute('href'),
      }),
    ),
  })
  expect(snapshot()).toEqual({
    triggerLabel:
      'PR #42: Untitled pull request, open, CI running. 1 more PR from this session',
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

it('shows pull request status on the mobile status panel button', () => {
  renderHeader(
    true,
    vi.fn(() => Promise.resolve(true)),
    {
      ...session,
      pullRequest: {
        number: 42,
        state: 'Open',
        owner: 'example-org',
        repo: 'example-project',
        title: 'Update the delivery flow',
        url: 'https://github.com/example-org/example-project/pull/42',
        headSha: 'fixture-head-sha',
        ciState: 'Successful',
        isCurrentBranch: true,
      },
      pullRequests: [
        {
          number: 42,
          state: 'Open',
          owner: 'example-org',
          repo: 'example-project',
          title: 'Update the delivery flow',
          url: 'https://github.com/example-org/example-project/pull/42',
          headSha: 'fixture-head-sha',
          ciState: 'Successful',
          isCurrentBranch: true,
        },
        {
          number: 43,
          state: 'Open',
          owner: 'example-org',
          repo: 'example-project',
          title: 'Add delivery retries',
          url: 'https://github.com/example-org/example-project/pull/43',
          headSha: 'fixture-other-head-sha',
          ciState: 'InProgress',
          isCurrentBranch: false,
        },
      ],
    },
  )

  const trigger = requireElement(
    container.querySelector<HTMLButtonElement>(
      '.mobile-conversation-status-toggle',
    ),
  )

  const snapshot = () => ({
    label: trigger.getAttribute('aria-label'),
    expanded: trigger.getAttribute('aria-expanded'),
    statusIcon: trigger.querySelector('svg') !== null,
    ciIndicator: trigger.querySelector('.session-pr-status-ci-dot')?.className,
    count: trigger.querySelector('.session-pr-status-count')?.textContent,
    dedicatedPrButton:
      container.querySelector('.session-pr-trigger--mobile') !== null,
    listRows: container.querySelectorAll(
      '.session-pr-list--panel .session-pr-row',
    ).length,
  })
  expect(snapshot()).toEqual({
    label:
      'Open session status panel. PR #42, open, CI passed. 1 more PR from this session',
    expanded: 'false',
    statusIcon: true,
    ciIndicator: 'session-pr-status-ci-dot is-passed',
    count: '2',
    dedicatedPrButton: false,
    listRows: 0,
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
          isCurrentBranch: true,
        },
      ],
    },
  )

  const trigger = requireElement(
    container.querySelector<HTMLButtonElement>('.session-pr-trigger'),
  )
  act(() => {
    requireElement(
      container.querySelector<HTMLButtonElement>('.merge-band-action'),
    ).click()
  })
  const confirmation = requireElement(
    container.querySelector<HTMLElement>('[role="alertdialog"]'),
  )
  const confirmationDescription =
    confirmation.querySelector('p')?.textContent ?? null

  await act(async () => {
    requireElement(
      [...confirmation.querySelectorAll<HTMLButtonElement>('button')].find(
        (button) => button.textContent === 'Confirm merge',
      ) ?? null,
    ).click()
    for (let attempt = 0; attempt < 20; attempt += 1) {
      if (container.querySelector('.async-action-item--success') !== null)
        return
      await new Promise((resolve) => window.setTimeout(resolve, 0))
    }
  })

  const mergeState = () => ({
    confirmationDescription,
    summary: trigger.getAttribute('aria-label'),
    request: fetchMock.mock.calls.map(([url]) => url),
    feedback: container.querySelector('[role="status"]')?.textContent,
  })
  expect(mergeState()).toEqual({
    confirmationDescription:
      'Merge PR #42 in example-org/example-repo using the confirmed head commit?',
    summary:
      'PR #42: Untitled pull request, merged, CI passed. 1 more PR from this session',
    request: ['/api/sessions/fixture-session/pull-requests/42/merge'],
    feedback: 'Merged PR #42A fixture title',
  })
})

it('merges a ready pull request from the merge band when several PRs exist', async () => {
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
          number: 42,
          state: 'Open',
          owner: 'example-org',
          repo: 'example-repo',
          url: 'https://github.com/example-org/example-repo/pull/42',
          headSha: 'fixture-running-head',
          ciState: 'InProgress',
          isCurrentBranch: true,
        },
        {
          number: 43,
          state: 'Open',
          owner: 'example-org',
          repo: 'example-infra',
          url: 'https://github.com/example-org/example-infra/pull/43',
          headSha: 'fixture-ready-head',
          ciState: 'Successful',
          isCurrentBranch: false,
        },
      ],
    },
  )

  act(() => {
    requireElement(
      container.querySelector<HTMLButtonElement>('.session-pr-trigger'),
    ).click()
  })
  const mergeButton = requireElement(
    container.querySelector<HTMLButtonElement>(
      '.merge-band-row button[aria-label^="Merge PR #43"]',
    ),
  )
  act(() => {
    mergeButton.click()
  })

  const confirmation = requireElement(
    container.querySelector<HTMLElement>('[role="alertdialog"]'),
  )
  const confirmedHeadSha = confirmation.querySelector('code')?.textContent
  await act(async () => {
    requireElement(
      [...confirmation.querySelectorAll<HTMLButtonElement>('button')].find(
        (button) => button.textContent === 'Confirm merge',
      ) ?? null,
    ).click()
    for (let attempt = 0; attempt < 20; attempt += 1) {
      if (container.querySelector('.async-action-item--success') !== null)
        return
      await new Promise((resolve) => window.setTimeout(resolve, 0))
    }
  })

  const mergeState = () => ({
    confirmedHeadSha,
    request: fetchMock.mock.calls.map(([url, init]) => ({
      url,
      method: init?.method,
      body: init?.body,
    })),
    feedback: container.querySelector('[role="status"]')?.textContent,
  })
  expect(mergeState()).toEqual({
    confirmedHeadSha: 'fixture-ready-head',
    request: [
      {
        url: '/api/sessions/fixture-session/pull-requests/43/merge',
        method: 'POST',
        body: JSON.stringify({
          owner: 'example-org',
          repo: 'example-infra',
          expectedHeadSha: 'fixture-ready-head',
        }),
      },
    ],
    feedback: 'Merged PR #43A fixture title',
  })
})

it.each([
  { description: 'desktop', isMobile: false },
  { description: 'phone', isMobile: true },
])(
  'opens the shared merge confirmation from a ready PR row on $description',
  ({ isMobile }) => {
    renderHeader(
      isMobile,
      vi.fn(() => Promise.resolve(true)),
      {
        ...session,
        pullRequests: [
          {
            number: 42,
            state: 'Open',
            owner: 'example-org',
            repo: 'example-repo',
            url: 'https://github.com/example-org/example-repo/pull/42',
            headSha: 'fixture-ready-head',
            ciState: 'Successful',
            isCurrentBranch: true,
          },
        ],
      },
    )

    if (isMobile) {
      act(() => {
        requireElement(
          container.querySelector<HTMLButtonElement>(
            '.mobile-conversation-status-toggle',
          ),
        ).click()
      })
    } else {
      act(() => {
        requireElement(
          container.querySelector<HTMLButtonElement>('.session-pr-trigger'),
        ).click()
      })
    }
    const mergeButton = requireElement(
      container.querySelector<HTMLButtonElement>(
        '.session-pr-row button[aria-label^="Merge PR #42"]',
      ),
    )
    act(() => {
      mergeButton.click()
    })

    const confirmation = container.querySelector<HTMLElement>(
      '[role="alertdialog"]',
    )
    const snapshot = () => ({
      confirmationLabel: confirmation?.querySelector('h2')?.textContent ?? null,
      confirmedHead: confirmation?.querySelector('code')?.textContent ?? null,
      listOpen: isMobile
        ? container.querySelector('.session-pr-list--panel') !== null
        : container
            .querySelector('.session-pr-trigger')
            ?.getAttribute('aria-expanded'),
    })
    expect(snapshot()).toEqual({
      confirmationLabel: 'Merge this pull request?',
      confirmedHead: 'fixture-ready-head',
      listOpen: isMobile ? false : 'false',
    })
  },
)

it('shows merge feedback when a PR uses the session repository fallback', async () => {
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
      pullRequests: [
        {
          number: 46,
          state: 'Open',
          title: 'Update repository metadata',
          headSha: 'fixture-fallback-head',
          ciState: 'Successful',
          isCurrentBranch: false,
        },
      ],
    },
  )

  act(() => {
    requireElement(
      container.querySelector<HTMLButtonElement>('.session-pr-trigger'),
    ).click()
  })
  const mergeButton = requireElement(
    container.querySelector<HTMLButtonElement>(
      'button[aria-label^="Merge PR #46"]',
    ),
  )
  const mergeButtonLabel = mergeButton.getAttribute('aria-label')
  act(() => {
    mergeButton.click()
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
      if (container.querySelector('.async-action-item--failure') !== null)
        return
      await new Promise((resolve) => window.setTimeout(resolve, 0))
    }
  })

  const mergeState = () => ({
    mergeButtonLabel,
    request: fetchMock.mock.calls.map(([url, init]) => ({
      url,
      body: init?.body,
    })),
    feedback: container.querySelector('[role="alert"]')?.textContent,
  })
  expect(mergeState()).toEqual({
    mergeButtonLabel: 'Merge PR #46',
    request: [
      {
        url: '/api/sessions/fixture-session/pull-requests/46/merge',
        body: JSON.stringify({
          owner: 'example',
          repo: 'project',
          expectedHeadSha: 'fixture-fallback-head',
        }),
      },
    ],
    feedback:
      'Merge failed · PR #46A fixture titleThe pull request head changed.RetryDetails',
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
    isCurrentBranch: true,
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
      container.querySelector<HTMLButtonElement>('.merge-band-action'),
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
      if (container.querySelector('.async-action-item--success') !== null)
        return
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
    mergeButton: container.querySelector('.merge-band-action') !== null,
    feedback:
      container.querySelector('.async-action-toast-stack')?.textContent ?? null,
  })
  expect(mergeState()).toEqual({
    summary:
      'PR #42: Untitled pull request, merged, CI passed. 1 more PR from this session',
    mergeButton: true,
    feedback: 'Merged PR #42A fixture title',
  })
})

it('keeps pending merge progress scoped to the PR repository identity', async () => {
  const pendingResponses: Array<(response: Response) => void> = []
  const fetchMock = vi.fn<typeof fetch>(
    () =>
      new Promise((resolve) => {
        pendingResponses.push(resolve)
      }),
  )
  vi.stubGlobal('fetch', fetchMock)
  const firstPullRequest = {
    number: 42,
    state: 'Open' as const,
    owner: 'example-org',
    repo: 'example-repository',
    url: 'https://github.com/example-org/example-repository/pull/42',
    headSha: 'fixture-first-head',
    ciState: 'Successful' as const,
  }
  const secondPullRequest = {
    number: 42,
    state: 'Open' as const,
    owner: 'example-team',
    repo: 'example-repository',
    url: 'https://github.com/example-team/example-repository/pull/42',
    headSha: 'fixture-second-head',
    ciState: 'Successful' as const,
  }
  const baseSession = {
    ...session,
    pullRequests: [firstPullRequest, secondPullRequest],
  }
  renderHeader(
    false,
    vi.fn(() => Promise.resolve(true)),
    baseSession,
  )

  const startMerge = () => {
    const trigger = requireElement(
      container.querySelector<HTMLButtonElement>('.session-pr-trigger'),
    )
    if (trigger.getAttribute('aria-expanded') !== 'true') {
      act(() => {
        trigger.click()
      })
    }
    act(() => {
      const row = container.querySelector<HTMLElement>('.merge-band-row')
      requireElement(
        row?.querySelector<HTMLButtonElement>('.merge-band-action') ?? null,
      ).click()
    })
    const confirmation = requireElement(
      container.querySelector<HTMLElement>('[role="alertdialog"]'),
    )
    act(() => {
      requireElement(
        [...confirmation.querySelectorAll<HTMLButtonElement>('button')].find(
          (button) => button.textContent === 'Confirm merge',
        ) ?? null,
      ).click()
    })
  }
  const openPullRequestList = () => {
    const trigger = requireElement(
      container.querySelector<HTMLButtonElement>('.session-pr-trigger'),
    )
    if (trigger.getAttribute('aria-expanded') !== 'true') {
      act(() => {
        trigger.click()
      })
    }
  }
  const mergeActions = () =>
    Array.from(
      container.querySelectorAll<HTMLElement>('.session-pr-row'),
      (row) => {
        return {
          href: row.querySelector('a')?.getAttribute('href'),
          action:
            row.querySelector('.session-pr-row-state')?.textContent ?? null,
        }
      },
    )

  startMerge()
  openPullRequestList()
  const actionForSecondPullRequest = mergeActions().find(
    ({ href }) => href === secondPullRequest.url,
  )?.action
  const actionForFirstPullRequest = mergeActions().find(
    ({ href }) => href === firstPullRequest.url,
  )?.action
  const secondRow =
    container.querySelectorAll<HTMLElement>('.merge-band-row')[1]
  act(() => {
    requireElement(
      secondRow?.querySelector<HTMLButtonElement>('.merge-band-action') ?? null,
    ).click()
  })
  const secondConfirmation = requireElement(
    container.querySelector<HTMLElement>('[role="alertdialog"]'),
  )
  act(() => {
    requireElement(
      [
        ...secondConfirmation.querySelectorAll<HTMLButtonElement>('button'),
      ].find((button) => button.textContent === 'Confirm merge') ?? null,
    ).click()
  })
  openPullRequestList()
  const actionWhileBothMergesArePending = mergeActions()
  const resolveFirstResponse = pendingResponses[0]
  if (resolveFirstResponse === undefined) {
    throw new Error('Expected the first merge request')
  }

  await act(async () => {
    resolveFirstResponse(Response.json({ result: 'merged' }))
    for (let attempt = 0; attempt < 20; attempt += 1) {
      await new Promise((resolve) => window.setTimeout(resolve, 0))
    }
  })

  openPullRequestList()
  const actionAfterFirstMergeCompletes = mergeActions()
  const resolveSecondResponse = pendingResponses[1]
  if (resolveSecondResponse !== undefined) {
    await act(async () => {
      resolveSecondResponse(Response.json({ result: 'merged' }))
      for (let attempt = 0; attempt < 20; attempt += 1) {
        await new Promise((resolve) => window.setTimeout(resolve, 0))
      }
    })
  }

  const mergeState = () => ({
    actionForSecondPullRequest,
    actionForFirstPullRequest,
    actionWhileBothMergesArePending,
    actionAfterFirstMergeCompletes,
    requestCount: fetchMock.mock.calls.length,
  })
  expect(mergeState()).toEqual({
    actionForSecondPullRequest: 'Merge',
    actionForFirstPullRequest: 'Merging…',
    actionWhileBothMergesArePending: [
      {
        href: 'https://github.com/example-org/example-repository/pull/42',
        action: 'Merging…',
      },
      {
        href: 'https://github.com/example-team/example-repository/pull/42',
        action: 'Merging…',
      },
    ],
    actionAfterFirstMergeCompletes: [
      {
        href: 'https://github.com/example-org/example-repository/pull/42',
        action: '✓ Merged',
      },
      {
        href: 'https://github.com/example-team/example-repository/pull/42',
        action: 'Merging…',
      },
    ],
    requestCount: 2,
  })
})

it('shows mobile merge feedback after confirming from the status panel PR list', async () => {
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
      container.querySelector<HTMLButtonElement>(
        '.mobile-conversation-status-toggle',
      ),
    ).click()
  })
  const mergeButton = requireElement(
    container.querySelector<HTMLButtonElement>(
      '.session-pr-list--panel .session-pr-row button[aria-label^="Merge PR #42"]',
    ),
  )
  act(() => {
    mergeButton.click()
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
      if (container.querySelector('.async-action-item--success') !== null) {
        return
      }
      await new Promise((resolve) => window.setTimeout(resolve, 0))
    }
  })

  const mergeState = () => ({
    request: fetchMock.mock.calls.map(([url]) => url),
    confirmationOpen: container.querySelector('[role="alertdialog"]') !== null,
    feedback: container.querySelector(
      '.async-action-toast-stack .async-action-item',
    )?.textContent,
    feedbackRole: container
      .querySelector('.async-action-toast-stack .async-action-item')
      ?.getAttribute('role'),
    headerHasFeedback: container
      .querySelector('.conversation-header')
      ?.classList.contains('has-session-pr-feedback'),
  })
  expect(mergeState()).toEqual({
    request: ['/api/sessions/fixture-session/pull-requests/42/merge'],
    confirmationOpen: false,
    feedback: 'Merged PR #42A fixture title',
    feedbackRole: 'status',
    headerHasFeedback: false,
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
        ciState: 'Successful',
        isCurrentBranch: true,
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
      const button =
        container.querySelector<HTMLButtonElement>('.merge-band-action')
      return button === null
        ? null
        : { text: button.textContent, disabled: button.disabled }
    })(),
    feedback:
      container.querySelector('.async-action-toast-stack .async-action-item')
        ?.textContent ?? null,
  })

  const pendingState = pendingMergeState()

  await act(async () => {
    resolveMerge(Response.json({ result: 'merged' }))
    for (let attempt = 0; attempt < 20; attempt += 1) {
      if (container.querySelector('.async-action-item--success') !== null)
        return
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
    mergeButton: container.querySelector('.merge-band-action') !== null,
    feedback: container.querySelector(
      '.async-action-toast-stack .async-action-item',
    )?.textContent,
  })
  expect(mergeState()).toEqual({
    confirmedSha: 'sample-head-sha',
    pendingState: {
      confirmationOpen: false,
      mergeButton: { text: 'Merging…', disabled: true },
      feedback: 'Merging PR #42A fixture title',
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
    feedback: 'Merged PR #42A fixture title',
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
        ciState: 'Successful',
        isCurrentBranch: true,
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
      if (container.querySelector('.async-action-item--queued') !== null) return
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
    mergeButton: (() => {
      const button =
        container.querySelector<HTMLButtonElement>('.merge-band-action')
      return button === null
        ? null
        : { label: button.textContent, disabled: button.disabled }
    })(),
    feedback: container.querySelector(
      '.async-action-toast-stack .async-action-item',
    )?.textContent,
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
    mergeButton: { label: 'Queued', disabled: true },
    feedback:
      'Added PR #42 to the merge queueA fixture titleWaiting for checks.',
  })
})

it('shows the current CI reason without retry after checks stop passing', async () => {
  const fetchMock = vi.fn<typeof fetch>(() =>
    Promise.resolve(
      Response.json({ error: 'The merge failed.' }, { status: 409 }),
    ),
  )
  vi.stubGlobal('fetch', fetchMock)
  const pullRequest: NonNullable<Session['pullRequest']> = {
    number: 42,
    state: 'Open',
    owner: 'example-org',
    repo: 'example-repo',
    headSha: 'sample-head-sha',
    ciState: 'Successful',
    isCurrentBranch: true,
  }
  renderHeader(
    false,
    vi.fn(() => Promise.resolve(true)),
    {
      ...session,
      pullRequest,
    },
  )

  act(() => {
    requireElement(
      container.querySelector<HTMLButtonElement>(
        'button[aria-label^="Merge PR #42"]',
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
      if (container.querySelector('.async-action-item--failure') !== null)
        return
      await new Promise((resolve) => window.setTimeout(resolve, 0))
    }
  })

  renderHeader(
    false,
    vi.fn(() => Promise.resolve(true)),
    {
      ...session,
      pullRequest: { ...pullRequest, ciState: 'Failed' },
    },
  )

  const snapshot = () => ({
    status: container.querySelector('.merge-band-heading strong')?.textContent,
    detail: container.querySelector('.merge-band-detail')?.textContent,
    retry: container.querySelector('.merge-band-action')?.textContent ?? null,
  })
  expect(snapshot()).toEqual({
    status: 'Merge failed',
    detail: '#42 CI failed',
    retry: null,
  })
})

it('re-enables merge after a queued pull request receives a new head', async () => {
  const fetchMock = vi.fn<typeof fetch>(() =>
    Promise.resolve(
      Response.json({ result: 'enqueued', message: 'Waiting for checks.' }),
    ),
  )
  vi.stubGlobal('fetch', fetchMock)
  const onAction = vi.fn(() => Promise.resolve(true))
  const queuedPullRequest: NonNullable<Session['pullRequest']> = {
    number: 42,
    state: 'Open',
    owner: 'example-org',
    repo: 'example-repo',
    url: 'https://github.com/example-org/example-repo/pull/42',
    headSha: 'queued-fixture-head',
    ciState: 'Successful',
    isCurrentBranch: true,
  }
  const queuedSession = { ...session, pullRequest: queuedPullRequest }
  renderHeader(false, onAction, queuedSession)

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
      if (container.querySelector('.async-action-item--queued') !== null) return
      await new Promise((resolve) => window.setTimeout(resolve, 0))
    }
  })

  renderHeader(false, onAction, {
    ...queuedSession,
    pullRequest: {
      ...queuedPullRequest,
      headSha: 'new-fixture-head',
    },
  })

  const mergeButton =
    container.querySelector<HTMLButtonElement>('.merge-band-action')
  const mergeState = () => ({
    requestCount: fetchMock.mock.calls.length,
    mergeButton:
      mergeButton === null
        ? null
        : { label: mergeButton.textContent, disabled: mergeButton.disabled },
    queuedFeedback:
      container.querySelector('.async-action-item--queued')?.textContent ??
      null,
  })
  expect(mergeState()).toEqual({
    requestCount: 1,
    mergeButton: { label: 'Merge', disabled: false },
    queuedFeedback:
      'Added PR #42 to the merge queueA fixture titleWaiting for checks.',
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
        ciState: 'Successful',
        isCurrentBranch: true,
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
      if (container.querySelector('.async-action-item--failure') !== null)
        return
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
    mergeButton: container.querySelector('.merge-band-action') !== null,
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
    feedback:
      'Merge failed · PR #42A fixture titleThe pull request head changed.RetryDetails',
  })
})

async function startStaleMergeFailure() {
  let fetchAttempts = 0
  const fetchMock = vi.fn<typeof fetch>(() => {
    fetchAttempts += 1
    return Promise.resolve(
      fetchAttempts === 1
        ? Response.json(
            { error: 'PR #42 head changed since confirmation.' },
            { status: 409 },
          )
        : Response.json({ result: 'merged' }),
    )
  })
  vi.stubGlobal('fetch', fetchMock)
  const pullRequest: NonNullable<Session['pullRequest']> = {
    number: 42,
    state: 'Open',
    owner: 'example-org',
    repo: 'example-repo',
    url: 'https://github.com/example-org/example-repo/pull/42',
    headSha: 'old-fixture-head',
    ciState: 'Successful',
    isCurrentBranch: true,
  }
  const currentSession = { ...session, pullRequest }
  const onAction = vi.fn(() => Promise.resolve(true))
  renderHeader(false, onAction, currentSession)

  act(() => {
    requireElement(
      container.querySelector<HTMLButtonElement>('.merge-band-action'),
    ).click()
  })
  await act(async () => {
    requireElement(
      [...container.querySelectorAll<HTMLButtonElement>('button')].find(
        (button) => button.textContent === 'Confirm merge',
      ) ?? null,
    ).click()
    for (let attempt = 0; attempt < 20; attempt += 1) {
      if (container.querySelector('.async-action-item--failure') !== null)
        return
      await new Promise((resolve) => window.setTimeout(resolve, 0))
    }
  })

  return { fetchMock, onAction, pullRequest }
}

it('keeps a stale merge retry pending until its session returns', async () => {
  const { fetchMock, onAction, pullRequest } = await startStaleMergeFailure()
  renderHeader(false, onAction, {
    ...session,
    id: 'other-fixture-session',
  })
  await act(async () => {
    requireElement(
      container.querySelector<HTMLButtonElement>(
        '.async-action-toast-stack .async-action-controls button',
      ),
    ).click()
    await Promise.resolve()
  })
  const pendingSnapshot = () => ({
    confirmationOpen: container.querySelector('[role="alertdialog"]') !== null,
    failureToast:
      container.querySelector(
        '.async-action-toast-stack .async-action-item--failure',
      )?.textContent ?? null,
    requestBodies: fetchMock.mock.calls.map(([, init]) => init?.body),
  })
  const pending = pendingSnapshot()

  renderHeader(false, onAction, {
    ...session,
    pullRequest: { ...pullRequest, headSha: 'current-fixture-head' },
  })
  await act(async () => {
    for (let attempt = 0; attempt < 20; attempt += 1) {
      if (container.querySelector('[role="alertdialog"]') !== null) return
      await new Promise((resolve) => window.setTimeout(resolve, 0))
    }
  })
  const confirmationSnapshot = () => ({
    confirmationOpen: container.querySelector('[role="alertdialog"]') !== null,
    headSha: container.querySelector('.pull-request-merge-head-sha')
      ?.textContent,
    requestBodies: fetchMock.mock.calls.map(([, init]) => init?.body),
  })
  const confirmation = confirmationSnapshot()

  await act(async () => {
    requireElement(
      [...container.querySelectorAll<HTMLButtonElement>('button')].find(
        (button) => button.textContent === 'Confirm merge',
      ) ?? null,
    ).click()
    for (let attempt = 0; attempt < 20; attempt += 1) {
      if (container.querySelector('.async-action-item--success') !== null)
        return
      await new Promise((resolve) => window.setTimeout(resolve, 0))
    }
  })
  const retrySnapshot = () => ({
    confirmationOpen: container.querySelector('[role="alertdialog"]') !== null,
    requestBodies: fetchMock.mock.calls.map(([, init]) => init?.body),
    successToast: container.querySelector(
      '.async-action-toast-stack .async-action-item--success',
    )?.textContent,
  })
  const retry = retrySnapshot()
  const snapshot = () => ({ pending, confirmation, retry })
  expect(snapshot()).toEqual({
    pending: {
      confirmationOpen: false,
      failureToast:
        'Merge failed · PR #42A fixture titlePR #42 head changed since confirmation.RetryDetails',
      requestBodies: [
        JSON.stringify({
          owner: 'example-org',
          repo: 'example-repo',
          expectedHeadSha: 'old-fixture-head',
        }),
      ],
    },
    confirmation: {
      confirmationOpen: true,
      headSha: 'current-fixture-head',
      requestBodies: [
        JSON.stringify({
          owner: 'example-org',
          repo: 'example-repo',
          expectedHeadSha: 'old-fixture-head',
        }),
      ],
    },
    retry: {
      confirmationOpen: false,
      requestBodies: [
        JSON.stringify({
          owner: 'example-org',
          repo: 'example-repo',
          expectedHeadSha: 'old-fixture-head',
        }),
        JSON.stringify({
          owner: 'example-org',
          repo: 'example-repo',
          expectedHeadSha: 'current-fixture-head',
        }),
      ],
      successToast: 'Merged PR #42A fixture title',
    },
  })
})

it('preserves stale merge retries requested for different sessions', async () => {
  let requestCount = 0
  const fetchMock = vi.fn<typeof fetch>(() => {
    requestCount += 1
    const response =
      requestCount === 1
        ? Response.json(
            { error: 'PR #42 head changed since confirmation.' },
            { status: 409 },
          )
        : requestCount === 2
          ? Response.json(
              { error: 'PR #43 head changed since confirmation.' },
              { status: 409 },
            )
          : Response.json({ result: 'merged' })
    return Promise.resolve(response)
  })
  vi.stubGlobal('fetch', fetchMock)
  const onAction = vi.fn(() => Promise.resolve(true))
  const pullRequestA: NonNullable<Session['pullRequest']> = {
    number: 42,
    state: 'Open',
    owner: 'example-org',
    repo: 'example-repo',
    url: 'https://github.com/example-org/example-repo/pull/42',
    headSha: 'old-first-head',
    ciState: 'Successful',
    isCurrentBranch: true,
  }
  const pullRequestB: NonNullable<Session['pullRequest']> = {
    ...pullRequestA,
    number: 43,
    url: 'https://github.com/example-org/example-repo/pull/43',
    headSha: 'old-second-head',
  }
  const sessionA = { ...session, pullRequest: pullRequestA }
  const sessionB = {
    ...session,
    id: 'second-fixture-session',
    title: 'Second fixture title',
    pullRequest: pullRequestB,
  }

  const submitMerge = async (currentSession: Session, number: number) => {
    renderHeader(false, onAction, currentSession)
    act(() => {
      requireElement(
        container.querySelector<HTMLButtonElement>('.merge-band-action'),
      ).click()
    })
    await act(async () => {
      requireElement(
        [...container.querySelectorAll<HTMLButtonElement>('button')].find(
          (button) => button.textContent === 'Confirm merge',
        ) ?? null,
      ).click()
      for (let attempt = 0; attempt < 20; attempt += 1) {
        const failures = Array.from(
          container.querySelectorAll(
            '.async-action-toast-stack .async-action-item--failure',
          ),
        )
        if (
          failures.some((failure) =>
            failure.textContent.includes(`PR #${String(number)}`),
          )
        )
          return
        await new Promise((resolve) => window.setTimeout(resolve, 0))
      }
    })
  }

  await submitMerge(sessionA, 42)
  await submitMerge(sessionB, 43)
  renderHeader(false, onAction, { ...session, id: 'away-fixture-session' })
  const retryButtonFor = (number: number) => {
    const failedAction = Array.from(
      container.querySelectorAll(
        '.async-action-toast-stack .async-action-item--failure',
      ),
    ).find((item) => item.textContent.includes(`PR #${String(number)}`))
    return requireElement(
      failedAction?.querySelector<HTMLButtonElement>(
        '.async-action-controls button',
      ) ?? null,
    )
  }
  act(() => {
    retryButtonFor(42).click()
    retryButtonFor(43).click()
  })

  renderHeader(false, onAction, {
    ...sessionA,
    pullRequest: { ...pullRequestA, headSha: 'new-first-head' },
  })
  await act(async () => {
    for (let attempt = 0; attempt < 20; attempt += 1) {
      if (container.querySelector('[role="alertdialog"]') !== null) return
      await new Promise((resolve) => window.setTimeout(resolve, 0))
    }
  })
  await act(async () => {
    requireElement(
      [...container.querySelectorAll<HTMLButtonElement>('button')].find(
        (button) => button.textContent === 'Confirm merge',
      ) ?? null,
    ).click()
    for (let attempt = 0; attempt < 20; attempt += 1) {
      if (
        Array.from(
          container.querySelectorAll(
            '.async-action-toast-stack .async-action-item--success',
          ),
        ).some((item) => item.textContent.includes('PR #42'))
      )
        return
      await new Promise((resolve) => window.setTimeout(resolve, 0))
    }
  })

  renderHeader(false, onAction, {
    ...sessionB,
    pullRequest: { ...pullRequestB, headSha: 'new-second-head' },
  })
  await act(async () => {
    for (let attempt = 0; attempt < 20; attempt += 1) {
      if (container.querySelector('[role="alertdialog"]') !== null) return
      await new Promise((resolve) => window.setTimeout(resolve, 0))
    }
  })
  await act(async () => {
    requireElement(
      [...container.querySelectorAll<HTMLButtonElement>('button')].find(
        (button) => button.textContent === 'Confirm merge',
      ) ?? null,
    ).click()
    for (let attempt = 0; attempt < 20; attempt += 1) {
      if (
        Array.from(
          container.querySelectorAll(
            '.async-action-toast-stack .async-action-item--success',
          ),
        ).some((item) => item.textContent.includes('PR #43'))
      )
        return
      await new Promise((resolve) => window.setTimeout(resolve, 0))
    }
  })

  const retrySnapshot = () => ({
    requestBodies: fetchMock.mock.calls.map(([, init]) => init?.body),
    successToasts: Array.from(
      container.querySelectorAll(
        '.async-action-toast-stack .async-action-item--success',
      ),
      (item) => item.textContent,
    ),
  })
  expect(retrySnapshot()).toEqual({
    requestBodies: [
      JSON.stringify({
        owner: 'example-org',
        repo: 'example-repo',
        expectedHeadSha: 'old-first-head',
      }),
      JSON.stringify({
        owner: 'example-org',
        repo: 'example-repo',
        expectedHeadSha: 'old-second-head',
      }),
      JSON.stringify({
        owner: 'example-org',
        repo: 'example-repo',
        expectedHeadSha: 'new-first-head',
      }),
      JSON.stringify({
        owner: 'example-org',
        repo: 'example-repo',
        expectedHeadSha: 'new-second-head',
      }),
    ],
    successToasts: [
      'Merged PR #43Second fixture title',
      'Merged PR #42A fixture title',
    ],
  })
})

it('opens a latest-head confirmation when retry is clicked after the header remounts', async () => {
  const { fetchMock, onAction, pullRequest } = await startStaleMergeFailure()
  renderHeader(false, onAction, {
    ...session,
    id: 'other-fixture-session',
  })
  renderHeader(false, onAction, {
    ...session,
    pullRequest: { ...pullRequest, headSha: 'current-fixture-head' },
  })

  await act(async () => {
    requireElement(
      container.querySelector<HTMLButtonElement>(
        '.async-action-toast-stack .async-action-controls button',
      ),
    ).click()
    for (let attempt = 0; attempt < 20; attempt += 1) {
      if (container.querySelector('[role="alertdialog"]') !== null) return
      await new Promise((resolve) => window.setTimeout(resolve, 0))
    }
  })

  const confirmationSnapshot = () => ({
    confirmationOpen: container.querySelector('[role="alertdialog"]') !== null,
    headSha: container.querySelector('.pull-request-merge-head-sha')
      ?.textContent,
    requestBodies: fetchMock.mock.calls.map(([, init]) => init?.body),
  })
  const confirmation = confirmationSnapshot()

  await act(async () => {
    requireElement(
      [...container.querySelectorAll<HTMLButtonElement>('button')].find(
        (button) => button.textContent === 'Confirm merge',
      ) ?? null,
    ).click()
    for (let attempt = 0; attempt < 20; attempt += 1) {
      if (container.querySelector('.async-action-item--success') !== null)
        return
      await new Promise((resolve) => window.setTimeout(resolve, 0))
    }
  })
  const retrySnapshot = () => ({
    requestBodies: fetchMock.mock.calls.map(([, init]) => init?.body),
    successToast: container.querySelector(
      '.async-action-toast-stack .async-action-item--success',
    )?.textContent,
  })
  const retry = retrySnapshot()
  const snapshot = () => ({ confirmation, retry })
  expect(snapshot()).toEqual({
    confirmation: {
      confirmationOpen: true,
      headSha: 'current-fixture-head',
      requestBodies: [
        JSON.stringify({
          owner: 'example-org',
          repo: 'example-repo',
          expectedHeadSha: 'old-fixture-head',
        }),
      ],
    },
    retry: {
      requestBodies: [
        JSON.stringify({
          owner: 'example-org',
          repo: 'example-repo',
          expectedHeadSha: 'old-fixture-head',
        }),
        JSON.stringify({
          owner: 'example-org',
          repo: 'example-repo',
          expectedHeadSha: 'current-fixture-head',
        }),
      ],
      successToast: 'Merged PR #42A fixture title',
    },
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
        ciState: 'Successful',
        isCurrentBranch: true,
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
    pullRequestLabel: 'PR #42: Untitled pull request, open, CI passed',
    mergeButton: false,
  })
})

it.each([
  {
    description: 'checks are running',
    ciState: 'InProgress' as const,
    isCurrentBranch: true,
    expectedVisible: false,
  },
  {
    description: 'checks are failing',
    ciState: 'Failed' as const,
    isCurrentBranch: true,
    expectedVisible: false,
  },
  {
    description: 'checks passed for this branch',
    ciState: 'Successful' as const,
    isCurrentBranch: true,
    expectedVisible: true,
  },
  {
    description: 'checks passed for another branch',
    ciState: 'Successful' as const,
    isCurrentBranch: false,
    expectedVisible: true,
  },
])('shows the merge band only when $description', (testCase) => {
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
        headSha: 'fixture-head-sha',
        ciState: testCase.ciState,
        isCurrentBranch: testCase.isCurrentBranch,
      },
    },
  )

  const snapshot = () => ({
    inHeader:
      container.querySelector(
        '.conversation-header-actions .merge-band-action',
      ) !== null,
    inBand: container.querySelector('.merge-band-action') !== null,
  })
  expect(snapshot()).toEqual({
    inHeader: false,
    inBand: testCase.expectedVisible,
  })
})

it('keeps Merge out of the mobile header when checks have passed', () => {
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
        headSha: 'fixture-head-sha',
        ciState: 'Successful',
        isCurrentBranch: true,
      },
    },
  )

  const snapshot = () => ({
    headerAction:
      container.querySelector(
        '.mobile-conversation-header-actions .merge-band-action',
      )?.textContent ?? null,
    bandAction:
      container.querySelector('.merge-band-action')?.textContent ?? null,
  })
  expect(snapshot()).toEqual({ headerAction: null, bandAction: 'Merge' })
})
