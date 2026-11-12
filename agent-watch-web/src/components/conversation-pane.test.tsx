import { act, type ReactNode, useState } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { userEvent } from 'vitest/browser'

import { getCommandPaletteShortcut } from '#command-palette-shortcut'
import { renderConversationMessage } from '#components/conversation-message'
import { ConversationMessageRow } from '#components/conversation-message-row'
import { ConversationPane } from '#components/conversation-pane'
import { ConversationPaneView } from '#components/conversation-pane-view'
import { InboxConversationPanes } from '#components/inbox-conversation-panes'
import { formatMessageTime } from '#components/message-time'
import { SessionStatusPanel } from '#components/session-status-panel'
import { useConversationStatusPanel } from '#components/use-conversation-status-panel'
import type {
  BackgroundTask,
  ChatItem,
  Session,
  SessionActions,
  SessionTreeNode,
} from '#types'

type PendingPermissionAction = NonNullable<
  Parameters<typeof ConversationPaneView>[0]['pendingPermissionActions']
>[string]

const { optimizeImageAttachmentMock } = vi.hoisted(() => ({
  optimizeImageAttachmentMock: vi.fn<(file: File) => Promise<File>>(),
}))

vi.mock('#lib/image-attachment', () => ({
  optimizeImageAttachment: optimizeImageAttachmentMock,
}))

vi.mock('#components/conversation-message', async (importOriginal) => {
  const original =
    await importOriginal<typeof import('#components/conversation-message')>()
  return {
    ...original,
    renderConversationMessage: vi.fn(original.renderConversationMessage),
  }
})

const session: Session = {
  id: 'sample-session',
  category: 'needs_action',
  isPaused: false,
  kind: 'Question',
  title: 'Choose a retry policy',
  preview: null,
  elapsedSeconds: 0,
  unread: true,
  task: null,
  repo: 'example/service',
  project: 'service refresh',
  parent: null,
  ancestorSessionIds: [],
  pullRequest: null,
}

const actions: SessionActions = {
  message: true,
  answerQuestion: true,
  permission: { approve: true, reject: true },
  interrupt: true,
  openInTerminal: true,
}

const codexTerminalActions: SessionActions = {
  message: true,
  answerQuestion: false,
  permission: { approve: false, reject: false },
  interrupt: true,
  openInTerminal: true,
}

const questionPrompt = {
  header: 'Cache policy',
  question: 'Which cache window should be used?',
  choices: [{ label: 'Window A' }, { label: 'Window B' }],
  multi_select: false,
  answer: null,
}

const question: ChatItem = {
  type: 'question',
  requestId: 'ask-request-main',
  questions: [questionPrompt],
}

const proposal: ChatItem = {
  type: 'proposal',
  seq: 3,
  title: 'When should the setting take effect?',
  summary: 'Choose how the change is introduced.',
  options: [
    {
      label: 'Use the existing flow',
      description: 'Explain the change before applying it.',
      pros: ['Small change'],
      cons: ['Adds one step'],
      recommended: true,
    },
    {
      label: 'Show the choice earlier',
      description: null,
      pros: [],
      cons: [],
      recommended: false,
    },
  ],
  answer: null,
  resolved: false,
}

function messagesResponse(
  messages: ChatItem[],
  backgroundTasks: BackgroundTask[] = [],
  metadata: {
    start: number
    total: number
    decisions?: Extract<ChatItem, { type: 'decision' }>[]
    hasUnansweredProposal?: boolean
    activeQuestionIndex?: number | null
  } = { start: 0, total: messages.length },
) {
  const activeQuestionIndex = messages.findLastIndex(
    (message) => message.type === 'question',
  )
  return {
    messages,
    backgroundTasks,
    start: metadata.start,
    total: metadata.total,
    decisions:
      metadata.decisions ??
      messages.filter(
        (message): message is Extract<ChatItem, { type: 'decision' }> =>
          message.type === 'decision',
      ),
    hasUnansweredProposal:
      metadata.hasUnansweredProposal ??
      messages.some(
        (message) =>
          message.type === 'proposal' &&
          message.answer === null &&
          !message.resolved,
      ),
    activeQuestionIndex:
      metadata.activeQuestionIndex === undefined
        ? activeQuestionIndex < 0
          ? null
          : metadata.start + activeQuestionIndex
        : metadata.activeQuestionIndex,
  }
}

function jsonResponse(body: unknown, status = 200) {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'Content-Type': 'application/json' },
  })
}

function requestUrl(input: RequestInfo | URL) {
  if (typeof input === 'string') return input
  return input instanceof URL ? input.href : input.url
}

function setPageVisibility(visibilityState: DocumentVisibilityState) {
  Object.defineProperty(document, 'visibilityState', {
    configurable: true,
    value: visibilityState,
  })
}

const permission: ChatItem = {
  type: 'permission',
  requestId: 'permission-request-main',
  tool: 'Shell',
  description: 'Run the formatter on changed files.',
}

const otherPermission: ChatItem = {
  type: 'permission',
  requestId: 'permission-request-subagent',
  tool: 'Subagent sample-agent',
  description: 'Waiting for permission approval.',
}
let container: HTMLDivElement
let root: Root | null = null
const originalExecCommand = Object.getOwnPropertyDescriptor(
  document,
  'execCommand',
)
const originalClipboard = Object.getOwnPropertyDescriptor(
  navigator,
  'clipboard',
)
const originalVisibilityState = Object.getOwnPropertyDescriptor(
  document,
  'visibilityState',
)
type ActionHandler = (
  action: string,
  payload?: Record<string, unknown>,
) => Promise<boolean>

function requireElement<T extends Element>(element: T | null): T {
  if (element === null) throw new Error('Expected element was not rendered')
  return element
}

async function waitForState(condition: () => boolean) {
  const deadline = Date.now() + 1000
  await act(async () => {
    while (!condition() && Date.now() < deadline) {
      await new Promise((resolve) => window.setTimeout(resolve, 10))
    }
  })
  return condition()
}

function terminalFeedback() {
  return Array.from(
    container.querySelectorAll('.conversation-terminal-action-status'),
    (element) => element.textContent,
  ).filter((text) => text !== '')
}

function setTextareaValue(textarea: HTMLTextAreaElement, value: string) {
  act(() => {
    const valueSetter = Reflect.getOwnPropertyDescriptor(
      textarea.constructor.prototype,
      'value',
    )?.set
    if (valueSetter !== undefined) {
      Reflect.apply(valueSetter, textarea, [value])
    }
    textarea.dispatchEvent(new Event('input', { bubbles: true }))
  })
}

function expectDraftRetained(
  textarea: HTMLTextAreaElement,
  selector: string,
  expectedValue: string,
) {
  const currentTextarea = container.querySelector<HTMLTextAreaElement>(selector)
  const snapshot = () => ({
    sameTextarea: textarea === currentTextarea,
    value: currentTextarea?.value,
  })
  expect(snapshot()).toEqual({
    sameTextarea: true,
    value: expectedValue,
  })
}

function renderView(
  onAction: ActionHandler,
  viewSession: Session = session,
  onSelectSession: (id: string) => void = () => undefined,
  viewMessages: readonly ChatItem[] = [],
  viewActions: SessionActions | null = actions,
  isMobile = false,
  viewBackgroundTasks: readonly BackgroundTask[] = [],
  viewSessions: readonly Session[] = [],
  statusPanelOpen = false,
  onToggleStatusPanel?: () => void,
  pendingPermissionAction?: PendingPermissionAction | null,
  today: {
    isToday: boolean
    onToggleToday: (isToday: boolean) => void
  } = { isToday: false, onToggleToday: () => undefined },
  mobileStatusPanel?: ReactNode,
) {
  root ??= createRoot(container)
  act(() => {
    root?.render(
      <ConversationPaneView
        session={viewSession}
        sessions={viewSessions}
        messages={viewMessages}
        backgroundTasks={viewBackgroundTasks}
        actions={viewActions}
        pendingPermissionActions={
          pendingPermissionAction === undefined ||
          pendingPermissionAction === null
            ? {}
            : {
                [pendingPermissionAction.requestId]: pendingPermissionAction,
              }
        }
        loading={false}
        error={null}
        onAction={onAction}
        onSelectSession={onSelectSession}
        isMobile={isMobile}
        isToday={today.isToday}
        onToggleToday={today.onToggleToday}
        statusPanelOpen={statusPanelOpen}
        mobileStatusPanel={mobileStatusPanel}
        {...(onToggleStatusPanel === undefined ? {} : { onToggleStatusPanel })}
      />,
    )
  })
}

function renderConversationMessages(
  messages: readonly ChatItem[],
  start: number,
) {
  root ??= createRoot(container)
  act(() => {
    root?.render(
      <ConversationPaneView
        session={session}
        messages={messages}
        start={start}
        actions={actions}
        loading={false}
        error={null}
        onAction={() => Promise.resolve(true)}
        onSelectSession={() => undefined}
      />,
    )
  })
}

function treeSession(
  id: string,
  title: string,
  parent: Session | null = null,
): Session {
  return {
    ...session,
    id,
    title,
    hasSessionTree: true,
    ancestorSessionIds:
      parent === null ? [] : [...parent.ancestorSessionIds, parent.id],
    parent: parent === null ? null : { id: parent.id, title: parent.title },
  }
}

function treeNode(
  id: string,
  parentId: string | null,
  title: string,
): SessionTreeNode {
  return {
    id,
    parentId,
    category: 'in_progress',
    isActive: true,
    kind: null,
    title,
    preview: null,
    elapsedSeconds: 0,
    task: null,
    repo: session.repo,
    decisionCount: 0,
    pullRequest: null,
  }
}

function inboxFetch(respondToTreeRequest: (url: string) => Promise<Response>) {
  return vi.fn<typeof fetch>((input) => {
    const url = requestUrl(input)
    if (url.endsWith('/messages?limit=100')) {
      return Promise.resolve(jsonResponse(messagesResponse([])))
    }
    if (url.endsWith('/actions')) return Promise.resolve(jsonResponse(actions))
    if (url.endsWith('/tree')) return respondToTreeRequest(url)
    return Promise.resolve(jsonResponse({}))
  })
}

function renderInboxPanes(
  sessions: readonly Session[],
  selectedSession: Session,
  sessionRefreshRevisions: ReadonlyMap<string, number> = new Map(),
) {
  root ??= createRoot(container)
  const inboxProps = {
    sessions,
    selectedId: selectedSession.id,
    loaded: true,
    error: null,
    connectionError: null,
    lastUpdatedAt: 0,
    showAllInProgress: false,
    onShowMore: () => undefined,
    onSelect: () => undefined,
    laterExpanded: false,
    onToggleLater: () => undefined,
    onLater: () => undefined,
    onToggleToday: () => undefined,
    todaySessions: [],
    onSelectToday: () => undefined,
    inProgressExpanded: false,
    onToggleInProgress: () => undefined,
    doneExpanded: false,
    onToggleDone: () => undefined,
  }
  act(() => {
    root?.render(
      <InboxConversationPanes
        inboxProps={inboxProps}
        sessions={sessions}
        selectedSession={selectedSession}
        sessionRefreshRevisions={sessionRefreshRevisions}
        isMobile={false}
        showMobileConversation={false}
        statusPanelOpen
        onBack={() => undefined}
        onLater={() => undefined}
        onCloseSession={() => undefined}
        onSelectSession={() => undefined}
      />,
    )
  })
}

it('refreshes a session tree only when a tree member changes', async () => {
  const rootSession = treeSession('root-session', 'Root session')
  const childSession = treeSession(
    'child-session',
    'Child session',
    rootSession,
  )
  const unrelatedSession = treeSession('unrelated-session', 'Other session')
  const sessions = [rootSession, childSession, unrelatedSession]
  const addedSession = treeSession(
    'added-session',
    'Added session',
    rootSession,
  )
  const tree = [
    treeNode(rootSession.id, null, rootSession.title),
    treeNode(childSession.id, rootSession.id, childSession.title),
  ]
  let treeRequestCount = 0
  const fetcher = inboxFetch(() => {
    treeRequestCount += 1
    return Promise.resolve(
      jsonResponse({
        revision: treeRequestCount,
        sessions:
          treeRequestCount === 1
            ? tree
            : [
                ...tree,
                treeNode(addedSession.id, rootSession.id, addedSession.title),
              ],
      }),
    )
  })
  vi.stubGlobal('fetch', fetcher)

  renderInboxPanes(
    sessions,
    rootSession,
    new Map([
      [rootSession.id, 1],
      [childSession.id, 1],
    ]),
  )
  await waitForState(
    () => container.querySelector('#session-tree-row-child-session') !== null,
  )

  renderInboxPanes(
    sessions,
    rootSession,
    new Map([
      [rootSession.id, 1],
      [childSession.id, 1],
      [unrelatedSession.id, 2],
    ]),
  )
  const requestsAfterUnrelatedChange = fetcher.mock.calls.map(([input]) =>
    requestUrl(input),
  )

  renderInboxPanes(
    [...sessions, addedSession],
    rootSession,
    new Map([
      [rootSession.id, 1],
      [childSession.id, 1],
      [unrelatedSession.id, 2],
      [addedSession.id, 3],
    ]),
  )
  await waitForState(
    () =>
      fetcher.mock.calls.some(([input]) =>
        requestUrl(input).endsWith('/tree'),
      ) &&
      fetcher.mock.calls.filter(([input]) =>
        requestUrl(input).endsWith('/tree'),
      ).length === 2,
  )
  await waitForState(
    () => container.querySelector('#session-tree-row-added-session') !== null,
  )

  const snapshot = () => ({
    requestsAfterUnrelatedChange,
    requests: fetcher.mock.calls.map(([input]) => requestUrl(input)),
    addedChildRow:
      container.querySelector('#session-tree-row-added-session') !== null,
  })
  expect(snapshot()).toEqual({
    requestsAfterUnrelatedChange: [
      '/api/sessions/root-session/tree',
      '/api/sessions/root-session/messages?limit=100',
      '/api/sessions/root-session/actions',
    ],
    requests: [
      '/api/sessions/root-session/tree',
      '/api/sessions/root-session/messages?limit=100',
      '/api/sessions/root-session/actions',
      '/api/sessions/root-session/tree',
    ],
    addedChildRow: true,
  })
})

it('uses existing descendant revisions for the initial tree request', async () => {
  const rootSession = treeSession('root-session', 'Root session')
  const childSession = treeSession(
    'child-session',
    'Child session',
    rootSession,
  )
  const sessions = [rootSession, childSession]
  const tree = [
    treeNode(rootSession.id, null, rootSession.title),
    treeNode(childSession.id, rootSession.id, childSession.title),
  ]
  const fetcher = inboxFetch(() =>
    Promise.resolve(jsonResponse({ revision: 1, sessions: tree })),
  )
  vi.stubGlobal('fetch', fetcher)

  renderInboxPanes(
    sessions,
    rootSession,
    new Map([
      [rootSession.id, 1],
      [childSession.id, 2],
    ]),
  )
  await waitForState(
    () => container.querySelector('#session-tree-row-child-session') !== null,
  )

  const snapshot = () => ({
    requests: fetcher.mock.calls.map(([input]) => requestUrl(input)),
    childRow:
      container.querySelector('#session-tree-row-child-session') !== null,
  })
  expect(snapshot()).toEqual({
    requests: [
      '/api/sessions/root-session/tree',
      '/api/sessions/root-session/messages?limit=100',
      '/api/sessions/root-session/actions',
    ],
    childRow: true,
  })
})

it('refetches the initial tree when a member changes while it loads', async () => {
  const rootSession = treeSession('root-session', 'Root session')
  const childSession = treeSession(
    'child-session',
    'Child session',
    rootSession,
  )
  const sessions = [rootSession, childSession]
  const tree = [
    treeNode(rootSession.id, null, rootSession.title),
    treeNode(childSession.id, rootSession.id, childSession.title),
  ]
  let resolveInitialTree: ((response: Response) => void) | undefined
  const initialTreeResponse = new Promise<Response>((resolve) => {
    resolveInitialTree = resolve
  })
  let treeRequestCount = 0
  const fetcher = inboxFetch(() => {
    treeRequestCount += 1
    return treeRequestCount === 1
      ? initialTreeResponse
      : Promise.resolve(jsonResponse({ revision: 2, sessions: tree }))
  })
  vi.stubGlobal('fetch', fetcher)

  renderInboxPanes(
    sessions,
    rootSession,
    new Map([
      [rootSession.id, 1],
      [childSession.id, 1],
    ]),
  )
  await waitForState(() => treeRequestCount === 1)

  renderInboxPanes(
    sessions,
    rootSession,
    new Map([
      [rootSession.id, 1],
      [childSession.id, 2],
    ]),
  )
  const requestsWhileInitialTreeIsPending = treeRequestCount

  await act(async () => {
    resolveInitialTree?.(jsonResponse({ revision: 1, sessions: tree }))
    await initialTreeResponse
    for (let index = 0; index < 12; index += 1) await Promise.resolve()
  })
  await waitForState(() => treeRequestCount === 2)
  await waitForState(
    () => container.querySelector('#session-tree-row-child-session') !== null,
  )

  const snapshot = () => ({
    requestsWhileInitialTreeIsPending,
    treeRequestCount,
    requests: fetcher.mock.calls.map(([input]) => requestUrl(input)),
  })
  expect(snapshot()).toEqual({
    requestsWhileInitialTreeIsPending: 1,
    treeRequestCount: 2,
    requests: [
      '/api/sessions/root-session/tree',
      '/api/sessions/root-session/messages?limit=100',
      '/api/sessions/root-session/actions',
      '/api/sessions/root-session/tree',
    ],
  })
})

function starredToggleSnapshot(onToggleToday: ReturnType<typeof vi.fn>) {
  const toggle = container.querySelector<HTMLButtonElement>(
    '.mobile-conversation-today',
  )
  return {
    label: toggle?.getAttribute('aria-label'),
    pressed: toggle?.getAttribute('aria-pressed'),
    title: toggle?.getAttribute('title'),
    calls: onToggleToday.mock.calls,
  }
}

it('toggles Starred from the iPhone conversation header', () => {
  const onToggleToday = vi.fn()
  renderView(
    () => Promise.resolve(true),
    session,
    undefined,
    [],
    actions,
    true,
    [],
    [],
    false,
    undefined,
    null,
    { isToday: false, onToggleToday },
  )

  const toggle = container.querySelector<HTMLButtonElement>(
    '.mobile-conversation-today',
  )
  act(() => {
    toggle?.dispatchEvent(new MouseEvent('click', { bubbles: true }))
  })

  expect(starredToggleSnapshot(onToggleToday)).toEqual({
    label: 'Add to Starred',
    pressed: 'false',
    title: 'Add to Starred',
    calls: [[true]],
  })
})

it('labels a Starred iPhone session for removal', () => {
  const onToggleToday = vi.fn()
  renderView(
    () => Promise.resolve(true),
    session,
    undefined,
    [],
    actions,
    true,
    [],
    [],
    false,
    undefined,
    null,
    { isToday: true, onToggleToday },
  )

  expect(starredToggleSnapshot(onToggleToday)).toEqual({
    label: 'Remove from Starred',
    pressed: 'true',
    title: 'Remove from Starred',
    calls: [],
  })
})

it('sets the primary linked task from desktop and mobile headers', async () => {
  const primaryTask = {
    id: 'sample-primary-task',
    number: 2048,
    title: 'Improve request reliability',
    priority: 1,
  }
  const secondaryTask = {
    id: 'sample-secondary-task',
    number: 2046,
    title: 'Add request tracing',
    priority: 4,
  }
  const sessionWithLinkedTasks = {
    ...session,
    task: primaryTask,
    tasks: [primaryTask, secondaryTask],
  }
  const onAction = vi.fn(() => Promise.resolve(true))

  for (const isMobile of [false, true]) {
    renderView(
      onAction,
      sessionWithLinkedTasks,
      undefined,
      [],
      actions,
      isMobile,
      [],
      [sessionWithLinkedTasks],
    )
    await act(async () => {
      container.querySelector('summary')?.click()
      await new Promise<void>((resolve) => window.setTimeout(resolve, 0))
    })
    await act(async () => {
      document.body
        .querySelector<HTMLButtonElement>('.session-task-links-make-primary')
        ?.click()
      await new Promise<void>((resolve) => window.setTimeout(resolve, 0))
    })
  }

  expect(onAction.mock.calls).toEqual([
    ['primary-task', { taskId: 'sample-secondary-task' }],
    ['primary-task', { taskId: 'sample-secondary-task' }],
  ])
})

it('places the session ID copy control beside the iPhone title', () => {
  renderView(() => Promise.resolve(true), session, undefined, [], actions, true)

  const titleRow = container.querySelector(
    '.mobile-conversation-heading .session-heading-title-row',
  )
  const copyButton = titleRow?.querySelector<HTMLButtonElement>(
    '.conversation-copy-session-id',
  )

  const snapshot = () => ({
    title: titleRow?.querySelector('h1')?.textContent,
    copyButton: {
      ariaLabel: copyButton?.getAttribute('aria-label'),
      title: copyButton?.getAttribute('title'),
      compact: copyButton?.getAttribute('data-compact') ?? null,
    },
    compactCopyControl: container.querySelector(
      '.conversation-copy-session-id[data-compact="true"]',
    ),
  })

  expect(snapshot()).toEqual({
    title: 'Choose a retry policy',
    copyButton: {
      ariaLabel: 'Copy session ID',
      title: 'Copy session ID',
      compact: null,
    },
    compactCopyControl: null,
  })
})

function dispatchKey(
  target: EventTarget,
  key: string,
  options: KeyboardEventInit = {},
) {
  const event = new KeyboardEvent('keydown', {
    key,
    bubbles: true,
    cancelable: true,
    ...options,
  })
  target.dispatchEvent(event)
  return event
}

function dispatchKeyInAct(
  target: EventTarget,
  key: string,
  options: KeyboardEventInit = {},
) {
  const event = new KeyboardEvent('keydown', {
    key,
    bubbles: true,
    cancelable: true,
    ...options,
  })
  act(() => {
    target.dispatchEvent(event)
  })
  return event
}

beforeEach(() => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true)
  optimizeImageAttachmentMock.mockReset()
  optimizeImageAttachmentMock.mockImplementation((file) =>
    Promise.resolve(file),
  )
  window.localStorage.clear()
  container = document.createElement('div')
  document.body.append(container)
})

it('loads and shows the session tree from the tree endpoint', async () => {
  const treeSession = { ...session, hasSessionTree: true }
  const tree: SessionTreeNode[] = [
    {
      id: session.id,
      parentId: null,
      category: 'in_progress',
      isActive: true,
      kind: null,
      title: session.title,
      preview: 'Delegating the task',
      elapsedSeconds: 20,
      task: null,
      repo: session.repo,
      decisionCount: 0,
      pullRequest: null,
    },
    {
      id: 'child-session',
      parentId: session.id,
      category: 'needs_action',
      isActive: true,
      kind: 'Question',
      title: 'Child session',
      preview: 'Waiting for a decision',
      elapsedSeconds: 8,
      task: null,
      repo: session.repo,
      decisionCount: 0,
      pullRequest: null,
    },
  ]
  const fetcher = vi.fn<typeof fetch>((input) => {
    const url = requestUrl(input)
    if (url.endsWith('/messages?limit=100')) {
      return Promise.resolve(jsonResponse(messagesResponse([])))
    }
    if (url.endsWith('/actions')) {
      return Promise.resolve(jsonResponse(actions))
    }
    if (url.endsWith('/tree')) {
      return Promise.resolve(jsonResponse({ revision: 1, sessions: tree }))
    }
    return Promise.resolve(jsonResponse({}))
  })
  vi.stubGlobal('fetch', fetcher)
  root = createRoot(container)
  act(() => {
    root?.render(
      <ConversationPane
        session={treeSession}
        refreshKey={0}
        sessionTreeRefreshKey={0}
        statusPanelOpen
        onSelectSession={() => undefined}
      />,
    )
  })

  await waitForState(
    () =>
      container.querySelector('#session-status-tab-tree') !== null &&
      container.querySelector('#session-tree-row-child-session') !== null,
  )
  const snapshot = () => ({
    treeRequest: fetcher.mock.calls
      .map(([input]) => requestUrl(input))
      .filter((url) => url.endsWith('/tree')),
    tab: container.querySelector('[role="tab"]')?.textContent,
    countTone: container.querySelector('.session-status-panel-count')
      ?.className,
    rows: Array.from(
      container.querySelectorAll('.session-tree-title'),
      (row) => row.textContent,
    ),
  })
  expect(snapshot()).toEqual({
    treeRequest: ['/api/sessions/sample-session/tree'],
    tab: 'Tree1',
    countTone: 'session-status-panel-count session-status-panel-count--warning',
    rows: ['Choose a retry policy', 'Child session'],
  })
})

it('shows linked pull requests alongside partial lookup errors', async () => {
  const taskSession: Session = {
    ...session,
    task: {
      id: 'synthetic-task-id',
      number: 2048,
      title: 'Show linked work in the sidebar',
      priority: null,
    },
  }
  const fetcher = vi.fn<typeof fetch>((input) => {
    const url = requestUrl(input)
    if (url.endsWith('/messages?limit=100')) {
      return Promise.resolve(jsonResponse(messagesResponse([])))
    }
    if (url.endsWith('/actions')) {
      return Promise.resolve(jsonResponse(actions))
    }
    if (url.endsWith('/pull-requests')) {
      return Promise.resolve(
        jsonResponse({
          pullRequests: [
            {
              number: 2048,
              owner: 'example',
              repo: 'project',
              title: 'Show linked work in the sidebar',
              url: 'https://github.com/example/project/pull/2048',
              headSha: 'confirmed-head',
              state: 'Open',
              ciState: 'Successful',
              reviewStatus: 'done',
            },
          ],
          error: 'Some linked pull requests could not be loaded.',
        }),
      )
    }
    return Promise.resolve(jsonResponse({}))
  })
  vi.stubGlobal('fetch', fetcher)
  root = createRoot(container)
  act(() => {
    root?.render(
      <ConversationPane
        session={taskSession}
        sessionTreeRefreshKey={0}
        refreshKey={0}
        statusPanelOpen
        onSelectSession={() => undefined}
      />,
    )
  })

  await waitForState(
    () => container.querySelector('.pull-request-list-title') !== null,
  )
  const snapshot = () => ({
    requests: fetcher.mock.calls
      .map(([input]) => requestUrl(input))
      .filter((url) => url.endsWith('/pull-requests')),
    tabs: Array.from(container.querySelectorAll('[role="tab"]'), (tab) => ({
      label: tab.textContent,
      selected: tab.getAttribute('aria-selected'),
    })),
    pullRequests: Array.from(
      container.querySelectorAll('.pull-request-list-item'),
      (item) => ({
        title: item.querySelector('.pull-request-list-title')?.textContent,
        href: item
          .querySelector('.pull-request-list-title')
          ?.getAttribute('href'),
        status: item.querySelector('.pull-request-list-meta')?.textContent,
      }),
    ),
    error: container.querySelector('.pull-request-list-error')?.textContent,
  })

  expect(snapshot()).toEqual({
    requests: ['/api/sessions/sample-session/pull-requests'],
    tabs: [{ label: 'PRs1', selected: 'true' }],
    pullRequests: [
      {
        title: '#2048Show linked work in the sidebar',
        href: 'https://github.com/example/project/pull/2048',
        status: 'OpenCI: PassingReview: Done',
      },
    ],
    error: 'Some linked pull requests could not be loaded.',
  })
})

it('pauses pull request refresh while hidden and refreshes when visible again', async () => {
  vi.useFakeTimers()
  setPageVisibility('hidden')
  const taskSession: Session = {
    ...session,
    task: {
      id: 'synthetic-task-id',
      number: 2048,
      title: 'Show linked work in the sidebar',
      priority: null,
    },
  }
  const fetcher = vi.fn<typeof fetch>((input) => {
    const url = requestUrl(input)
    if (url.endsWith('/messages?limit=100')) {
      return Promise.resolve(jsonResponse(messagesResponse([])))
    }
    if (url.endsWith('/actions')) {
      return Promise.resolve(jsonResponse(actions))
    }
    if (url.endsWith('/pull-requests')) {
      return Promise.resolve(jsonResponse({ pullRequests: [], error: null }))
    }
    return Promise.resolve(jsonResponse({}))
  })
  vi.stubGlobal('fetch', fetcher)
  root = createRoot(container)
  act(() => {
    root?.render(
      <ConversationPane
        session={taskSession}
        sessionTreeRefreshKey={0}
        refreshKey={0}
        statusPanelOpen
        onSelectSession={() => undefined}
      />,
    )
  })
  await act(async () => {
    for (let index = 0; index < 12; index += 1) await Promise.resolve()
  })
  const pullRequestRequests = () =>
    fetcher.mock.calls
      .map(([input]) => requestUrl(input))
      .filter((url) => url.endsWith('/pull-requests'))

  const hiddenBeforeFirstRequest = pullRequestRequests()
  setPageVisibility('visible')
  await act(async () => {
    document.dispatchEvent(new Event('visibilitychange'))
    for (let index = 0; index < 12; index += 1) await Promise.resolve()
  })
  const visibleAfterResume = pullRequestRequests()
  setPageVisibility('hidden')
  await act(async () => {
    document.dispatchEvent(new Event('visibilitychange'))
    await Promise.resolve()
  })
  await act(async () => {
    await vi.advanceTimersByTimeAsync(60_000)
  })
  const hiddenAfterRefreshInterval = pullRequestRequests()
  setPageVisibility('visible')
  await act(async () => {
    document.dispatchEvent(new Event('visibilitychange'))
    for (let index = 0; index < 12; index += 1) await Promise.resolve()
  })

  const snapshot = () => ({
    hiddenBeforeFirstRequest,
    visibleAfterResume,
    hiddenAfterRefreshInterval,
    visibleAfterSecondResume: pullRequestRequests(),
  })
  expect(snapshot()).toEqual({
    hiddenBeforeFirstRequest: [],
    visibleAfterResume: ['/api/sessions/sample-session/pull-requests'],
    hiddenAfterRefreshInterval: ['/api/sessions/sample-session/pull-requests'],
    visibleAfterSecondResume: [
      '/api/sessions/sample-session/pull-requests',
      '/api/sessions/sample-session/pull-requests',
    ],
  })
})

it('shows a tree request error instead of hiding the tree tab', async () => {
  const treeSession = { ...session, hasSessionTree: true }
  const fetcher = vi.fn<typeof fetch>((input) => {
    const url = requestUrl(input)
    if (url.endsWith('/messages?limit=100')) {
      return Promise.resolve(jsonResponse(messagesResponse([])))
    }
    if (url.endsWith('/actions')) {
      return Promise.resolve(jsonResponse(actions))
    }
    if (url.endsWith('/tree')) {
      return Promise.resolve(jsonResponse({ error: 'Sample tree error' }, 503))
    }
    return Promise.resolve(jsonResponse({}))
  })
  vi.stubGlobal('fetch', fetcher)
  root = createRoot(container)
  act(() => {
    root?.render(
      <ConversationPane
        session={treeSession}
        refreshKey={0}
        sessionTreeRefreshKey={0}
        statusPanelOpen
        onSelectSession={() => undefined}
      />,
    )
  })

  await waitForState(() => container.querySelector('[role="alert"]') !== null)
  const snapshot = () => ({
    treeRequest: fetcher.mock.calls
      .map(([input]) => requestUrl(input))
      .filter((url) => url.endsWith('/tree')),
    tab: container.querySelector('[role="tab"]')?.textContent,
    alert: container.querySelector('[role="alert"]')?.textContent,
  })
  expect(snapshot()).toEqual({
    treeRequest: ['/api/sessions/sample-session/tree'],
    tab: 'Tree',
    alert: 'Could not load the session tree: Sample tree error',
  })
})

it('shows an error when the returned tree omits the selected session', async () => {
  const treeSession = { ...session, hasSessionTree: true }
  const fetcher = vi.fn<typeof fetch>((input) => {
    const url = requestUrl(input)
    if (url.endsWith('/messages?limit=100')) {
      return Promise.resolve(jsonResponse(messagesResponse([])))
    }
    if (url.endsWith('/actions')) {
      return Promise.resolve(jsonResponse(actions))
    }
    if (url.endsWith('/tree')) {
      return Promise.resolve(
        jsonResponse({
          revision: 1,
          sessions: [
            {
              id: 'other-session',
              parentId: null,
              category: 'in_progress',
              isActive: true,
              kind: null,
              title: 'Other session',
              preview: null,
              elapsedSeconds: 0,
              task: null,
              repo: session.repo,
              decisionCount: 0,
              pullRequest: null,
            },
            {
              id: 'other-child-session',
              parentId: 'other-session',
              category: 'in_progress',
              isActive: true,
              kind: null,
              title: 'Other child',
              preview: null,
              elapsedSeconds: 0,
              task: null,
              repo: session.repo,
              decisionCount: 0,
              pullRequest: null,
            },
          ],
        }),
      )
    }
    return Promise.resolve(jsonResponse({}))
  })
  vi.stubGlobal('fetch', fetcher)
  root = createRoot(container)
  act(() => {
    root?.render(
      <ConversationPane
        session={treeSession}
        refreshKey={0}
        sessionTreeRefreshKey={0}
        statusPanelOpen
        onSelectSession={() => undefined}
      />,
    )
  })

  await waitForState(() => container.querySelector('[role="alert"]') !== null)
  const snapshot = () => ({
    treeRequest: fetcher.mock.calls
      .map(([input]) => requestUrl(input))
      .filter((url) => url.endsWith('/tree')),
    tab: container.querySelector('[role="tab"]')?.textContent,
    alert: container.querySelector('[role="alert"]')?.textContent,
    rows: container.querySelectorAll('.session-tree-row').length,
  })
  expect(snapshot()).toEqual({
    treeRequest: ['/api/sessions/sample-session/tree'],
    tab: 'Tree',
    alert:
      'Could not load the session tree: The session tree does not contain the selected session.',
    rows: 0,
  })
})

it('reuses cached tree state when navigating within the same tree', async () => {
  const rootSession = treeSession('root-session', 'Root session')
  const childSession = treeSession(
    'child-session',
    'Child session',
    rootSession,
  )
  const grandchildSession = treeSession(
    'grandchild-session',
    'Grandchild session',
    childSession,
  )
  const siblingSession = treeSession(
    'sibling-session',
    'Sibling session',
    rootSession,
  )
  const sessions = [
    rootSession,
    childSession,
    grandchildSession,
    siblingSession,
  ]
  const tree = [
    treeNode(rootSession.id, null, rootSession.title),
    treeNode(childSession.id, rootSession.id, childSession.title),
    treeNode(grandchildSession.id, childSession.id, grandchildSession.title),
    treeNode(siblingSession.id, rootSession.id, siblingSession.title),
  ]
  const fetcher = inboxFetch(() =>
    Promise.resolve(jsonResponse({ revision: 1, sessions: tree })),
  )
  vi.stubGlobal('fetch', fetcher)

  renderInboxPanes(sessions, rootSession)
  await waitForState(
    () =>
      container.querySelector('#session-tree-row-grandchild-session') !== null,
  )
  act(() => {
    container
      .querySelector<HTMLButtonElement>('[aria-label="Collapse Child session"]')
      ?.click()
  })
  renderInboxPanes(sessions, siblingSession)
  const sameTreeNavigation = {
    loading: container.querySelector('.session-tree-feedback')?.textContent,
    currentRow: container.querySelector('.session-tree-row.is-current')?.id,
    focusedRow: container.querySelector('.session-tree-row.is-focused')?.id,
    treeFocused:
      document.activeElement === container.querySelector('.session-tree-list'),
    childExpanded: container
      .querySelector('#session-tree-row-child-session')
      ?.getAttribute('aria-expanded'),
    treeRequests: fetcher.mock.calls
      .map(([input]) => requestUrl(input))
      .filter((url) => url.endsWith('/tree')),
  }

  renderInboxPanes(sessions, grandchildSession)
  const collapsedDescendantNavigation = {
    currentRow: container.querySelector('.session-tree-row.is-current')?.id,
    focusedRow: container.querySelector('.session-tree-row.is-focused')?.id,
    treeFocused:
      document.activeElement === container.querySelector('.session-tree-list'),
    childExpanded: container
      .querySelector('#session-tree-row-child-session')
      ?.getAttribute('aria-expanded'),
    grandchildRow:
      container.querySelector('#session-tree-row-grandchild-session') !== null,
    treeRequests: fetcher.mock.calls
      .map(([input]) => requestUrl(input))
      .filter((url) => url.endsWith('/tree')),
  }

  const snapshot = () => ({
    sameTreeNavigation,
    collapsedDescendantNavigation,
  })
  expect(snapshot()).toEqual({
    sameTreeNavigation: {
      loading: undefined,
      currentRow: 'session-tree-row-sibling-session',
      focusedRow: 'session-tree-row-child-session',
      treeFocused: true,
      childExpanded: 'false',
      treeRequests: ['/api/sessions/root-session/tree'],
    },
    collapsedDescendantNavigation: {
      currentRow: 'session-tree-row-grandchild-session',
      focusedRow: 'session-tree-row-child-session',
      treeFocused: true,
      childExpanded: 'true',
      grandchildRow: true,
      treeRequests: ['/api/sessions/root-session/tree'],
    },
  })
})

it.each(['success', 'failure'] as const)(
  'ignores an older tree request after a newer tree is cached (%s)',
  async (oldRequestResult) => {
    const rootSession = treeSession('root-session', 'Root session')
    const childSession = treeSession(
      'child-session',
      'Child session',
      rootSession,
    )
    const newChildSession = treeSession(
      'new-child-session',
      'New child session',
      childSession,
    )
    const sessions = [rootSession, childSession, newChildSession]
    const oldTree = [
      treeNode(rootSession.id, null, rootSession.title),
      treeNode(childSession.id, rootSession.id, childSession.title),
    ]
    const latestTree = [
      treeNode(childSession.id, null, childSession.title),
      treeNode(newChildSession.id, childSession.id, newChildSession.title),
    ]
    let resolveOldTree: ((response: Response) => void) | undefined
    const oldTreeResponse = new Promise<Response>((resolve) => {
      resolveOldTree = resolve
    })
    const fetcher = inboxFetch((url) => {
      if (url.endsWith(`/${rootSession.id}/tree`)) return oldTreeResponse
      return Promise.resolve(
        jsonResponse({ revision: 2, sessions: latestTree }),
      )
    })
    vi.stubGlobal('fetch', fetcher)

    renderInboxPanes(sessions, rootSession)
    renderInboxPanes(sessions, childSession)
    await waitForState(
      () =>
        container.querySelector('#session-tree-row-new-child-session') !== null,
    )
    await act(async () => {
      resolveOldTree?.(
        oldRequestResult === 'success'
          ? jsonResponse({ revision: 1, sessions: oldTree })
          : jsonResponse({ error: 'Old tree error' }, 503),
      )
      await oldTreeResponse
    })
    renderInboxPanes(sessions, childSession)

    const snapshot = () => ({
      alert: container.querySelector('[role="alert"]')?.textContent,
      currentRow: container.querySelector('.session-tree-row.is-current')?.id,
      newChildRow:
        container.querySelector('#session-tree-row-new-child-session') !== null,
      treeRequests: fetcher.mock.calls
        .map(([input]) => requestUrl(input))
        .filter((url) => url.endsWith('/tree')),
    })
    expect(snapshot()).toEqual({
      alert: undefined,
      currentRow: 'session-tree-row-child-session',
      newChildRow: true,
      treeRequests: [
        '/api/sessions/root-session/tree',
        '/api/sessions/child-session/tree',
      ],
    })
  },
)

it('retains the cached tree after a refresh failure without retrying the same refresh key', async () => {
  const rootSession = treeSession('root-session', 'Root session')
  const childSession = treeSession(
    'child-session',
    'Child session',
    rootSession,
  )
  const siblingSession = treeSession(
    'sibling-session',
    'Sibling session',
    rootSession,
  )
  const sessions = [rootSession, childSession, siblingSession]
  const tree = [
    treeNode(rootSession.id, null, rootSession.title),
    treeNode(childSession.id, rootSession.id, childSession.title),
    treeNode(siblingSession.id, rootSession.id, siblingSession.title),
  ]
  let resolveRefresh: ((response: Response) => void) | undefined
  const refreshResponse = new Promise<Response>((resolve) => {
    resolveRefresh = resolve
  })
  const fetcher = inboxFetch((url) =>
    url.endsWith(`/${siblingSession.id}/tree`)
      ? refreshResponse
      : Promise.resolve(jsonResponse({ revision: 1, sessions: tree })),
  )
  vi.stubGlobal('fetch', fetcher)

  renderInboxPanes(sessions, rootSession)
  await waitForState(
    () => container.querySelector('#session-tree-row-child-session') !== null,
  )
  renderInboxPanes(sessions, siblingSession)
  const requestsBeforeRefresh = fetcher.mock.calls
    .map(([input]) => requestUrl(input))
    .filter((url) => url.endsWith('/tree'))
  const sessionRefreshRevisions = new Map([[siblingSession.id, 1]])
  renderInboxPanes(sessions, siblingSession, sessionRefreshRevisions)
  await act(async () => {
    resolveRefresh?.(jsonResponse({ error: 'Refresh failed' }, 503))
    await refreshResponse
  })
  await waitForState(() => container.querySelector('[role="alert"]') !== null)
  renderInboxPanes(sessions, childSession, sessionRefreshRevisions)

  const snapshot = () => ({
    requestsBeforeRefresh,
    alert: container.querySelector('[role="alert"]')?.textContent,
    currentRow: container.querySelector('.session-tree-row.is-current')?.id,
    childRow:
      container.querySelector('#session-tree-row-child-session') !== null,
    treeRequests: fetcher.mock.calls
      .map(([input]) => requestUrl(input))
      .filter((url) => url.endsWith('/tree')),
  })
  expect(snapshot()).toEqual({
    requestsBeforeRefresh: ['/api/sessions/root-session/tree'],
    alert: 'Could not refresh the session tree: Refresh failed',
    currentRow: 'session-tree-row-child-session',
    childRow: true,
    treeRequests: [
      '/api/sessions/root-session/tree',
      '/api/sessions/sibling-session/tree',
    ],
  })
})

it('removes sessions dropped by a refreshed tree from the cache', async () => {
  const rootSession = treeSession('root-session', 'Root session')
  const childSession = treeSession(
    'child-session',
    'Child session',
    rootSession,
  )
  const grandchildSession = treeSession(
    'grandchild-session',
    'Grandchild session',
    childSession,
  )
  const siblingSession = treeSession(
    'sibling-session',
    'Sibling session',
    rootSession,
  )
  const sessions = [
    rootSession,
    childSession,
    grandchildSession,
    siblingSession,
  ]
  const oldTree = [
    treeNode(rootSession.id, null, rootSession.title),
    treeNode(childSession.id, rootSession.id, childSession.title),
    treeNode(grandchildSession.id, childSession.id, grandchildSession.title),
    treeNode(siblingSession.id, rootSession.id, siblingSession.title),
  ]
  const updatedTree = [
    treeNode(rootSession.id, null, rootSession.title),
    treeNode(childSession.id, rootSession.id, childSession.title),
    treeNode(siblingSession.id, rootSession.id, 'Updated sibling session'),
  ]
  const fetcher = inboxFetch((url) =>
    Promise.resolve(
      jsonResponse({
        revision: 2,
        sessions: url.endsWith(`/${rootSession.id}/tree`)
          ? oldTree
          : updatedTree,
      }),
    ),
  )
  vi.stubGlobal('fetch', fetcher)

  renderInboxPanes(sessions, rootSession)
  await waitForState(
    () =>
      container.querySelector('#session-tree-row-grandchild-session') !== null,
  )
  const sessionRefreshRevisions = new Map([[siblingSession.id, 1]])
  renderInboxPanes(sessions, siblingSession, sessionRefreshRevisions)
  await waitForState(
    () =>
      container.querySelector(
        '#session-tree-row-sibling-session .session-tree-title',
      )?.textContent === 'Updated sibling session',
  )
  renderInboxPanes(sessions, grandchildSession, sessionRefreshRevisions)
  await waitForState(() => container.querySelector('[role="alert"]') !== null)

  const snapshot = () => ({
    currentRow: container.querySelector('.session-tree-row.is-current')?.id,
    siblingTitle: container.querySelector(
      '#session-tree-row-sibling-session .session-tree-title',
    )?.textContent,
    alert: container.querySelector('[role="alert"]')?.textContent,
    treeRequests: fetcher.mock.calls
      .map(([input]) => requestUrl(input))
      .filter((url) => url.endsWith('/tree')),
  })
  expect(snapshot()).toEqual({
    currentRow: undefined,
    siblingTitle: undefined,
    alert:
      'Could not load the session tree: The session tree does not contain the selected session.',
    treeRequests: [
      '/api/sessions/root-session/tree',
      '/api/sessions/sibling-session/tree',
      '/api/sessions/grandchild-session/tree',
    ],
  })
})

it('loads a different tree after navigating away from a cached tree', async () => {
  const rootSession = treeSession('root-session', 'Root session')
  const childSession = treeSession(
    'child-session',
    'Child session',
    rootSession,
  )
  const otherSession = treeSession('other-session', 'Other session')
  const otherChildSession = treeSession(
    'other-child-session',
    'Other child session',
    otherSession,
  )
  const sessions = [rootSession, childSession, otherSession, otherChildSession]
  const rootTree = [
    treeNode(rootSession.id, null, rootSession.title),
    treeNode(childSession.id, rootSession.id, childSession.title),
  ]
  const otherTree = [
    treeNode(otherSession.id, null, otherSession.title),
    treeNode(otherChildSession.id, otherSession.id, otherChildSession.title),
  ]
  const fetcher = inboxFetch((url) =>
    Promise.resolve(
      jsonResponse({
        revision: 1,
        sessions: url.endsWith(`/${otherSession.id}/tree`)
          ? otherTree
          : rootTree,
      }),
    ),
  )
  vi.stubGlobal('fetch', fetcher)

  renderInboxPanes(sessions, rootSession)
  await waitForState(
    () => container.querySelector('#session-tree-row-child-session') !== null,
  )
  renderInboxPanes(sessions, otherSession)
  const otherTreeLoading = container.querySelector(
    '.session-tree-feedback',
  )?.textContent
  await waitForState(
    () =>
      container.querySelector('#session-tree-row-other-child-session') !== null,
  )

  const snapshot = () => ({
    otherTreeLoading,
    currentRow: container.querySelector('.session-tree-row.is-current')?.id,
    otherChildRow:
      container.querySelector('#session-tree-row-other-child-session') !== null,
    treeRequests: fetcher.mock.calls
      .map(([input]) => requestUrl(input))
      .filter((url) => url.endsWith('/tree')),
  })
  expect(snapshot()).toEqual({
    otherTreeLoading: 'Loading session tree…',
    currentRow: 'session-tree-row-other-session',
    otherChildRow: true,
    treeRequests: [
      '/api/sessions/root-session/tree',
      '/api/sessions/other-session/tree',
    ],
  })
})

afterEach(() => {
  if (root !== null) act(() => root?.unmount())
  root = null
  container.remove()
  vi.useRealTimers()
  vi.restoreAllMocks()
  optimizeImageAttachmentMock.mockImplementation((file) =>
    Promise.resolve(file),
  )
  vi.unstubAllGlobals()
  if (originalExecCommand === undefined) {
    Reflect.deleteProperty(document, 'execCommand')
  } else {
    Object.defineProperty(document, 'execCommand', originalExecCommand)
  }
  if (originalClipboard === undefined) {
    Reflect.deleteProperty(navigator, 'clipboard')
  } else {
    Object.defineProperty(navigator, 'clipboard', originalClipboard)
  }
  if (originalVisibilityState === undefined) {
    Reflect.deleteProperty(document, 'visibilityState')
  } else {
    Object.defineProperty(document, 'visibilityState', originalVisibilityState)
  }
})

it('shows pending and sent permission action feedback', () => {
  const onAction = vi.fn(() => Promise.resolve(true))
  const renderPermission = (status: 'sending' | 'sent') => {
    renderView(
      onAction,
      session,
      undefined,
      [permission, otherPermission],
      actions,
      false,
      [],
      [],
      false,
      undefined,
      {
        action: 'approve',
        status,
        requestId: 'permission-request-main',
      },
    )
  }
  renderPermission('sending')
  const cards = () =>
    Array.from(container.querySelectorAll('.chat-permission-card'))
  const sending = {
    cards: cards().map((card) => ({
      labels: Array.from(card.querySelectorAll('button')).map(
        (button) => button.textContent,
      ),
      disabled: Array.from(
        card.querySelectorAll<HTMLButtonElement>('button'),
      ).map((button) => button.disabled),
      status:
        card.querySelector('.permission-action-status')?.textContent ?? null,
    })),
  }

  renderPermission('sent')

  const snapshot = () => ({
    sending,
    sent: {
      cards: cards().map((card) => ({
        labels: Array.from(card.querySelectorAll('button')).map(
          (button) => button.textContent,
        ),
        disabled: Array.from(
          card.querySelectorAll<HTMLButtonElement>('button'),
        ).map((button) => button.disabled),
        status:
          card.querySelector('.permission-action-status')?.textContent ?? null,
      })),
    },
  })
  expect(snapshot()).toEqual({
    sending: {
      cards: [
        {
          labels: ['Approving…', 'Reject'],
          disabled: [true, true],
          status: null,
        },
        {
          labels: ['Approve', 'Reject'],
          disabled: [false, false],
          status: null,
        },
      ],
    },
    sent: {
      cards: [
        {
          labels: ['Approve', 'Reject'],
          disabled: [true, true],
          status: 'Approval sent. Waiting for Claude Code…',
        },
        {
          labels: ['Approve', 'Reject'],
          disabled: [false, false],
          status: null,
        },
      ],
    },
  })
})

it('sends the request id from the selected permission card', () => {
  const onAction = vi.fn(() => Promise.resolve(true))
  renderView(onAction, session, undefined, [permission, otherPermission])

  act(() => {
    requireElement(
      container.querySelectorAll<HTMLButtonElement>(
        '.chat-permission-card .permission-actions button',
      )[2] ?? null,
    ).click()
  })

  expect(onAction.mock.calls).toEqual([
    [
      'permission/approve',
      { requestId: 'permission-request-subagent' },
      { requestId: 'permission-request-subagent' },
    ],
  ])
})

it('copies the session ID from desktop and mobile headers', async () => {
  let copiedText = ''
  const writeText = vi.fn((text: string) => {
    copiedText = text
    return Promise.resolve()
  })
  Object.defineProperty(navigator, 'clipboard', {
    configurable: true,
    value: { writeText },
  })
  const onAction = vi.fn(() => Promise.resolve(true))
  renderView(onAction)

  const desktopButton = requireElement(
    container.querySelector<HTMLButtonElement>(
      '.conversation-header-copy .session-heading-title-row .conversation-copy-session-id',
    ),
  )
  await act(async () => {
    desktopButton.click()
    await new Promise((resolve) => window.setTimeout(resolve, 0))
  })
  const desktopButtonText = desktopButton.textContent

  renderView(onAction, session, undefined, [], actions, true)
  const mobileButton = requireElement(
    container.querySelector<HTMLButtonElement>(
      ".conversation-pane[data-mobile='true'] .conversation-copy-session-id",
    ),
  )
  await act(async () => {
    mobileButton.click()
    await new Promise((resolve) => window.setTimeout(resolve, 0))
  })

  const snapshot = () => ({
    copiedText,
    writeCalls: writeText.mock.calls,
    desktopButtonText,
    mobileButtonText: mobileButton.textContent,
    mobileAccessibleName: mobileButton.getAttribute('aria-label'),
  })
  expect(snapshot()).toEqual({
    copiedText: session.id,
    writeCalls: [[session.id], [session.id]],
    desktopButtonText: 'Copied',
    mobileButtonText: 'Copied',
    mobileAccessibleName: 'Session ID copied',
  })
})

it('copies the session ID through the HTTP-compatible clipboard fallback', async () => {
  Object.defineProperty(navigator, 'clipboard', {
    configurable: true,
    value: undefined,
  })
  let copiedText = ''
  const copyCommand = vi.fn(() => {
    copiedText =
      Array.from(document.body.querySelectorAll('textarea')).at(-1)?.value ?? ''
    return true
  })
  Object.defineProperty(document, 'execCommand', {
    configurable: true,
    value: copyCommand,
  })
  renderView(
    vi.fn(() => Promise.resolve(true)),
    session,
    undefined,
    [],
    actions,
    true,
  )

  const copyButton = requireElement(
    container.querySelector<HTMLButtonElement>(
      ".conversation-pane[data-mobile='true'] .conversation-copy-session-id",
    ),
  )
  let copyCallsAtClickReturn = 0
  act(() => {
    copyButton.click()
    copyCallsAtClickReturn = copyCommand.mock.calls.length
  })
  await act(async () => {
    await new Promise((resolve) => window.setTimeout(resolve, 0))
  })

  const snapshot = () => ({
    copiedText,
    copyCalls: copyCommand.mock.calls,
    copyCallsAtClickReturn,
    temporaryTextareaCount: document.body.querySelectorAll('textarea').length,
    buttonText: copyButton.textContent,
  })
  expect(snapshot()).toEqual({
    copiedText: session.id,
    copyCalls: [['copy']],
    copyCallsAtClickReturn: 1,
    temporaryTextareaCount: 1,
    buttonText: 'Copied',
  })
})

it('falls back when the Clipboard API rejects the copy request', async () => {
  const writeText = vi.fn(() => Promise.reject(new Error('copy denied')))
  Object.defineProperty(navigator, 'clipboard', {
    configurable: true,
    value: { writeText },
  })
  let copiedText = ''
  const copyCommand = vi.fn(() => {
    copiedText =
      Array.from(document.body.querySelectorAll('textarea')).at(-1)?.value ?? ''
    return true
  })
  Object.defineProperty(document, 'execCommand', {
    configurable: true,
    value: copyCommand,
  })
  renderView(
    vi.fn(() => Promise.resolve(true)),
    session,
    undefined,
    [],
    actions,
    true,
  )

  const copyButton = requireElement(
    container.querySelector<HTMLButtonElement>(
      ".conversation-pane[data-mobile='true'] .conversation-copy-session-id",
    ),
  )
  await act(async () => {
    copyButton.click()
    await new Promise((resolve) => window.setTimeout(resolve, 0))
  })

  const snapshot = () => ({
    clipboardCalls: writeText.mock.calls,
    fallbackCalls: copyCommand.mock.calls,
    copiedText,
    buttonText: copyButton.textContent,
  })
  expect(snapshot()).toEqual({
    clipboardCalls: [[session.id]],
    fallbackCalls: [['copy']],
    copiedText: session.id,
    buttonText: 'Copied',
  })
})

it('announces when neither clipboard method can copy the session ID', async () => {
  Object.defineProperty(navigator, 'clipboard', {
    configurable: true,
    value: undefined,
  })
  Object.defineProperty(document, 'execCommand', {
    configurable: true,
    value: vi.fn(() => false),
  })
  renderView(
    vi.fn(() => Promise.resolve(true)),
    session,
    undefined,
    [],
    actions,
    true,
  )

  const copyButton = requireElement(
    container.querySelector<HTMLButtonElement>(
      ".conversation-pane[data-mobile='true'] .conversation-copy-session-id",
    ),
  )
  await act(async () => {
    copyButton.click()
    await new Promise((resolve) => window.setTimeout(resolve, 0))
  })

  const snapshot = () => ({
    buttonText: copyButton.textContent,
    accessibleName: copyButton.getAttribute('aria-label'),
    liveStatus: container.querySelector('.sr-only[role="status"]')?.textContent,
  })
  expect(snapshot()).toEqual({
    buttonText: 'Copy failed',
    accessibleName: 'Could not copy session ID',
    liveStatus: 'Could not copy session ID',
  })
})

it('announces copy failure and removes the textarea when execCommand throws', async () => {
  Object.defineProperty(navigator, 'clipboard', {
    configurable: true,
    value: undefined,
  })
  const copyCommand = vi.fn(() => {
    throw new Error('copy unavailable')
  })
  Object.defineProperty(document, 'execCommand', {
    configurable: true,
    value: copyCommand,
  })
  renderView(
    vi.fn(() => Promise.resolve(true)),
    session,
    undefined,
    [],
    actions,
    true,
  )

  const copyButton = requireElement(
    container.querySelector<HTMLButtonElement>(
      ".conversation-pane[data-mobile='true'] .conversation-copy-session-id",
    ),
  )
  await act(async () => {
    copyButton.click()
    await new Promise((resolve) => window.setTimeout(resolve, 0))
  })

  const snapshot = () => ({
    copyCalls: copyCommand.mock.calls,
    temporaryTextareaCount: document.body.querySelectorAll('textarea').length,
    buttonText: copyButton.textContent,
    liveStatus: container.querySelector('.sr-only[role="status"]')?.textContent,
  })
  expect(snapshot()).toEqual({
    copyCalls: [['copy']],
    temporaryTextareaCount: 1,
    buttonText: 'Copy failed',
    liveStatus: 'Could not copy session ID',
  })
})

it('sends a selected answer with its request ID', async () => {
  const onAction = vi.fn(() => Promise.resolve(true))
  renderView(onAction, session, undefined, [question])

  const choice = requireElement(
    Array.from(
      container.querySelectorAll<HTMLButtonElement>(
        '.chat-question-choices button',
      ),
    ).find((button) => button.textContent.includes('Window B')) ?? null,
  )
  act(() => {
    choice.click()
  })

  const submit = requireElement(
    container.querySelector<HTMLButtonElement>(
      '.chat-question-actions button.aw-button--primary',
    ),
  )
  await act(async () => {
    submit.click()
    await Promise.resolve()
  })

  expect(onAction.mock.calls).toEqual([
    [
      'answer',
      {
        requestId: 'ask-request-main',
        answers: { 'Which cache window should be used?': 'Window B' },
      },
    ],
  ])
})

it('skips a question with its request ID', async () => {
  const onAction = vi.fn(() => Promise.resolve(true))
  renderView(onAction, session, undefined, [question])

  const skip = requireElement(
    Array.from(container.querySelectorAll<HTMLButtonElement>('button')).find(
      (button) => button.textContent.includes('No answer'),
    ) ?? null,
  )
  await act(async () => {
    skip.click()
    await Promise.resolve()
  })

  expect(onAction.mock.calls).toEqual([
    ['answer', { requestId: 'ask-request-main', skip: true }],
  ])
})

it('keeps every pending main and subagent question answerable', async () => {
  const onAction = vi.fn(() => Promise.resolve(true))
  const subagentQuestion: ChatItem = {
    type: 'question',
    requestId: 'ask-request-subagent',
    questions: [
      {
        ...questionPrompt,
        question: 'Which icon should the helper use?',
      },
    ],
  }
  renderView(onAction, session, undefined, [question, subagentQuestion])

  const cards = Array.from(
    container.querySelectorAll<HTMLElement>('.chat-question-card'),
  )
  const choiceButtons = cards.map((card) =>
    Array.from(
      card.querySelectorAll<HTMLButtonElement>('.chat-question-choices button'),
      (button) => button.disabled,
    ),
  )
  act(() => {
    cards[0]
      ?.querySelector<HTMLButtonElement>('.chat-question-choices button')
      ?.click()
    cards[1]
      ?.querySelector<HTMLButtonElement>('.chat-question-choices button')
      ?.click()
  })
  const submits = cards.map((card) =>
    card.querySelector<HTMLButtonElement>(
      '.chat-question-actions button.aw-button--primary',
    ),
  )
  await act(async () => {
    for (const submit of submits) {
      submit?.click()
      await Promise.resolve()
    }
  })

  const snapshot = () => ({
    choiceButtons,
    requests: onAction.mock.calls,
    cardCount: cards.length,
  })
  expect(snapshot()).toEqual({
    choiceButtons: [
      [false, false],
      [false, false],
    ],
    requests: [
      [
        'answer',
        {
          requestId: 'ask-request-main',
          answers: { 'Which cache window should be used?': 'Window A' },
        },
      ],
      [
        'answer',
        {
          requestId: 'ask-request-subagent',
          answers: { 'Which icon should the helper use?': 'Window A' },
        },
      ],
    ],
    cardCount: 2,
  })
})

it('shows terminal guidance for an unanswered question without a request ID', () => {
  const onAction = vi.fn(() => Promise.resolve(true))
  const noRequestQuestion: ChatItem = {
    type: 'question',
    questions: [questionPrompt],
  }
  renderView(onAction, session, undefined, [noRequestQuestion])

  const snapshot = () => ({
    choicesDisabled: Array.from(
      container.querySelectorAll<HTMLButtonElement>(
        '.chat-question-choices button',
      ),
      (button) => button.disabled,
    ),
    terminalMessage: container.querySelector(
      '.conversation-terminal-fallback span',
    )?.textContent,
    submitButton: container.querySelector(
      '.chat-question-actions button.aw-button--primary',
    ),
    actionCalls: onAction.mock.calls,
  })
  expect(snapshot()).toEqual({
    choicesDisabled: [true, true],
    terminalMessage: 'Answer in terminal to continue.',
    submitButton: null,
    actionCalls: [],
  })
})

it('keeps the message composer available while a proposal waits for a choice', () => {
  renderView(
    vi.fn(() => Promise.resolve(true)),
    session,
    undefined,
    [proposal],
  )

  const snapshot = () => ({
    placeholder: container
      .querySelector<HTMLTextAreaElement>('.conversation-composer textarea')
      ?.getAttribute('placeholder'),
    composerDisabled: container.querySelector<HTMLTextAreaElement>(
      '.conversation-composer textarea',
    )?.disabled,
  })

  expect(snapshot()).toEqual({
    placeholder: 'Choose an option above, or message the session',
    composerDisabled: false,
  })
})

it('sends Command-Enter from the message composer without answering the proposal', async () => {
  const onAction = vi.fn(() => Promise.resolve(true))
  renderView(onAction, session, undefined, [proposal])
  act(() => {
    requireElement(
      container.querySelector<HTMLInputElement>('.chat-proposal-option input'),
    ).click()
  })
  const messageField = requireElement(
    container.querySelector<HTMLTextAreaElement>(
      '.conversation-composer textarea',
    ),
  )
  setTextareaValue(messageField, 'Send a direct message.')
  const submitEvent = dispatchKeyInAct(messageField, 'Enter', { metaKey: true })
  await act(async () => {
    await Promise.resolve()
  })

  const snapshot = () => ({
    submitPrevented: submitEvent.defaultPrevented,
    actionCalls: onAction.mock.calls,
  })
  expect(snapshot()).toEqual({
    submitPrevented: true,
    actionCalls: [['messages', { content: 'Send a direct message.' }]],
  })
})

it('shows a selected answer, dims other choices, and renders Markdown', () => {
  const onAction = vi.fn(() => Promise.resolve(true))
  renderView(onAction, session, undefined, [
    {
      ...question,
      questions: [
        {
          ...questionPrompt,
          question: 'Which **retry** policy should be used?',
          choices: [{ label: 'Retry *a few* times' }, { label: 'Retry once' }],
          answer: 'Retry once',
        },
      ],
    },
  ])
  dispatchKeyInAct(window, '1')
  act(() => {
    container
      .querySelector<HTMLButtonElement>('.chat-question-choices button')
      ?.click()
  })

  const card = requireElement(container.querySelector('.chat-question-card'))
  const snapshot = () => ({
    question: card.querySelector('.chat-question-text strong')?.textContent,
    markdownChoice: card.querySelector('.chat-question-choice-text em')
      ?.textContent,
    answer: card.querySelector('.chat-question-answer')?.textContent,
    choices: Array.from(
      card.querySelectorAll<HTMLButtonElement>('.chat-question-choices button'),
      (button) => ({
        label: button.querySelector('.chat-question-choice-text')?.textContent,
        disabled: button.disabled,
        selected: button.classList.contains('is-selected'),
        dimmed: button.classList.contains('is-unselected'),
        check: button.querySelector('.chat-question-choice-check')?.textContent,
      }),
    ),
    actionCalls: onAction.mock.calls,
  })
  expect(snapshot()).toEqual({
    question: 'retry',
    markdownChoice: 'a few',
    answer: undefined,
    choices: [
      {
        label: 'Retry a few times',
        disabled: true,
        selected: false,
        dimmed: true,
        check: undefined,
      },
      {
        label: 'Retry once',
        disabled: true,
        selected: true,
        dimmed: false,
        check: '✓',
      },
    ],
    actionCalls: [],
  })
})

it('shows a freeform answer below the choices', () => {
  const onAction = vi.fn(() => Promise.resolve(true))
  renderView(onAction, session, undefined, [
    {
      ...question,
      questions: [
        {
          ...questionPrompt,
          answer: 'Wait until the next deployment',
        },
      ],
    },
  ])

  const snapshot = () => ({
    answer: container.querySelector('.chat-question-answer')?.textContent,
    selectedChoices: Array.from(
      container.querySelectorAll<HTMLButtonElement>(
        '.chat-question-choices button.is-selected',
      ),
      (button) => button.textContent,
    ),
    disabledChoices: Array.from(
      container.querySelectorAll<HTMLButtonElement>(
        '.chat-question-choices button',
      ),
      (button) => button.disabled,
    ),
    actionCalls: onAction.mock.calls,
  })
  expect(snapshot()).toEqual({
    answer: 'Answer: Wait until the next deployment',
    selectedChoices: [],
    disabledChoices: [true, true],
    actionCalls: [],
  })
})

it('sends on Enter and Command-Enter and preserves modified newlines', async () => {
  const completedSends: Array<(succeeded: boolean) => void> = []
  const onAction = vi.fn(
    () =>
      new Promise<boolean>((resolve) => {
        completedSends.push(resolve)
      }),
  )
  renderView(onAction)
  const textarea = requireElement(
    container.querySelector<HTMLTextAreaElement>('textarea'),
  )

  setTextareaValue(textarea, 'Use bounded retries')
  act(() => {
    dispatchKey(textarea, 'Enter')
  })
  await act(async () => {
    completedSends[0]?.(true)
    await Promise.resolve()
  })
  setTextareaValue(textarea, 'Keep this as a draft')
  const altEvent = dispatchKeyInAct(textarea, 'Enter', { altKey: true })
  setTextareaValue(textarea, 'Keep the error visible')
  act(() => {
    dispatchKey(textarea, 'Enter', { metaKey: true })
  })
  await act(async () => {
    completedSends[1]?.(true)
    await Promise.resolve()
  })
  setTextareaValue(textarea, 'Add one more detail')
  const shiftEvent = dispatchKeyInAct(textarea, 'Enter', { shiftKey: true })

  const snapshot = () => ({
    actions: onAction.mock.calls,
    defaultPrevented: [altEvent.defaultPrevented, shiftEvent.defaultPrevented],
  })
  expect(snapshot()).toEqual({
    actions: [
      ['messages', { content: 'Use bounded retries' }],
      ['messages', { content: 'Keep the error visible' }],
    ],
    defaultPrevented: [true, false],
  })
})

it('inserts Alt-Enter at the cursor without sending the message', async () => {
  const onAction = vi.fn(() => Promise.resolve(true))
  renderView(onAction)
  const textarea = requireElement(
    container.querySelector<HTMLTextAreaElement>('textarea'),
  )
  setTextareaValue(textarea, 'Keep this as a draft')
  textarea.focus()
  textarea.setSelectionRange(9, 9)

  await act(async () => {
    await userEvent.keyboard('{Alt>}{Enter}{/Alt}')
  })

  const snapshot = () => ({
    draft: textarea.value,
    actionCalls: onAction.mock.calls,
  })
  expect(snapshot()).toEqual({
    draft: `Keep this
 as a draft`,
    actionCalls: [],
  })
})

it('shows desktop newline shortcuts with the applicable interrupt hint', () => {
  const onAction = () => Promise.resolve(true)
  renderView(onAction)
  const interruptibleHint = container.querySelector(
    '.composer-footer > span',
  )?.textContent
  renderView(onAction, session, undefined, [], { ...actions, interrupt: false })
  const nonInterruptibleHint = container.querySelector(
    '.composer-footer > span',
  )?.textContent

  const snapshot = () => ({ interruptibleHint, nonInterruptibleHint })
  expect(snapshot()).toEqual({
    interruptibleHint: '↵ Send · ⇧↵ / ⌥↵ New line · Esc Interrupt',
    nonInterruptibleHint: '↵ Send · ⇧↵ / ⌥↵ New line',
  })
})

it('keeps Enter for newlines and exposes mobile conversation actions', () => {
  const onAction = vi.fn(() => Promise.resolve(true))
  renderView(onAction, session, undefined, [question], actions, true)
  const textarea = requireElement(
    container.querySelector<HTMLTextAreaElement>('textarea'),
  )

  const enterEvent = dispatchKeyInAct(textarea, 'Enter')
  const shiftEnterEvent = dispatchKeyInAct(textarea, 'Enter', {
    shiftKey: true,
  })
  act(() => {
    requireElement(
      container.querySelector<HTMLButtonElement>('.mobile-interrupt-button'),
    ).click()
  })

  const snapshot = () => ({
    actions: onAction.mock.calls,
    defaultPrevented: [
      enterEvent.defaultPrevented,
      shiftEnterEvent.defaultPrevented,
    ],
    keyboardHint: container.querySelector('.composer-footer > span')
      ?.textContent,
    choiceShortcutBadges: container.querySelectorAll(
      '.chat-question-choices kbd',
    ).length,
    interruptLabel: container
      .querySelector('.mobile-interrupt-button')
      ?.getAttribute('aria-label'),
  })
  expect(snapshot()).toEqual({
    actions: [['interrupt']],
    defaultPrevented: [false, false],
    keyboardHint: undefined,
    choiceShortcutBadges: 0,
    interruptLabel: 'Interrupt',
  })
})

it('does not send while Enter confirms an IME composition', () => {
  const onAction = vi.fn(() => Promise.resolve(true))
  renderView(onAction)
  const textarea = requireElement(
    container.querySelector<HTMLTextAreaElement>('textarea'),
  )
  setTextareaValue(textarea, 'A composed message')
  const composingEvent = new KeyboardEvent('keydown', {
    key: 'Enter',
    bubbles: true,
    cancelable: true,
    isComposing: true,
  })
  const legacyImeEvent = new KeyboardEvent('keydown', {
    key: 'Enter',
    bubbles: true,
    cancelable: true,
  })
  Object.defineProperty(legacyImeEvent, 'keyCode', { value: 229 })
  act(() => {
    textarea.dispatchEvent(composingEvent)
    textarea.dispatchEvent(legacyImeEvent)
  })

  const snapshot = () => ({
    actions: onAction.mock.calls,
    defaultPrevented: [
      composingEvent.defaultPrevented,
      legacyImeEvent.defaultPrevented,
    ],
    draft: textarea.value,
  })
  expect(snapshot()).toEqual({
    actions: [],
    defaultPrevented: [false, false],
    draft: 'A composed message',
  })
})

it('interrupts the session on Escape', () => {
  const onAction = vi.fn(() => Promise.resolve(true))
  renderView(onAction)
  const textarea = requireElement(container.querySelector('textarea'))
  const event = dispatchKeyInAct(textarea, 'Escape')

  const actual = JSON.stringify([onAction.mock.calls, event.defaultPrevented])
  const expected = JSON.stringify([[['interrupt']], true])
  expect(actual).toBe(expected)
})

it('does not use question shortcuts while typing', () => {
  const onAction = vi.fn(() => Promise.resolve(true))
  renderView(onAction)
  const textarea = requireElement(container.querySelector('textarea'))
  const event = dispatchKeyInAct(textarea, '1')

  const actual = JSON.stringify([onAction.mock.calls, event.defaultPrevented])
  const expected = JSON.stringify([[], false])
  expect(actual).toBe(expected)
})

it('selects the parent session from its link', () => {
  const onAction = vi.fn(() => Promise.resolve(true))
  const onSelectSession = vi.fn<(id: string) => void>()
  renderView(
    onAction,
    { ...session, parent: { id: 'parent-session', title: 'Parent session' } },
    onSelectSession,
  )

  act(() => {
    requireElement(
      container.querySelector<HTMLAnchorElement>('.conversation-parent'),
    ).click()
  })

  expect(onSelectSession.mock.calls).toEqual([['parent-session']])
})

it('links known session IDs in user, rich user, and assistant messages', () => {
  const linkedSession: Session = {
    ...session,
    id: '11111111-1111-4111-8111-111111111111',
    category: 'done',
    title: 'Inspect the retry path',
    project: 'example toolkit',
  }
  const onSelectSession = vi.fn<(id: string) => void>()
  renderView(
    vi.fn().mockResolvedValue(true),
    session,
    onSelectSession,
    [
      { type: 'user', text: `Use ${linkedSession.id}` },
      {
        type: 'user_with_content',
        content: [{ type: 'text', text: `Inspect ${linkedSession.id}` }],
      },
      { type: 'assistant', text: `I found ${linkedSession.id}` },
    ],
    actions,
    false,
    [],
    [linkedSession],
  )

  const links = Array.from(
    container.querySelectorAll<HTMLAnchorElement>(
      '.chat-user-message a, .chat-assistant-message a',
    ),
  )
  for (const link of links) {
    act(() => {
      link.click()
    })
  }

  const expected = {
    messageClasses: [
      'chat-user-group',
      'chat-user-group',
      'chat-assistant-message',
    ],
    linkHrefs: [
      `?session=${linkedSession.id}`,
      `?session=${linkedSession.id}`,
      `?session=${linkedSession.id}`,
    ],
    selectedIds: [linkedSession.id, linkedSession.id, linkedSession.id],
  }
  const getActual = () => ({
    messageClasses: links.map((link) => link.closest('article')?.className),
    linkHrefs: links.map((link) => link.getAttribute('href')),
    selectedIds: onSelectSession.mock.calls.map(([id]) => id),
  })
  expect(getActual()).toEqual(expected)
})

it('marks a queued prompt below the delivered conversation', () => {
  renderView(vi.fn().mockResolvedValue(true), session, undefined, [
    { type: 'user', text: 'Please summarize the current changes.' },
    { type: 'assistant', text: 'I am reviewing the current changes.' },
    {
      type: 'queued_user',
      content: [{ type: 'text', text: 'Also check the error handling.' }],
    },
  ])

  const actual = Array.from(
    container.querySelectorAll<HTMLElement>(
      '.chat-user-group, .chat-assistant-message',
    ),
  ).map((message) => ({
    className: message.className,
    label: message.querySelector('.chat-from')?.textContent ?? null,
    text: message.querySelector('.markdown-message')?.textContent ?? null,
    bubbleClassName:
      message.querySelector('.chat-user-message')?.className ?? null,
  }))

  expect(actual).toEqual([
    {
      className: 'chat-user-group',
      label: 'You',
      text: 'Please summarize the current changes.',
      bubbleClassName: 'markdown-message chat-user-message',
    },
    {
      className: 'chat-assistant-message',
      label: null,
      text: 'I am reviewing the current changes.',
      bubbleClassName: null,
    },
    {
      className: 'chat-user-group',
      label: 'You · Queued',
      text: 'Also check the error handling.',
      bubbleClassName: 'markdown-message chat-user-message',
    },
  ])
})

it('shows a local time on system event rows', () => {
  const timestamp = '2026-06-01T12:00:00Z'
  const localTime = formatMessageTime(timestamp) ?? 'invalid timestamp'
  renderView(() => Promise.resolve(true), session, undefined, [
    {
      type: 'event',
      kind: 'system',
      text: 'A synthetic operation was blocked.',
      failed: true,
      timestamp,
    },
  ])

  const event = container.querySelector<HTMLElement>('.chat-event')
  const getActual = () => ({
    text: event?.querySelector('.chat-event-text')?.textContent ?? null,
    time: event?.querySelector('time')?.textContent ?? null,
    dateTime: event?.querySelector('time')?.getAttribute('dateTime') ?? null,
    kind: event?.getAttribute('data-kind') ?? null,
    failed: event?.getAttribute('data-failed') ?? null,
  })
  expect(getActual()).toEqual({
    text: 'A synthetic operation was blocked.',
    time: localTime,
    dateTime: timestamp,
    kind: 'system',
    failed: 'true',
  })
})

it('renders compact boundaries as a distinct conversation event', () => {
  renderView(() => Promise.resolve(true), session, undefined, [
    {
      type: 'event',
      kind: 'compact_boundary',
      text: 'Context compacted',
      failed: false,
    },
  ])

  const event = container.querySelector<HTMLElement>('.chat-event')
  const getActual = () => ({
    text: event?.querySelector('.chat-event-text')?.textContent ?? null,
    kind: event?.getAttribute('data-kind') ?? null,
  })
  expect(getActual()).toEqual({
    text: 'Context compacted',
    kind: 'compact_boundary',
  })
})

it('shows peer images and supports keyboard navigation', () => {
  const linkedSession: Session = {
    ...session,
    id: 'fictional-peer-session',
    category: 'in_progress',
    title: 'Build verification',
  }
  const onSelectSession = vi.fn<(id: string) => void>()
  renderView(
    vi.fn().mockResolvedValue(true),
    session,
    onSelectSession,
    [
      {
        type: 'peer',
        direction: 'to',
        session_id: linkedSession.id,
        content: [
          { type: 'text', text: 'Please check the fictional output.' },
          {
            type: 'image',
            source: '/api/images/fixture-peer-image.png',
            available: false,
          },
        ],
      },
      {
        type: 'peer',
        direction: 'from',
        session_id: linkedSession.id,
        content: [
          { type: 'text', text: 'The fictional output passed.' },
          {
            type: 'image',
            source: '/api/images/fixture-peer-image.png',
            available: true,
          },
          { type: 'file', name: 'fixture-peer-layout.html' },
        ],
      },
    ],
    actions,
    false,
    [],
    [linkedSession],
  )

  const peerLink = requireElement(
    container.querySelector<HTMLAnchorElement>(
      '.chat-peer-message-from .session-link-trigger',
    ),
  )
  const fromContent = requireElement(
    container.querySelector<HTMLElement>(
      '.chat-peer-message-from .chat-peer-content',
    ),
  )
  const toContent = requireElement(
    container.querySelector<HTMLElement>(
      '.chat-peer-message-to .chat-peer-content',
    ),
  )
  act(() => {
    toContent.click()
  })
  dispatchKeyInAct(fromContent, 'Enter')
  dispatchKeyInAct(toContent, ' ')

  const snapshot = () => ({
    sessionLinkHref: peerLink.getAttribute('href'),
    items: Array.from(
      container.querySelectorAll<HTMLElement>('.chat-peer-message'),
      (element) => ({
        className: element.className,
        text: element.textContent,
        imageSources: Array.from(
          element.querySelectorAll<HTMLImageElement>('img'),
          (image) => image.getAttribute('src'),
        ),
        unavailableImageText: element.querySelector(
          '.chat-peer-image-unavailable',
        )?.textContent,
        fileNames: Array.from(
          element.querySelectorAll<HTMLElement>('.chat-peer-file'),
          (file) => file.textContent,
        ),
      }),
    ),
    selectedIds: onSelectSession.mock.calls.map(([id]) => id),
  })
  const expected = {
    sessionLinkHref: `?session=${linkedSession.id}`,
    items: [
      {
        className: 'chat-peer-message chat-peer-message-to',
        text: 'TOBuild verificationPlease check the fictional output.画像はもうありません',
        imageSources: [],
        unavailableImageText: '画像はもうありません',
        fileNames: [],
      },
      {
        className: 'chat-peer-message chat-peer-message-from',
        text: 'FROMBuild verificationThe fictional output passed.fixture-peer-layout.html',
        imageSources: ['/api/images/fixture-peer-image.png'],
        unavailableImageText: undefined,
        fileNames: ['fixture-peer-layout.html'],
      },
    ],
    selectedIds: [linkedSession.id, linkedSession.id, linkedSession.id],
  }
  expect(snapshot()).toEqual(expected)
})

it('shows the linked task count in a peer session card', async () => {
  const linkedSession: Session = {
    ...session,
    id: 'fictional-peer-session',
    category: 'in_progress',
    kind: 'Review',
    title: 'Review task links',
    preview: 'Check the linked task summary.',
    elapsedSeconds: 120,
    engine: 'claude',
    workType: 'PM',
    task: {
      id: 'fixture-task-one',
      number: 71,
      title: 'Review the linked task',
      priority: 1,
    },
    tasks: [
      {
        id: 'fixture-task-one',
        number: 71,
        title: 'Review the linked task',
        priority: 1,
      },
      {
        id: 'fixture-task-two',
        number: 72,
        title: 'Confirm the task count',
        priority: 2,
      },
      {
        id: 'fixture-task-three',
        number: 73,
        title: 'Update the overview',
        priority: null,
      },
    ],
    repo: 'sample-org/example-repo',
  }
  const message: ChatItem = {
    type: 'peer',
    direction: 'from',
    session_id: linkedSession.id,
    content: [{ type: 'text', text: 'A fictional peer update.' }],
  }
  renderView(
    vi.fn().mockResolvedValue(true),
    session,
    undefined,
    [message],
    actions,
    false,
    [],
    [linkedSession],
  )

  const peerLink = requireElement(
    container.querySelector<HTMLAnchorElement>(
      '.chat-peer-message-from .session-link-trigger',
    ),
  )
  await act(async () => {
    await userEvent.hover(peerLink)
    await new Promise((resolve) => setTimeout(resolve, 250))
  })

  const openBesideModifier =
    getCommandPaletteShortcut(navigator.platform).modifier === 'metaKey'
      ? '⌘'
      : 'Ctrl'
  const card = requireElement(
    document.querySelector<HTMLElement>('.session-link-card'),
  )
  expect(card.textContent).toBe(
    `Review task linksPM · claudeReview · Check the linked task summary. · 2m` +
      `P1 #71Review the linked task +2sample-org/example-repo` +
      `Click to open · ${openBesideModifier}-click to open beside`,
  )
})

it('refreshes peer session labels when the linked session title changes', () => {
  const linkedSession: Session = {
    ...session,
    id: 'fictional-peer-session',
    title: 'Earlier task name',
  }
  const message: ChatItem = {
    type: 'peer',
    direction: 'from',
    session_id: linkedSession.id,
    content: [{ type: 'text', text: 'A fictional update.' }],
  }
  const onAction = vi.fn().mockResolvedValue(true)
  renderView(
    onAction,
    session,
    undefined,
    [message],
    actions,
    false,
    [],
    [linkedSession],
  )
  const labelBeforeRefresh = requireElement(
    container.querySelector<HTMLElement>('.chat-peer-label'),
  ).textContent

  renderView(
    onAction,
    session,
    undefined,
    [message],
    actions,
    false,
    [],
    [{ ...linkedSession, title: 'Updated task name' }],
  )

  const snapshot = () => ({
    labelBeforeRefresh,
    labelAfterRefresh: container.querySelector('.chat-peer-label')?.textContent,
  })
  expect(snapshot()).toEqual({
    labelBeforeRefresh: 'FROMEarlier task name',
    labelAfterRefresh: 'FROMUpdated task name',
  })
})

it('renders file attachments returned by the messages endpoint', async () => {
  const jsonResponse = (body: unknown) =>
    new Response(JSON.stringify(body), {
      headers: { 'Content-Type': 'application/json' },
    })
  const fetcher = vi
    .fn<typeof fetch>()
    .mockResolvedValueOnce(
      jsonResponse(
        messagesResponse([
          {
            type: 'user_with_content',
            content: [
              { type: 'text', text: 'Inspect this document.' },
              { type: 'file', name: 'sample-layout.html' },
            ],
          },
        ]),
      ),
    )
    .mockResolvedValueOnce(jsonResponse(actions))
  vi.stubGlobal('fetch', fetcher)
  root = createRoot(container)
  act(() => {
    root?.render(
      <ConversationPane
        session={session}
        refreshKey={0}
        sessionTreeRefreshKey={0}
        onSelectSession={() => undefined}
      />,
    )
  })
  expect(
    await waitForState(
      () =>
        container.querySelector('.chat-user-file')?.textContent ===
        'sample-layout.html',
    ),
  ).toBe(true)

  const snapshot = () => ({
    message: container.querySelector('.chat-user-message')?.textContent,
    attachment: container.querySelector('.chat-user-file')?.textContent,
    alert: container.querySelector('[role="alert"]')?.textContent ?? null,
  })
  expect(snapshot()).toEqual({
    message: 'Inspect this document.',
    attachment: 'sample-layout.html',
    alert: null,
  })
})
it('posts a proposal answer to its sequence route and refreshes the conversation', async () => {
  const jsonResponse = (body: unknown) =>
    new Response(JSON.stringify(body), {
      headers: { 'Content-Type': 'application/json' },
    })
  const requests: Array<{
    url: string
    method: string
    body: string | null
  }> = []
  const fetcher = vi.fn<typeof fetch>((input, init) => {
    const url =
      typeof input === 'string'
        ? input
        : input instanceof URL
          ? input.toString()
          : input.url
    const method = init?.method ?? 'GET'
    requests.push({
      url,
      method,
      body: typeof init?.body === 'string' ? init.body : null,
    })
    return Promise.resolve(
      method === 'POST'
        ? new Response(null, { status: 204 })
        : url.split('?')[0]?.endsWith('/messages') === true
          ? jsonResponse(messagesResponse([proposal]))
          : jsonResponse(actions),
    )
  })
  vi.stubGlobal('fetch', fetcher)
  root = createRoot(container)
  act(() => {
    root?.render(
      <ConversationPane
        session={session}
        refreshKey={0}
        sessionTreeRefreshKey={0}
        onSelectSession={() => undefined}
      />,
    )
  })
  await act(async () => {
    for (let index = 0; index < 6; index += 1) await Promise.resolve()
  })

  act(() => {
    requireElement(
      container.querySelectorAll<HTMLInputElement>(
        '.chat-proposal-option input',
      )[1] ?? null,
    ).click()
    requireElement(
      container.querySelector<HTMLButtonElement>(
        '.chat-proposal-actions .aw-button--primary',
      ),
    ).click()
  })
  await act(async () => {
    for (let index = 0; index < 8; index += 1) await Promise.resolve()
  })

  const snapshot = () => ({
    post: requests.find((request) => request.method === 'POST') ?? null,
    placeholder: container
      .querySelector<HTMLTextAreaElement>('.conversation-composer textarea')
      ?.getAttribute('placeholder'),
    status: container.querySelector('.chat-proposal-card h2')?.textContent,
  })
  expect(snapshot()).toEqual({
    post: {
      url: '/api/sessions/sample-session/proposals/3/answer',
      method: 'POST',
      body: '{"choice":1,"comment":null}',
    },
    placeholder: 'Choose an option above, or message the session',
    status: 'Proposal · chose B',
  })
})

it('posts a todo completion to its sequence route with the comment', async () => {
  const todo: ChatItem = {
    type: 'todo',
    seq: 4,
    title: 'Check the page on a phone',
    detail: 'Confirm the action remains visible.',
    completed: false,
    comment: null,
  }
  const completedTodo: ChatItem = {
    ...todo,
    completed: true,
    comment: 'The page is readable.',
  }
  const requests: Array<{
    url: string
    method: string
    body: string | null
  }> = []
  let completed = false
  const fetcher = vi.fn<typeof fetch>((input, init) => {
    const url = requestUrl(input)
    const method = init?.method ?? 'GET'
    requests.push({
      url,
      method,
      body: typeof init?.body === 'string' ? init.body : null,
    })
    if (method === 'POST') completed = true
    return Promise.resolve(
      method === 'POST'
        ? new Response(null, { status: 204 })
        : url.split('?')[0]?.endsWith('/messages') === true
          ? jsonResponse(messagesResponse([completed ? completedTodo : todo]))
          : jsonResponse(actions),
    )
  })
  vi.stubGlobal('fetch', fetcher)
  root = createRoot(container)
  act(() => {
    root?.render(
      <ConversationPane
        session={session}
        refreshKey={0}
        sessionTreeRefreshKey={0}
        onSelectSession={() => undefined}
      />,
    )
  })
  expect(
    await waitForState(
      () => container.querySelector('.chat-todo-card') !== null,
    ),
  ).toBe(true)

  setTextareaValue(
    requireElement(
      container.querySelector<HTMLTextAreaElement>('.chat-todo-comment'),
    ),
    'The page is readable.',
  )
  act(() => {
    requireElement(
      container.querySelector<HTMLButtonElement>(
        '.chat-todo-footer .aw-button--primary',
      ),
    ).click()
  })
  expect(
    await waitForState(
      () =>
        container.querySelector('.chat-todo-completed-comment')?.textContent ===
          'The page is readable.' &&
        container.querySelector('.chat-todo-comment') === null,
    ),
  ).toBe(true)

  expect(requests.filter((request) => request.method === 'POST')).toEqual([
    {
      url: '/api/sessions/sample-session/todos/4/complete',
      method: 'POST',
      body: '{"comment":"The page is readable."}',
    },
  ])
})

it('accepts a subagent report without a description from the messages endpoint', async () => {
  const jsonResponse = (body: unknown) =>
    new Response(JSON.stringify(body), {
      headers: { 'Content-Type': 'application/json' },
    })
  const fetcher = vi
    .fn<typeof fetch>()
    .mockResolvedValueOnce(
      jsonResponse(
        messagesResponse([
          {
            type: 'subagent_report',
            agent_id: 'fixture-agent-id',
            agent_name: 'general-purpose',
            text: 'The fictional task is complete.',
          },
        ]),
      ),
    )
    .mockResolvedValueOnce(jsonResponse(actions))
  vi.stubGlobal('fetch', fetcher)
  root = createRoot(container)
  act(() => {
    root?.render(
      <ConversationPane
        session={session}
        refreshKey={0}
        sessionTreeRefreshKey={0}
        onSelectSession={() => undefined}
      />,
    )
  })
  await act(async () => {
    for (let index = 0; index < 6; index += 1) await Promise.resolve()
  })

  const snapshot = () => ({
    summary: container.querySelector('.chat-tool-summary summary')?.textContent,
    body: container.querySelector('.chat-tool-summary .markdown-message')
      ?.textContent,
    open: container.querySelector('.chat-tool-summary')?.hasAttribute('open'),
    alert: container.querySelector('[role="alert"]')?.textContent ?? null,
  })
  expect(snapshot()).toEqual({
    summary: 'general-purpose',
    body: 'The fictional task is complete.',
    open: false,
    alert: null,
  })
})

it('shows elapsed time while the session is working without an active tool', () => {
  renderView(
    vi.fn().mockResolvedValue(true),
    { ...session, category: 'in_progress', elapsedSeconds: 92 },
    undefined,
    [],
  )

  const snapshot = () => ({
    workingStatus: container.querySelector('.conversation-working-status')
      ?.textContent,
    message: container.querySelector('.conversation-status')?.textContent,
  })

  expect(snapshot()).toEqual({
    workingStatus: 'Working · 1m 32s',
    message: 'No messages yet.',
  })
})

it('shows the agent status reason while the stopped session is waiting', () => {
  renderView(vi.fn().mockResolvedValue(true), {
    ...session,
    category: 'in_progress',
    activityStatus: 'waiting',
    waitingReason: 'waiting for an external build',
  })

  const snapshot = () => ({
    status: container.querySelector('.conversation-working-status')
      ?.textContent,
    workingDotVisible:
      container.querySelector('.conversation-working-dot') !== null,
  })

  expect(snapshot()).toEqual({
    status: 'Waiting · waiting for an external build',
    workingDotVisible: false,
  })
})

it('shows when the stopped session is waiting for background tasks', () => {
  renderView(vi.fn().mockResolvedValue(true), {
    ...session,
    category: 'in_progress',
    activityStatus: 'stopped',
    waitingForBackgroundTasks: true,
  })

  const snapshot = () => ({
    status: container.querySelector('.conversation-working-status')
      ?.textContent,
    workingDotVisible:
      container.querySelector('.conversation-working-dot') !== null,
  })

  expect(snapshot()).toEqual({
    status: 'Waiting for background tasks',
    workingDotVisible: false,
  })
})

it('shows background task descriptions and elapsed times on mobile', () => {
  renderView(
    vi.fn().mockResolvedValue(true),
    { ...session, category: 'in_progress', elapsedSeconds: 92 },
    undefined,
    [],
    actions,
    true,
    [
      {
        id: 'fixture-task-one',
        description: 'Watch the build pipeline',
        elapsedSeconds: 128,
        kind: 'background_task',
      },
      {
        id: 'fixture-task-two',
        description: 'Review the generated report',
        elapsedSeconds: 43,
        kind: 'background_task',
      },
    ],
  )

  act(() => {
    requireElement(
      container.querySelector<HTMLElement>(
        '.conversation-background-tasks summary',
      ),
    ).click()
  })

  const snapshot = () => ({
    summary: container.querySelector('.conversation-background-tasks summary')
      ?.textContent,
    tasks: Array.from(
      container.querySelectorAll('.conversation-background-tasks li'),
      (item) => item.textContent,
    ),
  })
  expect(snapshot()).toEqual({
    summary: 'Background tasks (2)',
    tasks: ['Watch the build pipeline2m 8s', 'Review the generated report43s'],
  })
})

it('shows the compacting status while hiding elapsed time and stale tool details', () => {
  renderView(
    vi.fn().mockResolvedValue(true),
    { ...session, isCompacting: true, elapsedSeconds: 92 },
    undefined,
    [
      {
        type: 'tool_in_progress',
        tool: 'exec',
        target: 'cargo test --manifest-path fixture/Cargo.toml',
      },
    ],
  )

  const getSnapshot = () => ({
    workingStatus: container.querySelector('.conversation-working-status')
      ?.textContent,
    toolItems: container.querySelectorAll('.conversation-working-tool').length,
    inProgressItems: container.querySelectorAll('.tool-in-progress').length,
  })
  expect(getSnapshot()).toEqual({
    workingStatus: 'Compacting conversation…',
    toolItems: 0,
    inProgressItems: 0,
  })
})

it('shows an active tool and hides it from the completed conversation', () => {
  renderView(
    vi.fn().mockResolvedValue(true),
    { ...session, category: 'in_progress', elapsedSeconds: 92 },
    undefined,
    [
      {
        type: 'tool_summary',
        summary: '1 action',
      },
      {
        type: 'tool_in_progress',
        tool: 'exec',
        target: 'cargo test --manifest-path fixture/Cargo.toml',
      },
    ],
    actions,
    true,
  )

  const snapshot = () => ({
    workingStatus: Array.from(
      requireElement(container.querySelector('.conversation-working-status'))
        .children,
    )
      .slice(1)
      .map((child) => child.textContent),
    toolSummary:
      container.querySelector('.chat-tool-summary')?.textContent ?? null,
    inProgressItems: container.querySelectorAll('.tool-in-progress').length,
  })

  expect(snapshot()).toEqual({
    workingStatus: [
      'Working · 1m 32s',
      'execcargo test --manifest-path fixture/Cargo.toml',
    ],
    toolSummary: '1 action',
    inProgressItems: 0,
  })
})

it('hides working status when the session is done', () => {
  renderView(
    vi.fn().mockResolvedValue(true),
    { ...session, category: 'done', elapsedSeconds: 92 },
    undefined,
    [
      {
        type: 'tool_in_progress',
        tool: 'exec',
        target: 'cargo test',
      },
    ],
  )

  const snapshot = () => ({
    workingStatus: container.querySelector('.conversation-working-status'),
    toolSummary:
      container.querySelector('.chat-tool-summary summary')?.textContent ??
      null,
    operations: Array.from(
      container.querySelectorAll('.chat-tool-operations li'),
    ).map((operation) => operation.textContent),
  })

  expect(snapshot()).toEqual({
    workingStatus: null,
    toolSummary: '1 action',
    operations: ['execcargo test✓'],
  })
})

it('keeps background tasks visible while the session waits for input', () => {
  renderView(
    vi.fn().mockResolvedValue(true),
    { ...session, category: 'needs_action' },
    undefined,
    [],
    actions,
    true,
    [
      {
        id: 'fixture-task-one',
        description: 'Watch the build pipeline',
        elapsedSeconds: 128,
        kind: 'background_task',
      },
    ],
  )

  const snapshot = () => ({
    status: container.querySelector('.conversation-working-status')
      ?.textContent,
    workingDotVisible:
      container.querySelector('.conversation-working-dot') !== null,
    backgroundTasks: container.querySelector(
      '.conversation-background-tasks summary',
    )?.textContent,
  })

  expect(snapshot()).toEqual({
    status: 'Background tasks (1)Watch the build pipeline2m 8s',
    workingDotVisible: false,
    backgroundTasks: 'Background tasks (1)',
  })
})

it('accepts classified messages and the active tool from messages endpoint', async () => {
  const jsonResponse = (body: unknown) =>
    new Response(JSON.stringify(body), {
      headers: { 'Content-Type': 'application/json' },
    })
  const fetcher = vi
    .fn<typeof fetch>()
    .mockResolvedValueOnce(
      jsonResponse(
        messagesResponse(
          [
            {
              type: 'peer',
              direction: 'from',
              session_id: 'fixture-peer-session',
              session_title: 'Fixture peer session',
              content: [
                { type: 'text', text: 'A fictional peer update.' },
                {
                  type: 'image',
                  source: '/api/images/fixture-peer-image.png',
                  available: true,
                },
                { type: 'file', name: 'fixture-peer-layout.html' },
              ],
            },
            {
              type: 'event',
              kind: 'armyknife',
              text: 'A fictional event was blocked.',
              failed: true,
            },
            {
              type: 'tool_in_progress',
              tool: 'exec',
              target: 'cargo test',
            },
          ],
          [
            {
              id: 'fixture-task-one',
              description: 'Watch the build pipeline',
              elapsedSeconds: 128,
              kind: 'background_task',
            },
          ],
        ),
      ),
    )
    .mockResolvedValueOnce(
      jsonResponse({
        message: false,
        answerQuestion: false,
        interrupt: false,
        openInTerminal: false,
      }),
    )
  vi.stubGlobal('fetch', fetcher)
  root = createRoot(container)
  act(() => {
    root?.render(
      <ConversationPane
        session={{ ...session, category: 'in_progress', elapsedSeconds: 30 }}
        refreshKey={0}
        sessionTreeRefreshKey={0}
        onSelectSession={() => undefined}
        isMobile
      />,
    )
  })
  expect(
    await waitForState(
      () =>
        container.querySelector('.chat-peer-file')?.textContent ===
        'fixture-peer-layout.html',
    ),
  ).toBe(true)

  const snapshot = () => ({
    workingStatus: container.querySelector('.conversation-working-status')
      ?.children[1]?.textContent,
    activeTool: container.querySelector('.conversation-working-tool')
      ?.textContent,
    peerMessage: container.querySelector('.chat-peer-message')?.textContent,
    peerImage: container
      .querySelector<HTMLImageElement>('.chat-peer-message img')
      ?.getAttribute('src'),
    peerFile: container.querySelector('.chat-peer-file')?.textContent,
    event: Array.from(
      container.querySelectorAll<HTMLElement>('.chat-event'),
      (element) => ({
        text: element.textContent,
        failed: element.getAttribute('data-failed'),
      }),
    ),
    backgroundTasks: container.querySelector(
      '.conversation-background-tasks summary',
    )?.textContent,
    headerRunningCount: container.querySelector(
      '.conversation-status-panel-count',
    )?.textContent,
  })
  expect(snapshot()).toEqual({
    workingStatus: 'Working · 30s',
    activeTool: 'execcargo test',
    peerMessage:
      'FROMFixture peer session (ended)A fictional peer update.fixture-peer-layout.html',
    peerImage: '/api/images/fixture-peer-image.png',
    peerFile: 'fixture-peer-layout.html',
    event: [{ text: 'A fictional event was blocked.', failed: 'true' }],
    backgroundTasks: 'Background tasks (1)',
    headerRunningCount: '1',
  })
})

it('shows running tasks in the desktop status panel and counts them in the header', async () => {
  const jsonResponse = (body: unknown) =>
    new Response(JSON.stringify(body), {
      headers: { 'Content-Type': 'application/json' },
    })
  const fetcher = vi
    .fn<typeof fetch>()
    .mockResolvedValueOnce(
      jsonResponse(
        messagesResponse(
          [],
          [
            {
              id: 'fixture-agent-task',
              description: 'Inspect the fictional module',
              elapsedSeconds: 7_200,
              kind: 'subagent',
              agentType: 'FixtureExplorer',
            },
            {
              id: 'fixture-shell-task',
              description: 'cargo test -p fictional-crate',
              elapsedSeconds: 7_200,
              kind: 'background_task',
            },
          ],
        ),
      ),
    )
    .mockResolvedValueOnce(jsonResponse(actions))
  vi.stubGlobal('fetch', fetcher)
  root = createRoot(container)
  act(() => {
    root?.render(
      <ConversationPane
        session={{
          ...session,
          category: 'needs_action',
          elapsedSeconds: 30,
        }}
        refreshKey={0}
        sessionTreeRefreshKey={0}
        onSelectSession={() => undefined}
        statusPanelOpen
      />,
    )
  })
  await act(async () => {
    for (let index = 0; index < 6; index += 1) await Promise.resolve()
  })

  const snapshot = () => ({
    tabs: Array.from(container.querySelectorAll('[role="tab"]'), (tab) => ({
      label: tab.textContent,
      selected: tab.getAttribute('aria-selected'),
    })),
    selectedTab: container.querySelector('[role="tab"][aria-selected="true"]')
      ?.textContent,
    descriptionTitles: Array.from(
      container.querySelectorAll<HTMLElement>(
        '.running-task-subagent-description, .running-task-description',
      ),
      (description) => description.getAttribute('title'),
    ),
    sections: Array.from(
      container.querySelectorAll('.running-tasks-group h2'),
      (heading) => heading.textContent,
    ),
    tasks: Array.from(
      container.querySelectorAll('.running-task'),
      (task) => task.textContent,
    ),
    panelToggle: container.querySelector('.conversation-status-panel-toggle')
      ?.textContent,
    workingStatus: container.querySelector('.conversation-working-status')
      ?.textContent,
    legacyTaskList:
      container.querySelector('.conversation-background-tasks') !== null,
  })
  expect(snapshot()).toEqual({
    tabs: [{ label: 'Running2', selected: 'true' }],
    selectedTab: 'Running2',
    descriptionTitles: [
      'Inspect the fictional module',
      'cargo test -p fictional-crate',
    ],
    sections: ['SUBAGENTS', 'BACKGROUND TASKS'],
    tasks: [
      '◐FixtureExplorerInspect the fictional module2h 0m',
      '▶cargo test -p fictional-crateOutput2h 0m',
    ],
    panelToggle: '2',
    workingStatus: undefined,
    legacyTaskList: false,
  })
})

it('keeps the Decisions tab active when running tasks appear', async () => {
  const jsonResponse = (body: unknown) =>
    new Response(JSON.stringify(body), {
      headers: { 'Content-Type': 'application/json' },
    })
  const decision: ChatItem = {
    type: 'decision',
    seq: 1,
    title: 'Choose a retry policy',
    why: 'The service needs a predictable recovery path.',
    considered: ['Retry using a fixed limit'],
    easyToChange: 'One configuration value.',
    review: null,
    createdAt: '2026-11-11T03:25:59Z',
  }
  const runningTask: BackgroundTask = {
    id: 'fixture-started-subagent',
    description: 'Inspect the synthetic module',
    elapsedSeconds: 7,
    kind: 'subagent',
    agentType: 'FixtureExplorer',
  }
  const fetcher = vi
    .fn<typeof fetch>()
    .mockResolvedValueOnce(jsonResponse(messagesResponse([decision])))
    .mockResolvedValueOnce(jsonResponse(actions))
    .mockResolvedValueOnce(
      jsonResponse(messagesResponse([decision], [runningTask])),
    )
    .mockResolvedValueOnce(jsonResponse(actions))
  vi.stubGlobal('fetch', fetcher)
  root = createRoot(container)
  const renderConversation = (refreshKey: number) => {
    root?.render(
      <ConversationPane
        session={{
          ...session,
          category: 'needs_action',
          decisionCount: 1,
          unreviewedDecisionCount: 1,
        }}
        refreshKey={refreshKey}
        sessionTreeRefreshKey={0}
        onSelectSession={() => undefined}
        statusPanelOpen
      />,
    )
  }
  act(() => {
    renderConversation(0)
  })
  await act(async () => {
    for (let index = 0; index < 12; index += 1) await Promise.resolve()
  })

  const snapshot = () => ({
    tabs: Array.from(container.querySelectorAll('[role="tab"]'), (tab) => ({
      label: tab.textContent,
      selected: tab.getAttribute('aria-selected'),
    })),
    selectedTab: container.querySelector('[role="tab"][aria-selected="true"]')
      ?.textContent,
    decisionHeading: container.querySelector('.decisions-panel-header h2')
      ?.textContent,
  })
  const beforeTaskStarts = snapshot()
  act(() => {
    renderConversation(1)
  })
  await act(async () => {
    for (let index = 0; index < 12; index += 1) await Promise.resolve()
  })

  const states = () => ({ beforeTaskStarts, afterTaskStarts: snapshot() })
  expect(states()).toEqual({
    beforeTaskStarts: {
      tabs: [{ label: 'Decisions1', selected: 'true' }],
      selectedTab: 'Decisions1',
      decisionHeading: 'Decisions in this session',
    },
    afterTaskStarts: {
      tabs: [
        { label: 'Running1', selected: 'false' },
        { label: 'Decisions1', selected: 'true' },
      ],
      selectedTab: 'Decisions1',
      decisionHeading: 'Decisions in this session',
    },
  })
})

it('selects Running by default and switches to Decisions from its tab', () => {
  const decision: ChatItem = {
    type: 'decision',
    seq: 1,
    title: 'Choose a retry policy',
    why: 'The service needs a predictable recovery path.',
    considered: ['Retry using a fixed limit'],
    easyToChange: 'One configuration value.',
    review: null,
    createdAt: '2026-11-11T03:25:59Z',
  }
  const runningTask: BackgroundTask = {
    id: 'fixture-running-subagent',
    description: 'Inspect the synthetic module',
    elapsedSeconds: 7,
    kind: 'subagent',
    agentType: 'FixtureExplorer',
  }
  const reviewSession: Session = {
    ...session,
    category: 'needs_action',
    decisionCount: 1,
    unreviewedDecisionCount: 1,
  }
  function StatusPanelHarness() {
    const statusPanel = useConversationStatusPanel({
      session: reviewSession,
      messages: [decision],
      backgroundTasks: [runningTask],
      onAction: () => Promise.resolve(true),
      onCloseStatusPanel: () => undefined,
    })
    return (
      <>
        <ConversationPaneView
          session={reviewSession}
          messages={[decision]}
          backgroundTasks={[runningTask]}
          actions={actions}
          loading={false}
          error={null}
          onAction={() => Promise.resolve(true)}
          onSelectSession={() => undefined}
          statusPanelOpen
          selectedDecisionSeq={statusPanel.selectedDecisionSeq}
          onSelectDecision={statusPanel.onSelectDecision}
        />
        <SessionStatusPanel
          tabs={statusPanel.tabs}
          activeTabId={statusPanel.activeTabId}
          onTabChange={statusPanel.onTabChange}
          onClose={() => undefined}
        />
      </>
    )
  }
  root = createRoot(container)
  act(() => {
    root?.render(<StatusPanelHarness />)
  })

  const snapshot = () => ({
    tabs: Array.from(container.querySelectorAll('[role="tab"]'), (tab) => ({
      label: tab.textContent,
      selected: tab.getAttribute('aria-selected'),
    })),
    selectedTab: container.querySelector('[role="tab"][aria-selected="true"]')
      ?.textContent,
    sections: Array.from(
      container.querySelectorAll('.running-tasks-group h2'),
      (heading) => heading.textContent,
    ),
    decisionHeading: container.querySelector('.decisions-panel-header h2')
      ?.textContent,
  })
  const runningState = snapshot()
  act(() => {
    requireElement(
      container.querySelectorAll<HTMLButtonElement>('[role="tab"]').item(1),
    ).click()
  })
  const states = () => ({ runningState, decisionsState: snapshot() })
  expect(states()).toEqual({
    runningState: {
      tabs: [
        { label: 'Running1', selected: 'true' },
        { label: 'Decisions1', selected: 'false' },
      ],
      selectedTab: 'Running1',
      sections: ['SUBAGENTS'],
      decisionHeading: undefined,
    },
    decisionsState: {
      tabs: [
        { label: 'Running1', selected: 'false' },
        { label: 'Decisions1', selected: 'true' },
      ],
      selectedTab: 'Decisions1',
      sections: [],
      decisionHeading: 'Decisions in this session',
    },
  })
})

it('shows the unreviewed count instead of the total on the Decisions tab', () => {
  const decisions: ChatItem[] = [
    {
      type: 'decision',
      seq: 1,
      title: 'Choose a retry limit',
      why: 'Keep recovery bounded.',
      considered: ['Retry indefinitely'],
      easyToChange: 'Update one setting.',
      review: null,
      createdAt: '2026-11-11T03:25:59Z',
    },
    {
      type: 'decision',
      seq: 2,
      title: 'Keep the current interval',
      why: 'It already meets the service limit.',
      considered: ['Use a shorter interval'],
      easyToChange: 'Update one setting.',
      review: 'ok',
      createdAt: '2026-11-11T03:27:59Z',
    },
  ]
  const reviewSession: Session = {
    ...session,
    decisionCount: 2,
    unreviewedDecisionCount: 1,
  }
  function StatusPanelHarness() {
    const statusPanel = useConversationStatusPanel({
      session: reviewSession,
      messages: decisions,
      backgroundTasks: [],
      onAction: () => Promise.resolve(true),
      onCloseStatusPanel: () => undefined,
    })
    return (
      <SessionStatusPanel
        tabs={statusPanel.tabs}
        activeTabId={statusPanel.activeTabId}
        onTabChange={statusPanel.onTabChange}
        onClose={() => undefined}
      />
    )
  }
  root = createRoot(container)
  act(() => {
    root?.render(<StatusPanelHarness />)
  })

  const snapshot = () => ({
    tab: container.querySelector('[role="tab"]')?.textContent,
    decisions: Array.from(
      container.querySelectorAll('.decision-card--panel'),
      (card) => card.getAttribute('data-review'),
    ),
    progress: container.querySelector('.decisions-panel-header span')
      ?.textContent,
  })
  expect(snapshot()).toEqual({
    tab: 'Decisions1',
    decisions: ['unreviewed'],
    progress: '1 / 2 reviewed',
  })
})

it('prefers crit when other status tabs are available', () => {
  const decision: ChatItem = {
    type: 'decision',
    seq: 1,
    title: 'Choose a retry policy',
    why: 'The service needs a predictable recovery path.',
    considered: ['Retry using a fixed limit'],
    easyToChange: 'One configuration value.',
    review: null,
    createdAt: '2026-11-11T03:25:59Z',
  }
  const runningTask: BackgroundTask = {
    id: 'fixture-running-subagent',
    description: 'Inspect the synthetic module',
    elapsedSeconds: 7,
    kind: 'subagent',
    agentType: 'FixtureExplorer',
  }
  function StatusPanelHarness() {
    const statusPanel = useConversationStatusPanel({
      session: {
        ...session,
        category: 'needs_action',
        kind: 'Review',
        critUrl: 'http://127.0.0.1:5000/review/sample',
        decisionCount: 1,
        unreviewedDecisionCount: 1,
      },
      messages: [decision],
      backgroundTasks: [runningTask],
      onAction: () => Promise.resolve(true),
      onCloseStatusPanel: () => undefined,
    })
    return (
      <SessionStatusPanel
        tabs={statusPanel.tabs}
        activeTabId={statusPanel.activeTabId}
        onTabChange={statusPanel.onTabChange}
        onClose={() => undefined}
      />
    )
  }
  root = createRoot(container)
  act(() => {
    root?.render(<StatusPanelHarness />)
  })

  const snapshot = () => ({
    tabs: Array.from(container.querySelectorAll('[role="tab"]'), (tab) => ({
      label: tab.textContent,
      selected: tab.getAttribute('aria-selected'),
    })),
    selectedTab: container.querySelector('[role="tab"][aria-selected="true"]')
      ?.textContent,
    critPane: container.querySelector('.crit-pane') !== null,
  })
  expect(snapshot()).toEqual({
    tabs: [
      { label: 'Running1', selected: 'false' },
      { label: 'Decisions1', selected: 'false' },
      { label: 'crit', selected: 'true' },
    ],
    selectedTab: 'crit',
    critPane: true,
  })
})

it('posts decision reviews and reflects the refreshed review state', async () => {
  const jsonResponse = (body: unknown) =>
    new Response(JSON.stringify(body), {
      headers: { 'Content-Type': 'application/json' },
    })
  const firstDecision: ChatItem = {
    type: 'decision',
    seq: 1,
    title: 'Use a fixed retry limit',
    why: 'The current service has no reliable timing data.',
    considered: ['Retry using the latest duration'],
    easyToChange: 'One configuration value in the CI workflow.',
    review: null,
    createdAt: '2026-11-11T03:25:59Z',
  }
  const secondDecision: ChatItem = {
    ...firstDecision,
    seq: 2,
    title: 'Keep the current backoff interval',
    review: null,
  }
  const fetcher = vi
    .fn<typeof fetch>()
    .mockResolvedValueOnce(
      jsonResponse(messagesResponse([firstDecision, secondDecision])),
    )
    .mockResolvedValueOnce(jsonResponse(actions))
    .mockResolvedValueOnce(new Response(null, { status: 204 }))
    .mockResolvedValueOnce(
      jsonResponse(
        messagesResponse([{ ...firstDecision, review: 'ok' }, secondDecision]),
      ),
    )
    .mockResolvedValueOnce(jsonResponse(actions))
    .mockResolvedValueOnce(new Response(null, { status: 204 }))
    .mockResolvedValueOnce(
      jsonResponse(
        messagesResponse([
          { ...firstDecision, review: 'ok' },
          { ...secondDecision, review: 'change' },
        ]),
      ),
    )
    .mockResolvedValueOnce(jsonResponse(actions))
  vi.stubGlobal('fetch', fetcher)
  root = createRoot(container)
  act(() => {
    root?.render(
      <ConversationPane
        session={{ ...session, decisionCount: 2, unreviewedDecisionCount: 2 }}
        refreshKey={0}
        sessionTreeRefreshKey={0}
        onSelectSession={() => undefined}
      />,
    )
  })
  await act(async () => {
    for (let index = 0; index < 12; index += 1) await Promise.resolve()
  })

  const cards = () => container.querySelectorAll<HTMLElement>('.decision-card')
  const summary = (index: number) =>
    requireElement(
      container
        .querySelectorAll<HTMLButtonElement>('.decision-card-summary')
        .item(index),
    )
  act(() => {
    summary(0).click()
  })
  act(() => {
    requireElement(
      cards()
        .item(0)
        .querySelector<HTMLButtonElement>('.decision-card-buttons button'),
    ).click()
  })
  await act(async () => {
    for (let index = 0; index < 12; index += 1) await Promise.resolve()
  })

  act(() => {
    summary(1).click()
  })
  act(() => {
    requireElement(
      cards()
        .item(1)
        .querySelector<HTMLButtonElement>(
          '.decision-card-buttons button + button',
        ),
    ).click()
  })
  setTextareaValue(
    requireElement(container.querySelector<HTMLTextAreaElement>('textarea')),
    'Add a timing sample first',
  )
  await act(async () => {
    requireElement(
      cards()
        .item(1)
        .querySelector<HTMLButtonElement>('.decision-card-change-form button'),
    ).click()
    for (let index = 0; index < 12; index += 1) await Promise.resolve()
  })

  const postRequests = fetcher.mock.calls
    .filter(([, init]) => init?.method === 'POST')
    .map(([input, init]) => ({
      url:
        typeof input === 'string'
          ? input
          : input instanceof URL
            ? input.href
            : input.url,
      method: init?.method,
      body: init?.body ?? null,
    }))
  const reviewState = Array.from(
    container.querySelectorAll('.decision-card-review-state'),
    (element) => element.textContent,
  )
  const positions = Array.from(
    container.querySelectorAll('.decision-card-position'),
    (element) => element.textContent,
  )
  const snapshot = () => ({ postRequests, reviewState, positions })
  expect(snapshot()).toEqual({
    postRequests: [
      {
        url: '/api/sessions/sample-session/decisions/1/ok',
        method: 'POST',
        body: null,
      },
      {
        url: '/api/sessions/sample-session/decisions/2/change',
        method: 'POST',
        body: JSON.stringify({ comment: 'Add a timing sample first' }),
      },
    ],
    reviewState: ['Reviewed · OK', 'Change requested'],
    positions: ['1 of 2', '2 of 2'],
  })
})

it('shows the status panel toggle and reports its open state', () => {
  const onToggleStatusPanel = vi.fn<() => void>()
  renderView(
    vi.fn(() => Promise.resolve(true)),
    session,
    () => undefined,
    [question],
    actions,
    false,
    [],
    [],
    true,
    onToggleStatusPanel,
  )
  const toggle = requireElement(
    container.querySelector<HTMLButtonElement>(
      '.conversation-status-panel-toggle',
    ),
  )
  act(() => {
    toggle.click()
  })

  const snapshot = () => ({
    label: toggle.getAttribute('aria-label'),
    icon: toggle.querySelector('svg')?.getAttribute('class'),
    expanded: toggle.getAttribute('aria-expanded'),
    controls: toggle.getAttribute('aria-controls'),
    calls: onToggleStatusPanel.mock.calls,
  })
  expect(snapshot()).toEqual({
    label: 'Close session status panel',
    icon: 'lucide lucide-panel-right-close',
    expanded: 'true',
    controls: 'session-status-panel',
    calls: [[]],
  })
})

it('shows the open action when the status panel is closed', () => {
  renderView(vi.fn(() => Promise.resolve(true)))
  const toggle = requireElement(
    container.querySelector<HTMLButtonElement>(
      '.conversation-status-panel-toggle',
    ),
  )

  const snapshot = () => ({
    label: toggle.getAttribute('aria-label'),
    icon: toggle.querySelector('svg')?.getAttribute('class'),
    expanded: toggle.getAttribute('aria-expanded'),
  })
  expect(snapshot()).toEqual({
    label: 'Open session status panel',
    icon: 'lucide lucide-panel-right-open',
    expanded: 'false',
  })
})

it('keeps Decisions in the status panel and toggles Starred from the header', () => {
  const onToggleToday = vi.fn<(isToday: boolean) => void>()
  renderView(
    vi.fn(() => Promise.resolve(true)),
    {
      ...session,
      decisionCount: 2,
      unreviewedDecisionCount: 0,
    },
    undefined,
    [],
    actions,
    false,
    [],
    [],
    false,
    undefined,
    undefined,
    { isToday: false, onToggleToday },
  )
  const toggle = requireElement(
    container.querySelector<HTMLButtonElement>('.conversation-today-toggle'),
  )
  act(() => {
    toggle.click()
  })

  const snapshot = () => ({
    noDecisionsAction: container.querySelector(
      '.conversation-decisions-toggle',
    ),
    label: toggle.getAttribute('aria-label'),
    icon: toggle.textContent,
    pressed: toggle.getAttribute('aria-pressed'),
    title: toggle.getAttribute('title'),
    calls: onToggleToday.mock.calls,
  })
  expect(snapshot()).toEqual({
    noDecisionsAction: null,
    label: 'Add to Starred',
    icon: '☆',
    pressed: 'false',
    title: 'Add to Starred',
    calls: [[true]],
  })
})

it('keeps the Starred toggle available when a session has no decisions', () => {
  renderView(vi.fn(() => Promise.resolve(true)))

  const snapshot = () => ({
    star: container.querySelector('.conversation-today-toggle') !== null,
    decisions: container.querySelector('.conversation-decisions-toggle'),
  })
  expect(snapshot()).toEqual({ star: true, decisions: null })
})

it('removes a session from Starred from the desktop header', () => {
  const onToggleToday = vi.fn<(isToday: boolean) => void>()
  renderView(
    vi.fn(() => Promise.resolve(true)),
    session,
    undefined,
    [],
    actions,
    false,
    [],
    [],
    false,
    undefined,
    undefined,
    { isToday: true, onToggleToday },
  )
  const toggle = requireElement(
    container.querySelector<HTMLButtonElement>('.conversation-today-toggle'),
  )
  act(() => {
    toggle.click()
  })

  const snapshot = () => ({
    label: toggle.getAttribute('aria-label'),
    pressed: toggle.getAttribute('aria-pressed'),
    title: toggle.getAttribute('title'),
    calls: onToggleToday.mock.calls,
  })
  expect(snapshot()).toEqual({
    label: 'Remove from Starred',
    pressed: 'true',
    title: 'Remove from Starred',
    calls: [[false]],
  })
})

it('opens and closes the existing status panel over the mobile conversation', async () => {
  const jsonResponse = (body: unknown) =>
    new Response(JSON.stringify(body), {
      headers: { 'Content-Type': 'application/json' },
    })
  const decision: ChatItem = {
    type: 'decision',
    seq: 1,
    title: 'Use a bounded retry limit',
    why: 'The service does not expose reliable timing data yet.',
    considered: ['Retry until the service recovers'],
    easyToChange: 'Update the retry limit configuration.',
    review: null,
    createdAt: '2026-11-11T03:25:59Z',
  }
  vi.stubGlobal(
    'fetch',
    vi
      .fn<typeof fetch>()
      .mockResolvedValueOnce(jsonResponse(messagesResponse([decision])))
      .mockResolvedValueOnce(jsonResponse(actions)),
  )
  function ToggleHarness() {
    const [statusPanelOpen, setStatusPanelOpen] = useState(false)
    return (
      <ConversationPane
        session={session}
        refreshKey={0}
        sessionTreeRefreshKey={0}
        onSelectSession={() => undefined}
        isMobile
        statusPanelOpen={statusPanelOpen}
        onToggleStatusPanel={() => {
          setStatusPanelOpen((open) => !open)
        }}
        onCloseStatusPanel={() => {
          setStatusPanelOpen(false)
        }}
      />
    )
  }
  root = createRoot(container)
  act(() => root?.render(<ToggleHarness />))
  await act(async () => {
    for (let index = 0; index < 12; index += 1) await Promise.resolve()
  })
  act(() => {
    requireElement(
      container.querySelector<HTMLButtonElement>(
        '.mobile-conversation-status-toggle',
      ),
    ).click()
  })

  const opened = {
    overlay: container.querySelector('.mobile-session-status-overlay') !== null,
    selectedTab: container.querySelector('[role="tab"][aria-selected="true"]')
      ?.textContent,
    decisionHeading: container.querySelector('.decisions-panel-header h2')
      ?.textContent,
  }
  act(() => {
    requireElement(
      container.querySelector<HTMLButtonElement>(
        '.mobile-session-status-overlay .session-status-panel-close',
      ),
    ).click()
  })
  const closed = {
    overlay: container.querySelector('.mobile-session-status-overlay') !== null,
    conversation: container.querySelector('.conversation-pane') !== null,
  }

  const snapshot = () => ({ opened, closed })
  expect(snapshot()).toEqual({
    opened: {
      overlay: true,
      selectedTab: 'Decisions1',
      decisionHeading: 'Decisions in this session',
    },
    closed: { overlay: false, conversation: true },
  })
})

it('shows the paused state in the conversation header', () => {
  renderView(
    vi.fn().mockResolvedValue(true),
    { ...session, category: 'done', kind: null, isPaused: true },
    undefined,
    [],
    null,
  )

  const snapshot = () => ({
    title: container.querySelector('.conversation-header h1')?.textContent,
    paused: container.querySelector('.conversation-header [data-slot="chip"]')
      ?.textContent,
  })
  expect(snapshot()).toEqual({
    title: 'Choose a retry policy',
    paused: 'Paused',
  })
})

it('shows the paused state in the mobile conversation header', () => {
  renderView(
    vi.fn().mockResolvedValue(true),
    { ...session, category: 'done', kind: null, isPaused: true },
    undefined,
    [],
    null,
    true,
  )

  const snapshot = () => ({
    mobile: container
      .querySelector('.conversation-pane')
      ?.getAttribute('data-mobile'),
    title: container.querySelector('.mobile-conversation-heading h1')
      ?.textContent,
    paused: container.querySelector(
      '.mobile-conversation-heading [data-slot="chip"]',
    )?.textContent,
  })
  expect(snapshot()).toEqual({
    mobile: 'true',
    title: 'Choose a retry policy',
    paused: 'Paused',
  })
})

it('offers the status panel from the mobile header and shows running task count', () => {
  const onToggleStatusPanel = vi.fn<() => void>()
  renderView(
    vi.fn(() => Promise.resolve(true)),
    session,
    undefined,
    [],
    actions,
    true,
    [
      {
        id: 'sample-background-task',
        description: 'Inspect the retry flow',
        elapsedSeconds: 10,
        kind: 'subagent',
        agentType: 'Explorer',
      },
    ],
    [],
    false,
    onToggleStatusPanel,
  )
  const toggle = requireElement(
    container.querySelector<HTMLButtonElement>(
      '.mobile-conversation-status-toggle',
    ),
  )

  act(() => {
    toggle.click()
  })

  const snapshot = () => ({
    expanded: toggle.getAttribute('aria-expanded'),
    controls: toggle.getAttribute('aria-controls'),
    count: toggle.querySelector('.conversation-status-panel-count')
      ?.textContent,
    calls: onToggleStatusPanel.mock.calls,
  })
  expect(snapshot()).toEqual({
    expanded: 'false',
    controls: 'session-status-panel',
    count: '1',
    calls: [[]],
  })
})

it('does not tell mobile review sessions to open crit on a Mac', () => {
  renderView(
    vi.fn(() => Promise.resolve(true)),
    { ...session, kind: 'Review' },
    undefined,
    [],
    actions,
    true,
  )

  const snapshot = () => ({
    mobileNotice: container.querySelector('.mobile-review-note')?.textContent,
  })
  expect(snapshot()).toEqual({ mobileNotice: undefined })
})

it('keeps the status panel toggle available without a waiting crit URL', () => {
  renderView(
    vi.fn(() => Promise.resolve(true)),
    {
      ...session,
      category: 'needs_action',
      kind: 'Review',
      critUrl: null,
    },
  )
  const missingUrl =
    container.querySelector('.conversation-status-panel-toggle') !== null

  renderView(
    vi.fn(() => Promise.resolve(true)),
    {
      ...session,
      category: 'done',
      kind: 'Review',
      critUrl: 'http://127.0.0.1:5000/review/sample',
    },
  )
  const notWaiting =
    container.querySelector('.conversation-status-panel-toggle') !== null

  const snapshot = () => ({ missingUrl, notWaiting })
  expect(snapshot()).toEqual({
    missingUrl: true,
    notWaiting: true,
  })
})

it('directs Codex questions and permissions to the terminal', () => {
  const onAction = vi.fn(() => Promise.resolve(true))
  const codexActions: SessionActions = {
    message: true,
    answerQuestion: false,
    interrupt: true,
    openInTerminal: true,
  }
  renderView(
    onAction,
    { ...session, engine: 'codex' },
    undefined,
    [
      question,
      {
        type: 'permission',
        requestId: 'permission-request-codex',
        tool: 'Shell',
        description: 'Run the project formatter.',
      },
    ],
    codexActions,
  )
  let shortcutEvent: KeyboardEvent | null = null
  act(() => {
    shortcutEvent = dispatchKey(window, '1')
  })
  const terminalButtons = Array.from(
    container.querySelectorAll<HTMLButtonElement>(
      '.conversation-terminal-fallback button',
    ),
  )
  act(() => {
    terminalButtons.forEach((button) => {
      button.click()
    })
  })

  const snapshot = () => ({
    disabledChoices: Array.from(
      container.querySelectorAll<HTMLButtonElement>(
        '.chat-question-choices button',
      ),
      (button) => button.disabled,
    ),
    guidance: Array.from(
      container.querySelectorAll('.conversation-terminal-fallback span'),
      (element) => element.textContent,
    ),
    terminalButtons: terminalButtons.map((button) => ({
      label: button.getAttribute('aria-label'),
      title: button.title,
      text: button.textContent,
      hasIcon: button.querySelector('svg') !== null,
    })),
    composerDisabled: requireElement(
      container.querySelector<HTMLTextAreaElement>('textarea'),
    ).disabled,
    shortcutDefaultPrevented: shortcutEvent?.defaultPrevented,
    actionCalls: onAction.mock.calls,
  })
  expect(snapshot()).toEqual({
    disabledChoices: [true, true],
    guidance: [
      'Answer in terminal to continue.',
      'Review this permission in terminal.',
    ],
    terminalButtons: [
      {
        label: 'Open in terminal',
        title: 'Open in terminal',
        text: '',
        hasIcon: true,
      },
      {
        label: 'Open in terminal',
        title: 'Open in terminal',
        text: '',
        hasIcon: true,
      },
    ],
    composerDisabled: false,
    shortcutDefaultPrevented: false,
    actionCalls: [['focus'], ['focus']],
  })
})

it('keeps the message draft when sending fails', async () => {
  const onAction = vi.fn(() => Promise.resolve(false))
  renderView(onAction)
  const textarea = requireElement(
    container.querySelector<HTMLTextAreaElement>('textarea'),
  )
  setTextareaValue(textarea, 'Retain this draft')
  act(() => {
    dispatchKey(textarea, 'Enter')
  })
  await act(async () => {
    await Promise.resolve()
  })

  const snapshot = () => ({
    draft: textarea.value,
    storedDraft: window.localStorage.getItem(
      'agent-watch-web:composer-draft:sample-session',
    ),
    actions: onAction.mock.calls,
  })
  expect(snapshot()).toEqual({
    draft: 'Retain this draft',
    storedDraft: 'Retain this draft',
    actions: [['messages', { content: 'Retain this draft' }]],
  })
})

it('sends pasted file data with the message and keeps it when sending fails', async () => {
  const onAction = vi.fn(() => Promise.resolve(false))
  renderView(onAction)
  const textarea = requireElement(
    container.querySelector<HTMLTextAreaElement>('textarea'),
  )
  const file = new File(['payload'], 'clipboard.bin', {
    type: 'application/octet-stream',
  })
  const pasteEvent = new Event('paste', { bubbles: true, cancelable: true })
  Object.defineProperty(pasteEvent, 'clipboardData', {
    value: { files: [file] },
  })
  act(() => {
    void textarea.dispatchEvent(pasteEvent)
  })
  expect(
    await waitForState(
      () =>
        container.querySelector('.composer-attachment span')?.textContent ===
        file.name,
    ),
  ).toBe(true)
  act(() => {
    void dispatchKey(textarea, 'Enter')
  })
  expect(await waitForState(() => onAction.mock.calls.length === 1)).toBe(true)

  const snapshot = () => ({
    actionCalls: onAction.mock.calls,
    attachmentNames: Array.from(
      container.querySelectorAll('.composer-attachment span'),
      (element) => element.textContent,
    ),
  })
  expect(snapshot()).toEqual({
    actionCalls: [
      [
        'messages',
        {
          content: '',
          attachments: [
            {
              name: 'clipboard.bin',
              mimeType: 'application/octet-stream',
              size: 7,
              data: 'cGF5bG9hZA==',
            },
          ],
        },
      ],
    ],
    attachmentNames: ['clipboard.bin'],
  })
})

it('attaches dropped files through the composer and sends their data', async () => {
  const onAction = vi.fn(() => Promise.resolve(true))
  renderView(onAction)
  const file = new File(['payload'], 'dropped.png', { type: 'image/png' })
  const pane = requireElement(container.querySelector('.conversation-pane'))
  const dataTransfer = { types: ['Files'], files: [file], dropEffect: 'none' }
  const dragEnter = new Event('dragenter', { bubbles: true, cancelable: true })
  Object.defineProperty(dragEnter, 'dataTransfer', { value: dataTransfer })
  act(() => {
    pane.dispatchEvent(dragEnter)
  })
  const targetVisibleDuringDrag = container.querySelector(
    '.conversation-drop-target',
  )?.textContent
  const drop = new Event('drop', { bubbles: true, cancelable: true })
  Object.defineProperty(drop, 'dataTransfer', { value: dataTransfer })
  act(() => {
    pane.dispatchEvent(drop)
  })
  expect(
    await waitForState(
      () =>
        container.querySelector('.composer-attachment span')?.textContent ===
        file.name,
    ),
  ).toBe(true)
  const textarea = requireElement(
    container.querySelector<HTMLTextAreaElement>('textarea'),
  )
  act(() => {
    dispatchKey(textarea, 'Enter')
  })
  expect(
    await waitForState(
      () =>
        onAction.mock.calls.length === 1 &&
        container.querySelector('.composer-attachment') === null,
    ),
  ).toBe(true)

  const snapshot = () => ({
    targetVisibleDuringDrag,
    targetAfterDrop:
      container.querySelector('.conversation-drop-target')?.textContent ?? null,
    dropPrevented: drop.defaultPrevented,
    attachmentNames: Array.from(
      container.querySelectorAll('.composer-attachment span'),
      (element) => element.textContent,
    ),
    actionCalls: onAction.mock.calls,
  })
  expect(snapshot()).toEqual({
    targetVisibleDuringDrag: 'Drop files to attach',
    targetAfterDrop: null,
    dropPrevented: true,
    attachmentNames: [],
    actionCalls: [
      [
        'messages',
        {
          content: '',
          attachments: [
            {
              name: 'dropped.png',
              mimeType: 'image/png',
              size: 7,
              data: 'cGF5bG9hZA==',
            },
          ],
        },
      ],
    ],
  })
})

it('hides the drop target when a file drag leaves the conversation', () => {
  const onAction = vi.fn(() => Promise.resolve(true))
  renderView(onAction)
  const pane = requireElement(container.querySelector('.conversation-pane'))
  const dataTransfer = { types: ['Files'], files: [] }
  const dragEnter = new Event('dragenter', { bubbles: true, cancelable: true })
  Object.defineProperty(dragEnter, 'dataTransfer', { value: dataTransfer })
  act(() => {
    pane.dispatchEvent(dragEnter)
  })
  const targetAfterEnter = container.querySelector(
    '.conversation-drop-target',
  )?.textContent
  const dragLeave = new Event('dragleave', { bubbles: true, cancelable: true })
  Object.defineProperty(dragLeave, 'dataTransfer', { value: dataTransfer })
  act(() => {
    pane.dispatchEvent(dragLeave)
  })

  const snapshot = () => ({
    targetAfterEnter,
    targetAfterLeave:
      container.querySelector('.conversation-drop-target')?.textContent ?? null,
  })
  expect(snapshot()).toEqual({
    targetAfterEnter: 'Drop files to attach',
    targetAfterLeave: null,
  })
})

it('clears pasted attachments after a successful send', async () => {
  const onAction = vi.fn(() => Promise.resolve(true))
  renderView(onAction)
  const file = new File(['payload'], 'clipboard.bin', {
    type: 'application/octet-stream',
  })
  const textarea = requireElement(
    container.querySelector<HTMLTextAreaElement>('textarea'),
  )
  const pasteEvent = new Event('paste', { bubbles: true, cancelable: true })
  Object.defineProperty(pasteEvent, 'clipboardData', {
    value: { files: [file] },
  })
  act(() => {
    void textarea.dispatchEvent(pasteEvent)
  })
  expect(
    await waitForState(
      () =>
        container.querySelector('.composer-attachment span')?.textContent ===
        file.name,
    ),
  ).toBe(true)
  act(() => {
    void dispatchKey(textarea, 'Enter')
  })
  expect(
    await waitForState(
      () =>
        onAction.mock.calls.length === 1 &&
        container.querySelector('.composer-attachment') === null,
    ),
  ).toBe(true)

  const snapshot = () => ({
    actionCalls: onAction.mock.calls,
    attachmentNames: Array.from(
      container.querySelectorAll('.composer-attachment span'),
      (element) => element.textContent,
    ),
  })
  expect(snapshot()).toEqual({
    actionCalls: [
      [
        'messages',
        {
          content: '',
          attachments: [
            {
              name: 'clipboard.bin',
              mimeType: 'application/octet-stream',
              size: 7,
              data: 'cGF5bG9hZA==',
            },
          ],
        },
      ],
    ],
    attachmentNames: [],
  })
})

it('names the attachment that crosses the message size limit before sending', async () => {
  const onAction = vi.fn(() => Promise.resolve(false))
  renderView(onAction)
  const firstFile = new File(['first'], 'first.bin', {
    type: 'application/octet-stream',
  })
  const limitCrossingFile = new File(['second'], 'limit-crossing.bin', {
    type: 'application/octet-stream',
  })
  Object.defineProperty(firstFile, 'size', {
    value: 30 * 1024 * 1024,
  })
  Object.defineProperty(limitCrossingFile, 'size', {
    value: 21 * 1024 * 1024,
  })
  const textarea = requireElement(
    container.querySelector<HTMLTextAreaElement>('textarea'),
  )
  const pasteEvent = new Event('paste', { bubbles: true, cancelable: true })
  Object.defineProperty(pasteEvent, 'clipboardData', {
    value: { files: [firstFile, limitCrossingFile] },
  })
  act(() => {
    void textarea.dispatchEvent(pasteEvent)
  })
  expect(
    await waitForState(
      () => container.querySelectorAll('.composer-attachment').length === 2,
    ),
  ).toBe(true)
  act(() => {
    void dispatchKey(textarea, 'Enter')
  })
  expect(
    await waitForState(
      () =>
        container.querySelector('[role="alert"]')?.textContent ===
        'limit-crossing.bin would exceed the 50 MiB attachment limit',
    ),
  ).toBe(true)

  const snapshot = () => ({
    error: container.querySelector('[role="alert"]')?.textContent ?? null,
    attachmentNames: Array.from(
      container.querySelectorAll('.composer-attachment span'),
      (element) => element.textContent,
    ),
    actionCalls: onAction.mock.calls,
  })
  expect(snapshot()).toEqual({
    error: 'limit-crossing.bin would exceed the 50 MiB attachment limit',
    attachmentNames: ['first.bin', 'limit-crossing.bin'],
    actionCalls: [],
  })
})

it('names and blocks the attachment that crosses the attachment count limit', async () => {
  const onAction = vi.fn(() => Promise.resolve(false))
  renderView(onAction)
  const files = Array.from(
    { length: 17 },
    (_, index) => new File(['x'], `file-${String(index + 1)}.txt`),
  )
  const textarea = requireElement(
    container.querySelector<HTMLTextAreaElement>('textarea'),
  )
  const pasteEvent = new Event('paste', { bubbles: true, cancelable: true })
  Object.defineProperty(pasteEvent, 'clipboardData', {
    value: { files },
  })
  act(() => {
    void textarea.dispatchEvent(pasteEvent)
  })
  expect(
    await waitForState(
      () =>
        container.querySelectorAll('.composer-attachment').length ===
        files.length,
    ),
  ).toBe(true)
  act(() => {
    void dispatchKey(textarea, 'Enter')
  })
  expect(
    await waitForState(
      () =>
        container.querySelector('[role="alert"]')?.textContent ===
        'file-17.txt exceeds the limit of 16 attachments per message',
    ),
  ).toBe(true)

  const snapshot = () => ({
    error: container.querySelector('[role="alert"]')?.textContent ?? null,
    attachmentNames: Array.from(
      container.querySelectorAll('.composer-attachment span'),
      (element) => element.textContent,
    ),
    actionCalls: onAction.mock.calls,
  })
  expect(snapshot()).toEqual({
    error: 'file-17.txt exceeds the limit of 16 attachments per message',
    attachmentNames: [
      'file-1.txt',
      'file-2.txt',
      'file-3.txt',
      'file-4.txt',
      'file-5.txt',
      'file-6.txt',
      'file-7.txt',
      'file-8.txt',
      'file-9.txt',
      'file-10.txt',
      'file-11.txt',
      'file-12.txt',
      'file-13.txt',
      'file-14.txt',
      'file-15.txt',
      'file-16.txt',
      'file-17.txt',
    ],
    actionCalls: [],
  })
})

it('shows a server attachment validation error and retains the rejected attachment', async () => {
  const jsonResponse = (body: unknown, status = 200) =>
    new Response(JSON.stringify(body), {
      status,
      headers: { 'Content-Type': 'application/json' },
    })
  const fetcher = vi
    .fn<typeof fetch>()
    .mockResolvedValueOnce(jsonResponse(messagesResponse([])))
    .mockResolvedValueOnce(
      jsonResponse({
        message: true,
        answerQuestion: false,
        interrupt: false,
        openInTerminal: true,
      }),
    )
    .mockResolvedValueOnce(
      jsonResponse({ error: 'file attachment data is invalid' }, 400),
    )
  vi.stubGlobal('fetch', fetcher)
  root = createRoot(container)
  act(() => {
    root?.render(
      <ConversationPane
        session={session}
        refreshKey={0}
        sessionTreeRefreshKey={0}
        onSelectSession={() => undefined}
      />,
    )
  })
  expect(
    await waitForState(
      () =>
        container.querySelector<HTMLTextAreaElement>('textarea')?.disabled ===
        false,
    ),
  ).toBe(true)

  const file = new File(['not an image'], 'document.pdf', {
    type: 'application/pdf',
  })
  const textarea = requireElement(
    container.querySelector<HTMLTextAreaElement>('textarea'),
  )
  const pasteEvent = new Event('paste', { bubbles: true, cancelable: true })
  Object.defineProperty(pasteEvent, 'clipboardData', {
    value: { files: [file] },
  })
  act(() => {
    void textarea.dispatchEvent(pasteEvent)
  })
  expect(
    await waitForState(
      () =>
        container.querySelector('.composer-attachment span')?.textContent ===
        file.name,
    ),
  ).toBe(true)
  act(() => {
    void dispatchKey(textarea, 'Enter')
  })
  expect(
    await waitForState(
      () =>
        container.querySelector('[role="alert"]')?.textContent ===
          'file attachment data is invalid' &&
        fetcher.mock.calls.length === 3 &&
        container.querySelector('.composer-attachment span')?.textContent ===
          file.name,
    ),
  ).toBe(true)

  const snapshot = () => ({
    error: container.querySelector('[role="alert"]')?.textContent ?? null,
    attachmentName: container.querySelector('.composer-attachment span')
      ?.textContent,
    postBody: fetcher.mock.calls[2]?.[1]?.body,
  })
  expect(snapshot()).toEqual({
    error: 'file attachment data is invalid',
    attachmentName: 'document.pdf',
    postBody: JSON.stringify({
      content: '',
      attachments: [
        {
          name: 'document.pdf',
          mimeType: 'application/pdf',
          size: 12,
          data: 'bm90IGFuIGltYWdl',
        },
      ],
    }),
  })
})

it('waits for pasted image optimization and sends the optimized attachment', async () => {
  const fetcher = vi.fn<typeof fetch>((input) =>
    Promise.resolve(
      requestUrl(input).endsWith('/actions')
        ? jsonResponse({
            message: true,
            answerQuestion: false,
            interrupt: false,
            openInTerminal: true,
          })
        : jsonResponse(messagesResponse([])),
    ),
  )
  vi.stubGlobal('fetch', fetcher)
  const optimizedFile = new File(['optimized'], 'photo.jpg', {
    type: 'image/jpeg',
  })
  let finishOptimization: ((file: File) => void) | undefined
  optimizeImageAttachmentMock.mockImplementationOnce(
    () =>
      new Promise((resolve) => {
        finishOptimization = resolve
      }),
  )
  root = createRoot(container)
  act(() => {
    root?.render(
      <ConversationPane
        session={session}
        refreshKey={0}
        sessionTreeRefreshKey={0}
        onSelectSession={() => undefined}
      />,
    )
  })
  expect(
    await waitForState(
      () =>
        container.querySelector<HTMLTextAreaElement>('textarea')?.disabled ===
        false,
    ),
  ).toBe(true)

  const textarea = requireElement(
    container.querySelector<HTMLTextAreaElement>('textarea'),
  )
  setTextareaValue(textarea, 'Inspect this photo.')
  const file = new File(['source'], 'photo.heic', { type: 'image/heic' })
  const pasteEvent = new Event('paste', { bubbles: true, cancelable: true })
  Object.defineProperty(pasteEvent, 'clipboardData', {
    value: { files: [file] },
  })
  act(() => {
    void textarea.dispatchEvent(pasteEvent)
  })
  expect(
    await waitForState(
      () => optimizeImageAttachmentMock.mock.calls.length === 1,
    ),
  ).toBe(true)

  const sendButton = requireElement(
    container.querySelector<HTMLButtonElement>('[aria-label="Send message"]'),
  )
  expect(sendButton.disabled).toBe(true)
  const form = requireElement(sendButton.closest('form'))
  act(() => {
    form.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }))
  })
  expect(fetcher.mock.calls[2]).toBeUndefined()

  const complete = finishOptimization
  if (complete === undefined) throw new Error('Expected optimization to start')
  await act(async () => {
    complete(optimizedFile)
    await Promise.resolve()
  })
  expect(
    await waitForState(
      () =>
        container.querySelector('.composer-attachment span')?.textContent ===
          'photo.jpg' && !sendButton.disabled,
    ),
  ).toBe(true)
  act(() => {
    sendButton.click()
  })
  expect(
    await waitForState(() => fetcher.mock.calls[2]?.[1]?.body !== undefined),
  ).toBe(true)

  const sentBody = () => fetcher.mock.calls[2]?.[1]?.body
  expect(sentBody()).toEqual(
    JSON.stringify({
      content: 'Inspect this photo.',
      attachments: [
        {
          name: 'photo.jpg',
          mimeType: 'image/jpeg',
          size: 9,
          data: 'b3B0aW1pemVk',
        },
      ],
    }),
  )
})

it('clears the message draft after a successful send', async () => {
  const onAction = vi.fn(() => Promise.resolve(true))
  renderView(onAction)
  const textarea = requireElement(
    container.querySelector<HTMLTextAreaElement>('textarea'),
  )
  setTextareaValue(textarea, 'Send this message')
  act(() => {
    dispatchKey(textarea, 'Enter')
  })
  await act(async () => {
    await Promise.resolve()
  })

  const snapshot = () => ({
    draft: textarea.value,
    storedDraft: window.localStorage.getItem(
      'agent-watch-web:composer-draft:sample-session',
    ),
    actions: onAction.mock.calls,
  })
  expect(snapshot()).toEqual({
    draft: '',
    storedDraft: null,
    actions: [['messages', { content: 'Send this message' }]],
  })
})

it('keeps text added after a send starts', async () => {
  let completeSend: ((succeeded: boolean) => void) | undefined
  const onAction = vi.fn(
    () =>
      new Promise<boolean>((resolve) => {
        completeSend = resolve
      }),
  )
  renderView(onAction)
  const textarea = requireElement(
    container.querySelector<HTMLTextAreaElement>('textarea'),
  )
  setTextareaValue(textarea, 'First message')
  act(() => {
    dispatchKey(textarea, 'Enter')
  })
  setTextareaValue(textarea, 'Next message')

  await act(async () => {
    completeSend?.(true)
    await Promise.resolve()
  })

  const snapshot = () => ({
    draft: textarea.value,
    storedDraft: window.localStorage.getItem(
      'agent-watch-web:composer-draft:sample-session',
    ),
    actions: onAction.mock.calls,
  })
  expect(snapshot()).toEqual({
    draft: 'Next message',
    storedDraft: 'Next message',
    actions: [['messages', { content: 'First message' }]],
  })
})

it('restores the message draft after the conversation is remounted', () => {
  renderView(() => Promise.resolve(true))
  const textarea = requireElement(
    container.querySelector<HTMLTextAreaElement>('textarea'),
  )
  setTextareaValue(textarea, 'Continue this draft')

  act(() => {
    root?.unmount()
  })
  root = null
  renderView(() => Promise.resolve(true))

  const snapshot = () => ({
    draft: container.querySelector<HTMLTextAreaElement>('textarea')?.value,
    storedDraft: window.localStorage.getItem(
      'agent-watch-web:composer-draft:sample-session',
    ),
  })
  expect(snapshot()).toEqual({
    draft: 'Continue this draft',
    storedDraft: 'Continue this draft',
  })
})

it('keeps the composer usable when browser storage is unavailable', () => {
  vi.spyOn(Storage.prototype, 'getItem').mockImplementation(() => {
    throw new Error('storage unavailable')
  })
  vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => {
    throw new Error('storage unavailable')
  })
  renderView(() => Promise.resolve(true))
  const textarea = requireElement(
    container.querySelector<HTMLTextAreaElement>('textarea'),
  )
  setTextareaValue(textarea, 'Keep editing in memory')

  const snapshot = () => ({
    draft: textarea.value,
    composerVisible: textarea.isConnected,
    storageNotice: container.querySelector('.conversation-error[role="status"]')
      ?.textContent,
  })
  expect(snapshot()).toEqual({
    draft: 'Keep editing in memory',
    composerVisible: true,
    storageNotice:
      'Browser storage is unavailable. Drafts may not survive a page reload.',
  })
})

it('reports when a draft cannot be saved after loading storage succeeds', () => {
  vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => {
    throw new Error('storage unavailable')
  })
  renderView(() => Promise.resolve(true))
  const textarea = requireElement(
    container.querySelector<HTMLTextAreaElement>('textarea'),
  )
  setTextareaValue(textarea, 'Keep editing in memory')

  const snapshot = () => ({
    draft: textarea.value,
    composerVisible: textarea.isConnected,
    storageNotice: container.querySelector('.conversation-error[role="status"]')
      ?.textContent,
  })
  expect(snapshot()).toEqual({
    draft: 'Keep editing in memory',
    composerVisible: true,
    storageNotice:
      'Browser storage is unavailable. Drafts may not survive a page reload.',
  })
})

it('keeps drafts separate for each session', () => {
  renderView(() => Promise.resolve(true))
  setTextareaValue(
    requireElement(container.querySelector<HTMLTextAreaElement>('textarea')),
    'First session draft',
  )

  act(() => {
    root?.unmount()
  })
  root = null
  renderView(() => Promise.resolve(true), { ...session, id: 'another-session' })
  const otherTextarea = requireElement(
    container.querySelector<HTMLTextAreaElement>('textarea'),
  )
  setTextareaValue(otherTextarea, 'Second session draft')

  act(() => {
    root?.unmount()
  })
  root = null
  renderView(() => Promise.resolve(true))

  const snapshot = () => ({
    selectedDraft:
      container.querySelector<HTMLTextAreaElement>('textarea')?.value,
    firstSessionDraft: window.localStorage.getItem(
      'agent-watch-web:composer-draft:sample-session',
    ),
    secondSessionDraft: window.localStorage.getItem(
      'agent-watch-web:composer-draft:another-session',
    ),
  })
  expect(snapshot()).toEqual({
    selectedDraft: 'First session draft',
    firstSessionDraft: 'First session draft',
    secondSessionDraft: 'Second session draft',
  })
})

it('blocks duplicate sends and shows their pending status', async () => {
  let completeSend: ((succeeded: boolean) => void) | undefined
  const onAction = vi.fn(
    () =>
      new Promise<boolean>((resolve) => {
        completeSend = resolve
      }),
  )
  renderView(onAction)
  const textarea = requireElement(
    container.querySelector<HTMLTextAreaElement>('textarea'),
  )
  setTextareaValue(textarea, 'Send once')
  const events: KeyboardEvent[] = []
  act(() => {
    events.push(dispatchKey(textarea, 'Enter'))
    events.push(dispatchKey(textarea, 'Enter'))
  })
  const sendButton = requireElement(
    container.querySelector<HTMLButtonElement>('[aria-label="Send message"]'),
  )
  const duringSend = () => ({
    actionCalls: onAction.mock.calls,
    buttonDisabled: sendButton.disabled,
    defaultPrevented: events.map((event) => event.defaultPrevented),
    draft: textarea.value,
    status:
      container.querySelector('.composer-footer [role="status"]')
        ?.textContent ?? null,
  })
  const pendingSnapshot = duringSend()

  await act(async () => {
    completeSend?.(true)
    await Promise.resolve()
  })

  const snapshot = () => ({
    duringSend: pendingSnapshot,
    afterSend: {
      buttonDisabled: sendButton.disabled,
      draft: textarea.value,
      status:
        container.querySelector('.composer-footer [role="status"]')
          ?.textContent ?? null,
    },
  })
  const expected = {
    duringSend: {
      actionCalls: [['messages', { content: 'Send once' }]],
      buttonDisabled: true,
      defaultPrevented: [true, true],
      draft: 'Send once',
      status: 'Sending…',
    },
    afterSend: {
      buttonDisabled: true,
      draft: '',
      status: null,
    },
  }
  expect(snapshot()).toEqual(expected)
})

it('follows new messages at the bottom without moving a reader who scrolled up', () => {
  const onAction = vi.fn(() => Promise.resolve(true))
  const firstMessage: ChatItem = {
    type: 'assistant',
    text: 'First message',
  }
  renderView(onAction, session, undefined, [firstMessage])
  const scroll = requireElement(
    container.querySelector<HTMLDivElement>('.conversation-scroll'),
  )
  let height = 300
  Object.defineProperties(scroll, {
    scrollHeight: { configurable: true, get: () => height },
    clientHeight: { configurable: true, value: 100 },
    scrollTop: { configurable: true, writable: true, value: 0 },
  })
  renderView(onAction, session, undefined, [firstMessage])
  const initialBottom = scroll.scrollTop

  height = 500
  renderView(onAction, session, undefined, [
    firstMessage,
    { type: 'assistant', text: 'Second message' },
  ])
  const afterNewMessage = scroll.scrollTop

  act(() => {
    scroll.scrollTop = 40
    scroll.dispatchEvent(new Event('scroll'))
  })
  height = 600
  renderView(onAction, session, undefined, [
    firstMessage,
    { type: 'assistant', text: 'Second message' },
    { type: 'assistant', text: 'Third message' },
  ])
  const whileReadingEarlierMessages = scroll.scrollTop

  act(() => {
    scroll.scrollTop = 500
    scroll.dispatchEvent(new Event('scroll'))
  })
  height = 700
  renderView(onAction, session, undefined, [
    firstMessage,
    { type: 'assistant', text: 'Second message' },
    { type: 'assistant', text: 'Third message' },
    { type: 'assistant', text: 'Fourth message' },
  ])

  const snapshot = () => ({
    initialBottom,
    afterNewMessage,
    whileReadingEarlierMessages,
    afterReturningToBottom: scroll.scrollTop,
  })
  expect(snapshot()).toEqual({
    initialBottom: 200,
    afterNewMessage: 400,
    whileReadingEarlierMessages: 40,
    afterReturningToBottom: 600,
  })
})

it.each([
  { action: 'approve' as const, buttonIndex: 0 },
  { action: 'reject' as const, buttonIndex: 1 },
])('shows the $action error independently', async ({ action, buttonIndex }) => {
  const jsonResponse = (body: unknown, status = 200) =>
    new Response(JSON.stringify(body), {
      status,
      headers: { 'Content-Type': 'application/json' },
    })
  const fetcher = vi
    .fn<typeof fetch>()
    .mockResolvedValueOnce(
      jsonResponse(
        messagesResponse([
          {
            type: 'permission',
            requestId: 'permission-request-api-test',
            tool: 'Shell',
            description: 'Run the formatter on changed files.',
          },
        ]),
      ),
    )
    .mockResolvedValueOnce(
      jsonResponse({
        message: true,
        answerQuestion: false,
        permission: { approve: true, reject: true },
        interrupt: true,
        openInTerminal: true,
      }),
    )
    .mockResolvedValueOnce(
      jsonResponse(
        {
          error:
            'permission request is no longer pending; answer it in the terminal',
        },
        409,
      ),
    )
  vi.stubGlobal('fetch', fetcher)
  root = createRoot(container)
  act(() => {
    root?.render(
      <ConversationPane
        session={session}
        refreshKey={0}
        sessionTreeRefreshKey={0}
        onSelectSession={() => undefined}
      />,
    )
  })
  await act(async () => {
    for (let index = 0; index < 6; index += 1) await Promise.resolve()
  })
  const permissionButtons = () =>
    container.querySelectorAll<HTMLButtonElement>('.permission-actions button')
  act(() => {
    requireElement(permissionButtons().item(buttonIndex)).click()
  })
  await act(async () => {
    for (let index = 0; index < 12; index += 1) await Promise.resolve()
  })

  const actionRequest = fetcher.mock.calls[2]?.[0]
  const actionRequestUrl =
    typeof actionRequest === 'string'
      ? actionRequest
      : actionRequest instanceof URL
        ? actionRequest.href
        : actionRequest?.url
  const actionRequestBody = fetcher.mock.calls[2]?.[1]?.body
  const expected = JSON.stringify([
    'permission request is no longer pending; answer it in the terminal',
    `/api/sessions/sample-session/permission/${action}`,
    JSON.stringify({ requestId: 'permission-request-api-test' }),
    [
      { label: 'Approve', disabled: false },
      { label: 'Reject', disabled: false },
    ],
  ])
  const buttonStates = Array.from(permissionButtons(), (button) => ({
    label: button.textContent,
    disabled: button.disabled,
  }))
  const actual = JSON.stringify([
    container.querySelector('[role="alert"]')?.textContent,
    actionRequestUrl,
    actionRequestBody,
    buttonStates,
  ])
  expect(actual).toBe(expected)
})

it.each([
  {
    action: 'approve' as const,
    pendingLabel: 'Approving…',
    sentMessage: 'Approval sent. Waiting for Claude Code…',
  },
  {
    action: 'reject' as const,
    pendingLabel: 'Rejecting…',
    sentMessage: 'Rejection sent. Waiting for Claude Code…',
  },
])(
  'keeps a successful permission $action pending until it resolves',
  async ({ action, pendingLabel, sentMessage }) => {
    const jsonResponse = (body: unknown) =>
      new Response(JSON.stringify(body), {
        headers: { 'Content-Type': 'application/json' },
      })
    let completeAction: ((response: Response) => void) | undefined
    const completePendingAction = (response: Response) => {
      const resolve = completeAction
      if (resolve === undefined) {
        throw new Error('Expected a pending action request')
      }
      resolve(response)
    }
    const unavailableActions: SessionActions = {
      ...actions,
      permission: { approve: false, reject: false },
    }
    const fetcher = vi
      .fn<typeof fetch>()
      .mockResolvedValueOnce(jsonResponse(messagesResponse([permission])))
      .mockResolvedValueOnce(jsonResponse(actions))
      .mockImplementationOnce(
        () =>
          new Promise<Response>((resolve) => {
            completeAction = resolve
          }),
      )
      .mockResolvedValueOnce(jsonResponse(messagesResponse([permission])))
      .mockResolvedValueOnce(jsonResponse(actions))
      .mockResolvedValueOnce(jsonResponse(messagesResponse([permission])))
      .mockResolvedValueOnce(jsonResponse(unavailableActions))
      .mockResolvedValueOnce(jsonResponse(messagesResponse([])))
      .mockResolvedValueOnce(jsonResponse(unavailableActions))
    vi.stubGlobal('fetch', fetcher)
    root = createRoot(container)
    act(() => {
      root?.render(
        <ConversationPane
          session={session}
          refreshKey={0}
          sessionTreeRefreshKey={0}
          onSelectSession={() => undefined}
        />,
      )
    })
    await act(async () => {
      for (let index = 0; index < 8; index += 1) await Promise.resolve()
    })

    act(() => {
      requireElement(
        container.querySelectorAll<HTMLButtonElement>(
          '.permission-actions button',
        )[action === 'approve' ? 0 : 1] ?? null,
      ).click()
    })
    const duringRequest = {
      labels: Array.from(
        container.querySelectorAll<HTMLButtonElement>(
          '.permission-actions button',
        ),
      ).map((button) => button.textContent),
      disabled: Array.from(
        container.querySelectorAll<HTMLButtonElement>(
          '.permission-actions button',
        ),
      ).map((button) => button.disabled),
    }

    completePendingAction(new Response(null, { status: 204 }))
    await act(async () => {
      for (let index = 0; index < 12; index += 1) await Promise.resolve()
    })
    const afterSuccess = {
      labels: Array.from(
        container.querySelectorAll<HTMLButtonElement>(
          '.permission-actions button',
        ),
      ).map((button) => button.textContent),
      disabled: Array.from(
        container.querySelectorAll<HTMLButtonElement>(
          '.permission-actions button',
        ),
      ).map((button) => button.disabled),
      status:
        container.querySelector('.permission-action-status')?.textContent ??
        null,
    }
    const actionRequest = fetcher.mock.calls[2]?.[0]
    const actionRequestUrl =
      typeof actionRequest === 'string'
        ? actionRequest
        : actionRequest instanceof URL
          ? actionRequest.href
          : actionRequest?.url

    act(() => {
      root?.render(
        <ConversationPane
          session={session}
          refreshKey={1}
          sessionTreeRefreshKey={0}
          onSelectSession={() => undefined}
        />,
      )
    })
    await act(async () => {
      for (let index = 0; index < 12; index += 1) await Promise.resolve()
    })
    const afterCapabilityDisappears = {
      permissionCard: container.querySelector('.chat-permission-card') !== null,
      permissionButtons: container.querySelectorAll(
        '.permission-actions button',
      ).length,
      status:
        container.querySelector('.permission-action-status')?.textContent ??
        null,
    }

    act(() => {
      root?.render(
        <ConversationPane
          session={session}
          refreshKey={2}
          sessionTreeRefreshKey={0}
          onSelectSession={() => undefined}
        />,
      )
    })
    await act(async () => {
      for (let index = 0; index < 12; index += 1) await Promise.resolve()
    })

    const snapshot = () => ({
      duringRequest,
      afterSuccess,
      actionRequestUrl,
      afterCapabilityDisappears,
      afterCardDisappears: {
        permissionCard: container.querySelector('.chat-permission-card'),
        status:
          container.querySelector('.permission-action-status')?.textContent ??
          null,
      },
    })
    expect(snapshot()).toEqual({
      duringRequest: {
        labels:
          action === 'approve'
            ? [pendingLabel, 'Reject']
            : ['Approve', pendingLabel],
        disabled: [true, true],
      },
      afterSuccess: {
        labels: ['Approve', 'Reject'],
        disabled: [true, true],
        status: sentMessage,
      },
      actionRequestUrl: `/api/sessions/sample-session/permission/${action}`,
      afterCapabilityDisappears: {
        permissionCard: true,
        permissionButtons: 0,
        status: null,
      },
      afterCardDisappears: { permissionCard: null, status: null },
    })
  },
)

it('shows an HTTP status when the initial messages response has no body', async () => {
  const actionsResponse = new Response(
    JSON.stringify({
      message: false,
      answerQuestion: false,
      interrupt: false,
      openInTerminal: true,
    }),
    { headers: { 'Content-Type': 'application/json' } },
  )
  const fetcher = vi
    .fn<typeof fetch>()
    .mockResolvedValueOnce(
      new Response('', { status: 404, statusText: 'Not Found' }),
    )
    .mockResolvedValueOnce(actionsResponse)
  vi.stubGlobal('fetch', fetcher)
  root = createRoot(container)
  act(() => {
    root?.render(
      <ConversationPane
        session={session}
        refreshKey={0}
        sessionTreeRefreshKey={0}
        onSelectSession={() => undefined}
      />,
    )
  })
  await act(async () => {
    await Promise.resolve()
    await Promise.resolve()
    await Promise.resolve()
    await Promise.resolve()
    await Promise.resolve()
    await Promise.resolve()
  })

  const actual = JSON.stringify([
    container.querySelector('[role="alert"]')?.textContent,
    container.querySelector('.conversation-status')?.textContent,
  ])
  const expected = JSON.stringify(['Not Found', 'No messages yet.'])
  expect(actual).toBe(expected)
})

it('reloads the selected conversation when the session feed changes', async () => {
  const jsonResponse = (body: unknown) =>
    new Response(JSON.stringify(body), {
      headers: { 'Content-Type': 'application/json' },
    })
  const fetcher = vi
    .fn<typeof fetch>()
    .mockResolvedValueOnce(
      jsonResponse(
        messagesResponse(
          [{ type: 'assistant', text: 'Before update' }],
          [
            {
              id: 'fixture-task-one',
              description: 'Watch the build pipeline',
              elapsedSeconds: 18,
              kind: 'background_task',
            },
          ],
        ),
      ),
    )
    .mockResolvedValueOnce(
      jsonResponse({
        message: true,
        answerQuestion: false,
        interrupt: true,
        openInTerminal: true,
      }),
    )
    .mockResolvedValueOnce(
      jsonResponse(
        messagesResponse([{ type: 'assistant', text: 'After update' }]),
      ),
    )
    .mockResolvedValueOnce(
      jsonResponse({
        message: true,
        answerQuestion: false,
        interrupt: true,
        openInTerminal: true,
      }),
    )
  vi.stubGlobal('fetch', fetcher)
  root = createRoot(container)
  act(() => {
    root?.render(
      <ConversationPane
        session={session}
        refreshKey={0}
        sessionTreeRefreshKey={0}
        onSelectSession={() => undefined}
      />,
    )
  })
  await act(async () => {
    await Promise.resolve()
    await Promise.resolve()
    await Promise.resolve()
    await Promise.resolve()
    await Promise.resolve()
    await Promise.resolve()
  })
  act(() => {
    root?.render(
      <ConversationPane
        session={session}
        refreshKey={1}
        sessionTreeRefreshKey={0}
        onSelectSession={() => undefined}
      />,
    )
  })
  await act(async () => {
    await Promise.resolve()
    await Promise.resolve()
    await Promise.resolve()
    await Promise.resolve()
    await Promise.resolve()
    await Promise.resolve()
  })

  const snapshot = () => ({
    transcript: container.querySelector('.chat-assistant-message')?.textContent,
    backgroundTasks: container.querySelector(
      '.conversation-background-tasks summary',
    )?.textContent,
    requestCount: fetcher.mock.calls.length,
  })
  expect(snapshot()).toEqual({
    transcript: 'After update',
    backgroundTasks: undefined,
    requestCount: 4,
  })
})

it('keeps conversation DOM connected while toggling the status panel', async () => {
  const jsonResponse = (body: unknown) =>
    new Response(JSON.stringify(body), {
      headers: { 'Content-Type': 'application/json' },
    })
  const stableMessages = messagesResponse([
    { type: 'assistant', text: 'Stable conversation message' },
  ])
  const stableActions = {
    message: true,
    answerQuestion: false,
    interrupt: true,
    openInTerminal: true,
  }
  const fetcher = vi
    .fn<typeof fetch>()
    .mockResolvedValueOnce(jsonResponse(stableMessages))
    .mockResolvedValueOnce(jsonResponse(stableActions))
  vi.stubGlobal('fetch', fetcher)
  root = createRoot(container)
  const renderConversation = (statusPanelOpen: boolean, refreshKey: number) => {
    root?.render(
      <ConversationPane
        session={session}
        refreshKey={refreshKey}
        sessionTreeRefreshKey={0}
        onSelectSession={() => undefined}
        statusPanelOpen={statusPanelOpen}
      />,
    )
  }
  act(() => {
    renderConversation(false, 0)
  })
  await act(async () => {
    for (let index = 0; index < 12; index += 1) await Promise.resolve()
  })
  const retainedMessage = requireElement(
    container.querySelector('.chat-assistant-message'),
  )

  act(() => {
    renderConversation(true, 0)
  })
  await act(async () => {
    for (let index = 0; index < 12; index += 1) await Promise.resolve()
  })
  const sameMessageAfterOpening =
    retainedMessage.isConnected &&
    container.querySelector('.chat-assistant-message') === retainedMessage

  act(() => {
    renderConversation(false, 0)
  })
  await act(async () => {
    for (let index = 0; index < 12; index += 1) await Promise.resolve()
  })
  const sameMessageAfterClosing =
    retainedMessage.isConnected &&
    container.querySelector('.chat-assistant-message') === retainedMessage

  const actual = JSON.stringify([
    sameMessageAfterOpening,
    sameMessageAfterClosing,
    fetcher.mock.calls.length,
  ])
  const expected = JSON.stringify([true, true, 2])
  expect(actual).toBe(expected)
})

it('keeps the conversation and focused draft while a feed refresh is pending', async () => {
  const jsonResponse = (body: unknown) =>
    new Response(JSON.stringify(body), {
      headers: { 'Content-Type': 'application/json' },
    })
  const fetcher = vi
    .fn<typeof fetch>()
    .mockResolvedValueOnce(
      jsonResponse(
        messagesResponse([
          { type: 'assistant', text: 'Current conversation' },
          question,
          {
            type: 'permission',
            requestId: 'permission-request-refresh',
            tool: 'Shell',
            description: 'Run the project formatter.',
          },
        ]),
      ),
    )
    .mockResolvedValueOnce(
      jsonResponse({
        message: true,
        answerQuestion: true,
        permission: { approve: true, reject: true },
        interrupt: true,
        openInTerminal: true,
      }),
    )
    .mockImplementation(() => new Promise<Response>(() => undefined))
  vi.stubGlobal('fetch', fetcher)
  root = createRoot(container)
  act(() => {
    root?.render(
      <ConversationPane
        session={session}
        refreshKey={0}
        sessionTreeRefreshKey={0}
        onSelectSession={() => undefined}
      />,
    )
  })
  await act(async () => {
    await Promise.resolve()
    await Promise.resolve()
    await Promise.resolve()
    await Promise.resolve()
    await Promise.resolve()
    await Promise.resolve()
  })
  const textarea = requireElement(
    container.querySelector<HTMLTextAreaElement>('textarea'),
  )
  setTextareaValue(textarea, 'Draft during refresh')
  act(() => {
    textarea.focus()
    root?.render(
      <ConversationPane
        session={session}
        refreshKey={1}
        sessionTreeRefreshKey={0}
        onSelectSession={() => undefined}
      />,
    )
  })

  const snapshot = () => ({
    sameComposer: textarea === container.querySelector('textarea'),
    composerValue: textarea.value,
    composerDisabled: textarea.disabled,
    composerFocused: document.activeElement === textarea,
    transcript: container.querySelector('.chat-assistant-message')?.textContent,
    questionChoiceDisabled: Array.from(
      container.querySelectorAll<HTMLButtonElement>(
        '.chat-question-choices button',
      ),
      (button) => button.disabled,
    ),
    permissionButtonsDisabled: Array.from(
      container.querySelectorAll<HTMLButtonElement>(
        '.permission-actions button',
      ),
      (button) => button.disabled,
    ),
    conversationStatus: container.querySelector('.conversation-status')
      ?.textContent,
    requestsStarted: fetcher.mock.calls.length,
  })
  expect(snapshot()).toEqual({
    sameComposer: true,
    composerValue: 'Draft during refresh',
    composerDisabled: false,
    composerFocused: true,
    transcript: 'Current conversation',
    questionChoiceDisabled: [false, false],
    permissionButtonsDisabled: [false, false],
    conversationStatus: undefined,
    requestsStarted: 4,
  })
})

it('shows terminal switch progress and success without reloading messages', async () => {
  let resolveFocus: ((response: Response) => void) | undefined
  const focusResponse = new Promise<Response>((resolve) => {
    resolveFocus = resolve
  })
  const fetcher = vi
    .fn<typeof fetch>()
    .mockResolvedValueOnce(
      jsonResponse(messagesResponse([question, permission])),
    )
    .mockResolvedValueOnce(jsonResponse(codexTerminalActions))
    .mockReturnValueOnce(focusResponse)
  vi.stubGlobal('fetch', fetcher)
  root = createRoot(container)
  act(() => {
    root?.render(
      <ConversationPane
        session={{ ...session, engine: 'codex' }}
        refreshKey={0}
        sessionTreeRefreshKey={0}
        onSelectSession={() => undefined}
      />,
    )
  })
  await act(async () => {
    for (let index = 0; index < 12; index += 1) await Promise.resolve()
  })
  const initialLiveRegions = Array.from(
    container.querySelectorAll('.conversation-terminal-action-status'),
  )

  const terminalButtons = () =>
    Array.from(
      container.querySelectorAll<HTMLButtonElement>(
        '.conversation-open-in-terminal button',
      ),
    )
  act(() => {
    terminalButtons()[0]?.click()
  })
  const pendingSnapshot = () => ({
    feedback: terminalFeedback(),
    disabled: terminalButtons().map((button) => button.disabled),
  })
  const pending = pendingSnapshot()

  await act(async () => {
    resolveFocus?.(new Response(null, { status: 204 }))
    await focusResponse
  })
  const successSnapshot = () => ({
    feedback: terminalFeedback(),
    requests: fetcher.mock.calls.map(([url]) => url),
    liveRegions: initialLiveRegions.map((element, index) => ({
      sameElement:
        element ===
        container.querySelectorAll('.conversation-terminal-action-status')[
          index
        ],
      role: element.getAttribute('role'),
      ariaLive: element.getAttribute('aria-live'),
      ariaAtomic: element.getAttribute('aria-atomic'),
      ariaHidden: element.getAttribute('aria-hidden'),
    })),
    announcementCount: container.querySelectorAll(
      '.conversation-terminal-action-status[role="status"][aria-live="polite"]',
    ).length,
  })
  const success = successSnapshot()
  const snapshot = () => ({ pending, success })
  const expected = {
    pending: {
      feedback: ['Switching tmux…', 'Switching tmux…', 'Switching tmux…'],
      disabled: [true, true, true],
    },
    success: {
      feedback: [
        'Tmux switched to this session.',
        'Tmux switched to this session.',
        'Tmux switched to this session.',
      ],
      requests: [
        '/api/sessions/sample-session/messages?limit=100',
        '/api/sessions/sample-session/actions',
        '/api/sessions/sample-session/focus',
      ],
      liveRegions: [
        {
          sameElement: true,
          role: 'status',
          ariaLive: 'polite',
          ariaAtomic: 'true',
          ariaHidden: null,
        },
        {
          sameElement: true,
          role: null,
          ariaLive: null,
          ariaAtomic: null,
          ariaHidden: 'true',
        },
        {
          sameElement: true,
          role: null,
          ariaLive: null,
          ariaAtomic: null,
          ariaHidden: 'true',
        },
      ],
      announcementCount: 1,
    },
  }
  expect(snapshot()).toEqual(expected)
})

it('shows the server reason beside every terminal action when switching fails', async () => {
  const fetcher = vi
    .fn<typeof fetch>()
    .mockResolvedValueOnce(
      jsonResponse(messagesResponse([question, permission])),
    )
    .mockResolvedValueOnce(jsonResponse(codexTerminalActions))
    .mockResolvedValueOnce(
      jsonResponse({ error: 'The terminal session is unavailable.' }, 503),
    )
  vi.stubGlobal('fetch', fetcher)
  root = createRoot(container)
  act(() => {
    root?.render(
      <ConversationPane
        session={{ ...session, engine: 'codex' }}
        refreshKey={0}
        sessionTreeRefreshKey={0}
        onSelectSession={() => undefined}
      />,
    )
  })
  await act(async () => {
    for (let index = 0; index < 12; index += 1) await Promise.resolve()
  })
  act(() => {
    container
      .querySelector<HTMLButtonElement>('.conversation-open-in-terminal button')
      ?.click()
  })
  await act(async () => {
    for (let index = 0; index < 12; index += 1) await Promise.resolve()
  })

  const snapshot = () => ({
    feedback: terminalFeedback(),
    requests: fetcher.mock.calls.map(([url]) => url),
  })
  expect(snapshot()).toEqual({
    feedback: [
      'Could not switch tmux: The terminal session is unavailable.',
      'Could not switch tmux: The terminal session is unavailable.',
      'Could not switch tmux: The terminal session is unavailable.',
    ],
    requests: [
      '/api/sessions/sample-session/messages?limit=100',
      '/api/sessions/sample-session/actions',
      '/api/sessions/sample-session/focus',
    ],
  })
})

it('ignores a terminal result after the conversation switches sessions', async () => {
  let resolveFocus: ((response: Response) => void) | undefined
  const focusResponse = new Promise<Response>((resolve) => {
    resolveFocus = resolve
  })
  const fetcher = vi.fn<typeof fetch>().mockImplementation((input) => {
    const url =
      typeof input === 'string'
        ? input
        : input instanceof URL
          ? input.href
          : input.url
    if (url.split('?')[0]?.endsWith('/messages') === true) {
      return Promise.resolve(
        jsonResponse(messagesResponse([question, permission])),
      )
    }
    if (url.endsWith('/actions')) {
      return Promise.resolve(jsonResponse(codexTerminalActions))
    }
    return focusResponse
  })
  vi.stubGlobal('fetch', fetcher)
  root = createRoot(container)
  act(() => {
    root?.render(
      <ConversationPane
        session={{ ...session, engine: 'codex' }}
        refreshKey={0}
        sessionTreeRefreshKey={0}
        onSelectSession={() => undefined}
      />,
    )
  })
  await act(async () => {
    for (let index = 0; index < 12; index += 1) await Promise.resolve()
  })

  act(() => {
    container
      .querySelector<HTMLButtonElement>('.conversation-open-in-terminal button')
      ?.click()
  })
  act(() => {
    root?.render(
      <ConversationPane
        session={{
          ...session,
          id: 'next-session',
          title: 'A later session',
          engine: 'codex',
        }}
        refreshKey={0}
        sessionTreeRefreshKey={0}
        onSelectSession={() => undefined}
      />,
    )
  })
  await act(async () => {
    for (let index = 0; index < 12; index += 1) await Promise.resolve()
  })
  await act(async () => {
    resolveFocus?.(new Response(null, { status: 204 }))
    await focusResponse
    for (let index = 0; index < 12; index += 1) await Promise.resolve()
  })

  const snapshot = () => ({
    feedback: terminalFeedback(),
    requests: fetcher.mock.calls.map(([url]) => url),
  })
  expect(snapshot()).toEqual({
    feedback: [],
    requests: [
      '/api/sessions/sample-session/messages?limit=100',
      '/api/sessions/sample-session/actions',
      '/api/sessions/sample-session/focus',
      '/api/sessions/next-session/messages?limit=100',
      '/api/sessions/next-session/actions',
    ],
  })
})

it('hides successful terminal feedback after three seconds', async () => {
  vi.useFakeTimers()
  const fetcher = vi
    .fn<typeof fetch>()
    .mockResolvedValueOnce(
      jsonResponse(messagesResponse([question, permission])),
    )
    .mockResolvedValueOnce(jsonResponse(codexTerminalActions))
    .mockResolvedValueOnce(new Response(null, { status: 204 }))
  vi.stubGlobal('fetch', fetcher)
  root = createRoot(container)
  act(() => {
    root?.render(
      <ConversationPane
        session={{ ...session, engine: 'codex' }}
        refreshKey={0}
        sessionTreeRefreshKey={0}
        onSelectSession={() => undefined}
      />,
    )
  })
  await act(async () => {
    for (let index = 0; index < 12; index += 1) await Promise.resolve()
  })
  act(() => {
    container
      .querySelector<HTMLButtonElement>('.conversation-open-in-terminal button')
      ?.click()
  })
  await act(async () => {
    for (let index = 0; index < 12; index += 1) await Promise.resolve()
  })
  const visibleFeedback = terminalFeedback()
  await act(async () => {
    await vi.advanceTimersByTimeAsync(3_000)
  })

  const snapshot = () => ({
    visibleFeedback,
    feedbackAfterTimeout: terminalFeedback(),
    requests: fetcher.mock.calls.map(([url]) => url),
  })
  expect(snapshot()).toEqual({
    visibleFeedback: [
      'Tmux switched to this session.',
      'Tmux switched to this session.',
      'Tmux switched to this session.',
    ],
    feedbackAfterTimeout: [],
    requests: [
      '/api/sessions/sample-session/messages?limit=100',
      '/api/sessions/sample-session/actions',
      '/api/sessions/sample-session/focus',
    ],
  })
})

it('clears terminal feedback timers when the session changes or unmounts', async () => {
  vi.useFakeTimers()
  const fetcher = vi.fn<typeof fetch>().mockImplementation((input) => {
    const url =
      typeof input === 'string'
        ? input
        : input instanceof URL
          ? input.href
          : input.url
    if (url.split('?')[0]?.endsWith('/messages') === true) {
      return Promise.resolve(
        jsonResponse(messagesResponse([question, permission])),
      )
    }
    if (url.endsWith('/actions')) {
      return Promise.resolve(jsonResponse(codexTerminalActions))
    }
    return Promise.resolve(new Response(null, { status: 204 }))
  })
  vi.stubGlobal('fetch', fetcher)
  root = createRoot(container)
  act(() => {
    root?.render(
      <ConversationPane
        session={{ ...session, engine: 'codex' }}
        refreshKey={0}
        sessionTreeRefreshKey={0}
        onSelectSession={() => undefined}
      />,
    )
  })
  await act(async () => {
    for (let index = 0; index < 12; index += 1) await Promise.resolve()
  })
  const baselineTimers = vi.getTimerCount()
  act(() => {
    container
      .querySelector<HTMLButtonElement>('.conversation-open-in-terminal button')
      ?.click()
  })
  await act(async () => {
    for (let index = 0; index < 12; index += 1) await Promise.resolve()
  })
  const timersAfterFirstSuccess = vi.getTimerCount() - baselineTimers

  act(() => {
    root?.render(
      <ConversationPane
        session={{
          ...session,
          id: 'next-session',
          title: 'A later session',
          engine: 'codex',
        }}
        refreshKey={0}
        sessionTreeRefreshKey={0}
        onSelectSession={() => undefined}
      />,
    )
  })
  await act(async () => {
    for (let index = 0; index < 12; index += 1) await Promise.resolve()
  })
  const timersAfterSessionSwitch = vi.getTimerCount() - baselineTimers
  act(() => {
    container
      .querySelector<HTMLButtonElement>('.conversation-open-in-terminal button')
      ?.click()
  })
  await act(async () => {
    for (let index = 0; index < 12; index += 1) await Promise.resolve()
  })
  const timersBeforeUnmount = vi.getTimerCount() - baselineTimers
  act(() => {
    root?.unmount()
  })
  root = null

  const snapshot = () => ({
    timersAfterFirstSuccess,
    timersAfterSessionSwitch,
    timersBeforeUnmount,
    timersAfterUnmount: vi.getTimerCount() - baselineTimers,
  })
  expect(snapshot()).toEqual({
    timersAfterFirstSuccess: 1,
    timersAfterSessionSwitch: 0,
    timersBeforeUnmount: 1,
    timersAfterUnmount: 0,
  })
})

it('keeps question and permission actions active after a refresh error', async () => {
  const jsonResponse = (body: unknown) =>
    new Response(JSON.stringify(body), {
      headers: { 'Content-Type': 'application/json' },
    })
  const fetcher = vi
    .fn<typeof fetch>()
    .mockResolvedValueOnce(
      jsonResponse(
        messagesResponse([
          question,
          {
            type: 'permission',
            requestId: 'permission-request-refresh-error',
            tool: 'Shell',
            description: 'Run the project formatter.',
          },
        ]),
      ),
    )
    .mockResolvedValueOnce(
      jsonResponse({
        message: true,
        answerQuestion: true,
        permission: { approve: true, reject: true },
        interrupt: true,
        openInTerminal: true,
      }),
    )
    .mockResolvedValueOnce(
      new Response(JSON.stringify({ error: 'Messages unavailable' }), {
        status: 503,
        headers: { 'Content-Type': 'application/json' },
      }),
    )
    .mockResolvedValueOnce(
      jsonResponse({
        message: true,
        answerQuestion: true,
        permission: { approve: true, reject: true },
        interrupt: true,
        openInTerminal: true,
      }),
    )
  vi.stubGlobal('fetch', fetcher)
  root = createRoot(container)
  act(() => {
    root?.render(
      <ConversationPane
        session={session}
        refreshKey={0}
        sessionTreeRefreshKey={0}
        onSelectSession={() => undefined}
      />,
    )
  })
  await act(async () => {
    for (let index = 0; index < 12; index += 1) await Promise.resolve()
  })
  act(() => {
    root?.render(
      <ConversationPane
        session={session}
        refreshKey={1}
        sessionTreeRefreshKey={0}
        onSelectSession={() => undefined}
      />,
    )
  })
  await act(async () => {
    for (let index = 0; index < 12; index += 1) await Promise.resolve()
  })

  const snapshot = () => ({
    questionChoiceDisabled: Array.from(
      container.querySelectorAll<HTMLButtonElement>(
        '.chat-question-choices button',
      ),
      (button) => button.disabled,
    ),
    permissionButtonsDisabled: Array.from(
      container.querySelectorAll<HTMLButtonElement>(
        '.permission-actions button',
      ),
      (button) => button.disabled,
    ),
    error: container.querySelector('[role="alert"]')?.textContent,
    requestCount: fetcher.mock.calls.length,
  })
  expect(snapshot()).toEqual({
    questionChoiceDisabled: [false, false],
    permissionButtonsDisabled: [false, false],
    error: 'Messages unavailable',
    requestCount: 4,
  })
})

it('keeps unchanged message DOM nodes across a conversation refresh', async () => {
  const jsonResponse = (body: unknown) =>
    new Response(JSON.stringify(body), {
      headers: { 'Content-Type': 'application/json' },
    })
  const messages: ChatItem[] = [
    { type: 'assistant', text: 'The conversation is unchanged.' },
  ]
  const sessions: Session[] = []
  const onSelectSession = () => undefined
  const fetcher = vi
    .fn<typeof fetch>()
    .mockResolvedValueOnce(jsonResponse(messagesResponse(messages)))
    .mockResolvedValueOnce(jsonResponse(actions))
    .mockResolvedValueOnce(jsonResponse(messagesResponse(messages)))
    .mockResolvedValueOnce(jsonResponse(actions))
  vi.stubGlobal('fetch', fetcher)
  root = createRoot(container)
  act(() => {
    root?.render(
      <ConversationPane
        session={session}
        sessions={sessions}
        refreshKey={0}
        sessionTreeRefreshKey={0}
        onSelectSession={onSelectSession}
      />,
    )
  })
  await waitForState(
    () => container.querySelector('.chat-assistant-message') !== null,
  )
  const assistantMessage = requireElement(
    container.querySelector('.chat-assistant-message'),
  )

  act(() => {
    root?.render(
      <ConversationPane
        session={session}
        sessions={sessions}
        refreshKey={1}
        sessionTreeRefreshKey={0}
        onSelectSession={onSelectSession}
      />,
    )
  })
  await act(async () => {
    for (let index = 0; index < 12; index += 1) await Promise.resolve()
  })

  const snapshot = () => ({
    sameMessageNode:
      assistantMessage === container.querySelector('.chat-assistant-message'),
    messageText: container.querySelector('.chat-assistant-message')
      ?.textContent,
    requestCount: fetcher.mock.calls.length,
  })
  expect(snapshot()).toEqual({
    sameMessageNode: true,
    messageText: 'The conversation is unchanged.',
    requestCount: 4,
  })
})

it('loads only the latest page initially and refreshes from the loaded start', async () => {
  const jsonResponse = (body: unknown) =>
    new Response(JSON.stringify(body), {
      headers: { 'Content-Type': 'application/json' },
    })
  const firstInitialMessage: ChatItem = {
    type: 'assistant',
    text: 'Loaded message one.',
  }
  const initialMessages = [
    firstInitialMessage,
    { type: 'assistant', text: 'Loaded message two.' },
  ] satisfies ChatItem[]
  const refreshedMessages = [
    firstInitialMessage,
    { type: 'assistant', text: 'Loaded message two updated.' },
    { type: 'assistant', text: 'New message.' },
  ] satisfies ChatItem[]
  const fetcher = vi
    .fn<typeof fetch>()
    .mockResolvedValueOnce(
      jsonResponse(
        messagesResponse(initialMessages, [], { start: 8, total: 10 }),
      ),
    )
    .mockResolvedValueOnce(jsonResponse(actions))
    .mockResolvedValueOnce(
      jsonResponse(
        messagesResponse(refreshedMessages, [], { start: 8, total: 11 }),
      ),
    )
    .mockResolvedValueOnce(jsonResponse(actions))
  vi.stubGlobal('fetch', fetcher)
  root = createRoot(container)
  const renderConversation = (refreshKey: number) => {
    root?.render(
      <ConversationPane
        session={session}
        refreshKey={refreshKey}
        sessionTreeRefreshKey={0}
        onSelectSession={() => undefined}
      />,
    )
  }
  act(() => {
    renderConversation(0)
  })
  await waitForState(
    () => container.querySelector('.chat-assistant-message') !== null,
  )
  const retainedMessage = requireElement(
    container.querySelector('.chat-assistant-message'),
  )
  act(() => {
    renderConversation(1)
  })
  await act(async () => {
    for (let index = 0; index < 12; index += 1) await Promise.resolve()
  })

  const snapshot = () => ({
    requests: fetcher.mock.calls.map(([url]) => url),
    retainedMessage:
      retainedMessage === container.querySelector('.chat-assistant-message'),
    messages: Array.from(
      container.querySelectorAll('.chat-assistant-message'),
      (element) => element.textContent,
    ),
  })
  expect(snapshot()).toEqual({
    requests: [
      '/api/sessions/sample-session/messages?limit=100',
      '/api/sessions/sample-session/actions',
      '/api/sessions/sample-session/messages?from=8',
      '/api/sessions/sample-session/actions',
    ],
    retainedMessage: true,
    messages: [
      'Loaded message one.',
      'Loaded message two updated.',
      'New message.',
    ],
  })
})

it('prepends an older page without moving existing messages in the viewport', async () => {
  const jsonResponse = (body: unknown) =>
    new Response(JSON.stringify(body), {
      headers: { 'Content-Type': 'application/json' },
    })
  const firstVisibleMessage: ChatItem = {
    type: 'assistant',
    text: 'First visible message.',
  }
  const visibleMessages = [
    firstVisibleMessage,
    { type: 'assistant', text: 'Latest visible message.' },
  ] satisfies ChatItem[]
  const fetcher = vi
    .fn<typeof fetch>()
    .mockResolvedValueOnce(
      jsonResponse(
        messagesResponse(visibleMessages, [], { start: 1, total: 3 }),
      ),
    )
    .mockResolvedValueOnce(jsonResponse(actions))
    .mockResolvedValueOnce(
      jsonResponse(
        messagesResponse([{ type: 'assistant', text: 'Older message.' }], [], {
          start: 0,
          total: 3,
        }),
      ),
    )
  vi.stubGlobal('fetch', fetcher)
  root = createRoot(container)
  act(() => {
    root?.render(
      <ConversationPane
        session={session}
        refreshKey={0}
        sessionTreeRefreshKey={0}
        onSelectSession={() => undefined}
      />,
    )
  })
  await waitForState(
    () => container.querySelectorAll('.chat-assistant-message').length === 2,
  )
  const scroll = requireElement(
    container.querySelector<HTMLDivElement>('.conversation-scroll'),
  )
  Object.defineProperties(scroll, {
    scrollHeight: {
      configurable: true,
      get: () =>
        100 + container.querySelectorAll('.chat-assistant-message').length * 40,
    },
    clientHeight: { configurable: true, value: 100 },
    scrollTop: { configurable: true, writable: true, value: 0 },
  })
  act(() => {
    scroll.scrollTop = 30
    scroll.dispatchEvent(new Event('scroll'))
  })
  const retainedMessage = requireElement(
    container.querySelector('.chat-assistant-message'),
  )
  act(() => {
    container
      .querySelector<HTMLButtonElement>('.conversation-load-older')
      ?.click()
  })
  await waitForState(
    () => container.querySelectorAll('.chat-assistant-message').length === 3,
  )

  const snapshot = () => ({
    requests: fetcher.mock.calls.map(([url]) => url),
    retainedMessage:
      retainedMessage ===
      container.querySelectorAll('.chat-assistant-message')[1],
    scrollTop: scroll.scrollTop,
    olderButton: container.querySelector('.conversation-load-older'),
    messages: Array.from(
      container.querySelectorAll('.chat-assistant-message'),
      (element) => element.textContent,
    ),
  })
  expect(snapshot()).toEqual({
    requests: [
      '/api/sessions/sample-session/messages?limit=100',
      '/api/sessions/sample-session/actions',
      '/api/sessions/sample-session/messages?before=1&limit=100',
    ],
    retainedMessage: true,
    scrollTop: 70,
    olderButton: null,
    messages: [
      'Older message.',
      'First visible message.',
      'Latest visible message.',
    ],
  })
})

it('loads an older page near the top and ignores repeated scrolls while it loads', async () => {
  const visibleMessages = [
    { type: 'assistant', text: 'First visible message.' },
    { type: 'assistant', text: 'Latest visible message.' },
  ] satisfies ChatItem[]
  let resolveOlderPage: ((response: Response) => void) | undefined
  const olderPage = new Promise<Response>((resolve) => {
    resolveOlderPage = resolve
  })
  const fetcher = vi
    .fn<typeof fetch>()
    .mockResolvedValueOnce(
      jsonResponse(
        messagesResponse(visibleMessages, [], { start: 1, total: 3 }),
      ),
    )
    .mockResolvedValueOnce(jsonResponse(actions))
    .mockReturnValueOnce(olderPage)
  vi.stubGlobal('fetch', fetcher)
  root = createRoot(container)
  act(() => {
    root?.render(
      <ConversationPane
        session={session}
        refreshKey={0}
        sessionTreeRefreshKey={0}
        onSelectSession={() => undefined}
      />,
    )
  })
  await waitForState(
    () => container.querySelectorAll('.chat-assistant-message').length === 2,
  )
  const scroll = requireElement(
    container.querySelector<HTMLDivElement>('.conversation-scroll'),
  )
  Object.defineProperties(scroll, {
    scrollHeight: {
      configurable: true,
      get: () =>
        100 + container.querySelectorAll('.chat-assistant-message').length * 40,
    },
    clientHeight: { configurable: true, value: 100 },
    scrollTop: { configurable: true, writable: true, value: 0 },
  })
  act(() => {
    scroll.scrollTop = 30
    scroll.dispatchEvent(new Event('scroll'))
    scroll.dispatchEvent(new Event('scroll'))
  })
  act(() => {
    scroll.scrollTop = 80
    scroll.dispatchEvent(new Event('scroll'))
  })
  await waitForState(() => fetcher.mock.calls.length === 3)
  const loadingState = {
    requests: fetcher.mock.calls.map(([url]) => requestUrl(url)),
    loadingLabel: container.querySelector('.conversation-load-older')
      ?.textContent,
  }
  await act(async () => {
    resolveOlderPage?.(
      jsonResponse(
        messagesResponse([{ type: 'assistant', text: 'Older message.' }], [], {
          start: 0,
          total: 3,
        }),
      ),
    )
    for (let index = 0; index < 12; index += 1) await Promise.resolve()
  })
  const anchoredScrollTop = scroll.scrollTop
  act(() => {
    scroll.scrollTop = 0
    scroll.dispatchEvent(new Event('scroll'))
  })

  const snapshot = () => ({
    loadingState,
    scrollTop: anchoredScrollTop,
    messages: Array.from(
      container.querySelectorAll('.chat-assistant-message'),
      (element) => element.textContent,
    ),
    olderButton: container.querySelector('.conversation-load-older'),
  })
  expect(snapshot()).toEqual({
    loadingState: {
      requests: [
        '/api/sessions/sample-session/messages?limit=100',
        '/api/sessions/sample-session/actions',
        '/api/sessions/sample-session/messages?before=1&limit=100',
      ],
      loadingLabel: 'Loading older messages…',
    },
    scrollTop: 120,
    messages: [
      'Older message.',
      'First visible message.',
      'Latest visible message.',
    ],
    olderButton: null,
  })
})

it('prunes to the latest 200 messages at the bottom and can load the discarded range again', async () => {
  const assistantMessage = (index: number): ChatItem => ({
    type: 'assistant',
    text: `Message ${String(index)}.`,
  })
  const decision = (seq: number): Extract<ChatItem, { type: 'decision' }> => ({
    type: 'decision',
    seq,
    title: `Decision ${String(seq)}`,
    why: 'Keep the behavior predictable.',
    considered: ['Use the existing flow'],
    easyToChange: 'Change one setting.',
    review: null,
    createdAt: '2026-11-11T03:25:59Z',
  })
  const firstPage = Array.from({ length: 100 }, (_, index) =>
    assistantMessage(index + 200),
  )
  const secondPage = Array.from({ length: 100 }, (_, index) =>
    assistantMessage(index + 100),
  )
  const thirdPage = Array.from({ length: 100 }, (_, index) =>
    assistantMessage(index),
  )
  const allDecisions = [decision(1), decision(2), decision(3)]
  const refreshedMessages = Array.from({ length: 301 }, (_, index) =>
    assistantMessage(index),
  )
  refreshedMessages[150] = proposal
  refreshedMessages[300] = question
  refreshedMessages[250] = decision(2)
  const refetchedOlderMessages = Array.from({ length: 100 }, (_, index) =>
    assistantMessage(index + 1),
  )
  const conversationMessageCount = () =>
    container.querySelectorAll(
      '.chat-assistant-message, .chat-question-card, .chat-proposal-card, .decision-card--conversation',
    ).length
  let refreshKey = 0
  const fetcher = vi.fn<typeof fetch>((input) => {
    const url = requestUrl(input)
    if (url.includes('before=200')) {
      return Promise.resolve(
        jsonResponse(
          messagesResponse(secondPage, [], { start: 100, total: 300 }),
        ),
      )
    }
    if (url.includes('before=100')) {
      return Promise.resolve(
        jsonResponse(messagesResponse(thirdPage, [], { start: 0, total: 300 })),
      )
    }
    if (url.includes('before=101')) {
      return Promise.resolve(
        jsonResponse(
          messagesResponse(refetchedOlderMessages, [], {
            start: 1,
            total: 301,
          }),
        ),
      )
    }
    if (url.includes('from=0')) {
      return Promise.resolve(
        jsonResponse(
          messagesResponse(refreshedMessages, [], {
            start: 0,
            total: 301,
            decisions: allDecisions,
            hasUnansweredProposal: true,
            activeQuestionIndex: 300,
          }),
        ),
      )
    }
    if (url.endsWith('/actions')) return Promise.resolve(jsonResponse(actions))
    return Promise.resolve(
      jsonResponse(messagesResponse(firstPage, [], { start: 200, total: 300 })),
    )
  })
  vi.stubGlobal('fetch', fetcher)
  root = createRoot(container)
  const renderConversation = () => {
    root?.render(
      <ConversationPane
        session={session}
        refreshKey={refreshKey}
        sessionTreeRefreshKey={0}
        onSelectSession={() => undefined}
        statusPanelOpen
      />,
    )
  }
  act(renderConversation)
  await waitForState(() => conversationMessageCount() === 100)
  const scroll = requireElement(
    container.querySelector<HTMLDivElement>('.conversation-scroll'),
  )
  Object.defineProperties(scroll, {
    scrollHeight: {
      configurable: true,
      get: () =>
        100 +
        container.querySelectorAll(
          '.chat-assistant-message, .chat-question-card, .chat-proposal-card, .decision-card--conversation',
        ).length *
          10,
    },
    clientHeight: { configurable: true, value: 100 },
    scrollTop: { configurable: true, writable: true, value: 0 },
  })
  act(() => {
    container
      .querySelector<HTMLButtonElement>('.conversation-load-older')
      ?.click()
  })
  await waitForState(() => conversationMessageCount() === 200)
  act(() => {
    container
      .querySelector<HTMLButtonElement>('.conversation-load-older')
      ?.click()
  })
  await waitForState(() => conversationMessageCount() === 300)
  const retainedMessage = requireElement(
    Array.from(container.querySelectorAll('.chat-assistant-message')).find(
      (element) => element.textContent === 'Message 101.',
    ) ?? null,
  )
  refreshKey += 1
  act(renderConversation)
  await waitForState(() => conversationMessageCount() === 200)
  const retainedMessageReused =
    retainedMessage === container.querySelector('.chat-assistant-message')
  const prunedState = {
    startOfMessages: container.querySelector('.chat-assistant-message')
      ?.textContent,
    endOfMessages: Array.from(
      container.querySelectorAll('.chat-assistant-message'),
    ).at(-1)?.textContent,
    decisionPosition: container.querySelector(
      '.decision-card--conversation .decision-card-position',
    )?.textContent,
    questionChoicesDisabled: Array.from(
      container.querySelectorAll<HTMLButtonElement>(
        '.chat-question-choices button',
      ),
      (button) => button.disabled,
    ),
    statusPanelDecisions: container.querySelectorAll('.decision-card--panel')
      .length,
    composerPlaceholder: container
      .querySelector<HTMLTextAreaElement>(
        'form.conversation-composer:not(.chat-question-composer) textarea',
      )
      ?.getAttribute('placeholder'),
  }
  act(() => {
    scroll.scrollTop = 0
    scroll.dispatchEvent(new Event('scroll'))
  })
  await waitForState(() => conversationMessageCount() === 300)

  const snapshot = () => ({
    requests: fetcher.mock.calls.map(([url]) => requestUrl(url)),
    loadedMessageCount: conversationMessageCount(),
    prunedState,
    reloadedFirstMessage: container.querySelector('.chat-assistant-message')
      ?.textContent,
    retainedMessageReused,
    olderButton: container.querySelector('.conversation-load-older') !== null,
  })
  expect(snapshot()).toEqual({
    requests: [
      '/api/sessions/sample-session/messages?limit=100',
      '/api/sessions/sample-session/actions',
      '/api/sessions/sample-session/messages?before=200&limit=100',
      '/api/sessions/sample-session/messages?before=100&limit=100',
      '/api/sessions/sample-session/messages?from=0',
      '/api/sessions/sample-session/actions',
      '/api/sessions/sample-session/messages?before=101&limit=100',
    ],
    loadedMessageCount: 300,
    prunedState: {
      startOfMessages: 'Message 101.',
      endOfMessages: 'Message 299.',
      decisionPosition: '2 of 3',
      questionChoicesDisabled: [false, false],
      statusPanelDecisions: 3,
      composerPlaceholder: 'Choose an option above, or message the session',
    },
    reloadedFirstMessage: 'Message 1.',
    retainedMessageReused: true,
    olderButton: true,
  })
})

it('re-enables older-page loading after a request fails so it can be retried', async () => {
  const jsonResponse = (body: unknown, status = 200) =>
    new Response(JSON.stringify(body), {
      status,
      headers: { 'Content-Type': 'application/json' },
    })
  const visibleMessages = [
    { type: 'assistant', text: 'First visible message.' },
    { type: 'assistant', text: 'Latest visible message.' },
  ] satisfies ChatItem[]
  const fetcher = vi
    .fn<typeof fetch>()
    .mockResolvedValueOnce(
      jsonResponse(
        messagesResponse(visibleMessages, [], { start: 1, total: 3 }),
      ),
    )
    .mockResolvedValueOnce(jsonResponse(actions))
    .mockResolvedValueOnce(
      jsonResponse({ error: 'Older messages unavailable' }, 503),
    )
    .mockResolvedValueOnce(
      jsonResponse(
        messagesResponse([{ type: 'assistant', text: 'Older message.' }], [], {
          start: 0,
          total: 3,
        }),
      ),
    )
  vi.stubGlobal('fetch', fetcher)
  root = createRoot(container)
  act(() => {
    root?.render(
      <ConversationPane
        session={session}
        refreshKey={0}
        sessionTreeRefreshKey={0}
        onSelectSession={() => undefined}
      />,
    )
  })
  await waitForState(
    () => container.querySelectorAll('.chat-assistant-message').length === 2,
  )
  const olderButton = requireElement(
    container.querySelector<HTMLButtonElement>('.conversation-load-older'),
  )
  act(() => {
    olderButton.click()
  })
  await waitForState(
    () =>
      container.querySelector('[role="alert"]')?.textContent ===
      'Older messages unavailable',
  )
  const stateAfterFailure = {
    messages: Array.from(
      container.querySelectorAll('.chat-assistant-message'),
      (element) => element.textContent,
    ),
    error: container.querySelector('[role="alert"]')?.textContent,
    buttonDisabled: olderButton.disabled,
  }
  act(() => {
    olderButton.click()
  })
  await waitForState(
    () => container.querySelectorAll('.chat-assistant-message').length === 3,
  )

  const snapshot = () => ({
    requests: fetcher.mock.calls.map(([url]) => url),
    stateAfterFailure,
    finalState: {
      messages: Array.from(
        container.querySelectorAll('.chat-assistant-message'),
        (element) => element.textContent,
      ),
      error: container.querySelector('[role="alert"]')?.textContent ?? null,
      olderButton: container.querySelector('.conversation-load-older'),
    },
  })
  expect(snapshot()).toEqual({
    requests: [
      '/api/sessions/sample-session/messages?limit=100',
      '/api/sessions/sample-session/actions',
      '/api/sessions/sample-session/messages?before=1&limit=100',
      '/api/sessions/sample-session/messages?before=1&limit=100',
    ],
    stateAfterFailure: {
      messages: ['First visible message.', 'Latest visible message.'],
      error: 'Older messages unavailable',
      buttonDisabled: false,
    },
    finalState: {
      messages: [
        'Older message.',
        'First visible message.',
        'Latest visible message.',
      ],
      error: null,
      olderButton: null,
    },
  })
})

it('keeps an older page loaded when a conversation refresh overlaps its request', async () => {
  const jsonResponse = (body: unknown) =>
    new Response(JSON.stringify(body), {
      headers: { 'Content-Type': 'application/json' },
    })
  const firstVisibleMessage: ChatItem = {
    type: 'assistant',
    text: 'First visible message.',
  }
  const visibleMessages = [
    firstVisibleMessage,
    { type: 'assistant', text: 'Latest visible message.' },
  ] satisfies ChatItem[]
  let resolveOlderPage: ((response: Response) => void) | undefined
  let resolveRefresh: ((response: Response) => void) | undefined
  const olderPage = new Promise<Response>((resolve) => {
    resolveOlderPage = resolve
  })
  const refreshPage = new Promise<Response>((resolve) => {
    resolveRefresh = resolve
  })
  const fetcher = vi.fn<typeof fetch>((input) => {
    const url = requestUrl(input)
    if (url.includes('before=')) return olderPage
    if (url.includes('from=')) return refreshPage
    if (url.endsWith('/actions')) return Promise.resolve(jsonResponse(actions))
    return Promise.resolve(
      jsonResponse(
        messagesResponse(visibleMessages, [], { start: 1, total: 3 }),
      ),
    )
  })
  vi.stubGlobal('fetch', fetcher)
  root = createRoot(container)
  const renderConversation = (refreshKey: number) => {
    root?.render(
      <ConversationPane
        session={session}
        refreshKey={refreshKey}
        sessionTreeRefreshKey={0}
        onSelectSession={() => undefined}
      />,
    )
  }
  act(() => {
    renderConversation(0)
  })
  await waitForState(
    () => container.querySelectorAll('.chat-assistant-message').length === 2,
  )
  act(() => {
    container
      .querySelector<HTMLButtonElement>('.conversation-load-older')
      ?.click()
  })
  act(() => {
    renderConversation(1)
  })
  await waitForState(() =>
    fetcher.mock.calls.some(([url]) => requestUrl(url).includes('from=')),
  )
  await act(async () => {
    resolveOlderPage?.(
      jsonResponse(
        messagesResponse([{ type: 'assistant', text: 'Older message.' }], [], {
          start: 0,
          total: 3,
        }),
      ),
    )
    for (let index = 0; index < 12; index += 1) await Promise.resolve()
  })
  await waitForState(
    () => container.querySelectorAll('.chat-assistant-message').length === 3,
  )
  await act(async () => {
    resolveRefresh?.(
      jsonResponse(
        messagesResponse(
          [
            firstVisibleMessage,
            { type: 'assistant', text: 'Latest visible message updated.' },
            { type: 'assistant', text: 'New message.' },
          ],
          [],
          { start: 1, total: 4 },
        ),
      ),
    )
    for (let index = 0; index < 12; index += 1) await Promise.resolve()
  })

  const snapshot = () => ({
    requests: fetcher.mock.calls.map(([url]) => requestUrl(url)),
    messages: Array.from(
      container.querySelectorAll('.chat-assistant-message'),
      (element) => element.textContent,
    ),
    olderButton: container.querySelector('.conversation-load-older'),
  })
  expect(snapshot()).toEqual({
    requests: [
      '/api/sessions/sample-session/messages?limit=100',
      '/api/sessions/sample-session/actions',
      '/api/sessions/sample-session/messages?before=1&limit=100',
      '/api/sessions/sample-session/messages?from=1',
      '/api/sessions/sample-session/actions',
    ],
    messages: [
      'Older message.',
      'First visible message.',
      'Latest visible message updated.',
      'New message.',
    ],
    olderButton: null,
  })
})

it('uses full conversation metadata for questions, proposals, and decisions', async () => {
  const jsonResponse = (body: unknown) =>
    new Response(JSON.stringify(body), {
      headers: { 'Content-Type': 'application/json' },
    })
  const decision = (
    seq: number,
    title: string,
  ): Extract<ChatItem, { type: 'decision' }> => ({
    type: 'decision',
    seq,
    title,
    why: 'Keep behavior predictable.',
    considered: ['Use the simpler path'],
    easyToChange: 'Change one setting.',
    review: null,
    createdAt: '2026-11-11T03:25:59Z',
  })
  const firstQuestion: ChatItem = {
    ...question,
    questions: [
      {
        ...questionPrompt,
        question: 'The active question from the loaded page?',
      },
    ],
  }
  const laterQuestion: ChatItem = {
    ...question,
    questions: [
      {
        ...questionPrompt,
        question: 'A later, already answered question?',
        answer: 'Window B',
      },
    ],
  }
  const visibleDecision = decision(2, 'Visible decision')
  const fullDecisions = [
    decision(1, 'Earlier decision'),
    visibleDecision,
    decision(3, 'Later decision'),
  ]
  const page = messagesResponse(
    [firstQuestion, visibleDecision, laterQuestion],
    [],
    {
      start: 10,
      total: 20,
      decisions: fullDecisions,
      hasUnansweredProposal: true,
      activeQuestionIndex: 10,
    },
  )
  const fetcher = vi
    .fn<typeof fetch>()
    .mockResolvedValueOnce(jsonResponse(page))
    .mockResolvedValueOnce(jsonResponse(actions))
  vi.stubGlobal('fetch', fetcher)
  root = createRoot(container)
  act(() => {
    root?.render(
      <ConversationPane
        session={session}
        refreshKey={0}
        sessionTreeRefreshKey={0}
        onSelectSession={() => undefined}
        statusPanelOpen
      />,
    )
  })
  await waitForState(
    () =>
      container.querySelectorAll('.chat-question-choices button').length === 4,
  )

  const snapshot = () => ({
    request: fetcher.mock.calls.map(([url]) => url),
    questionChoicesDisabled: Array.from(
      container.querySelectorAll<HTMLButtonElement>(
        '.chat-question-choices button',
      ),
      (button) => button.disabled,
    ),
    conversationDecisionPosition: container.querySelector(
      '.decision-card--conversation .decision-card-position',
    )?.textContent,
    statusPanelDecisionCount: container.querySelectorAll(
      '.decision-card--panel',
    ).length,
    composerPlaceholder: container
      .querySelector<HTMLTextAreaElement>(
        'form.conversation-composer:not(.chat-question-composer) textarea',
      )
      ?.getAttribute('placeholder'),
  })
  expect(snapshot()).toEqual({
    request: [
      '/api/sessions/sample-session/messages?limit=100',
      '/api/sessions/sample-session/actions',
    ],
    questionChoicesDisabled: [false, false, true, true],
    conversationDecisionPosition: '2 of 3',
    statusPanelDecisionCount: 3,
    composerPlaceholder: 'Choose an option above, or message the session',
  })
})

it('skips committing an unchanged memoized conversation row', () => {
  const message: ChatItem = {
    type: 'assistant',
    text: 'The conversation is unchanged.',
  }
  const sessions: Session[] = []
  const onAction = () => Promise.resolve(true)
  const onOpenInTerminal = () => undefined
  const onSelectDecision = () => undefined
  const onSelectSession = () => undefined
  const renderMessage = vi.mocked(renderConversationMessage)
  renderMessage.mockClear()
  const rowProps = {
    actions,
    actionsReady: true,
    isMobile: false,
    selected: false,
    decisionPosition: undefined,
    message,
    pendingAction: null,
    openInTerminalStatus: null,
    sessionEngine: 'claude' as const,
    sessions,
    onAction,
    onOpenInTerminal,
    onSelectDecision,
    onSelectSession,
  }
  root = createRoot(container)
  act(() => {
    root?.render(<ConversationMessageRow {...rowProps} />)
  })
  act(() => {
    root?.render(<ConversationMessageRow {...rowProps} />)
  })

  expect(renderMessage).toHaveBeenCalledTimes(1)
})

it('refreshes a memoized message time when the local day changes', () => {
  const timestamp = new Date(2030, 8, 11, 23, 58).toISOString()
  const message: ChatItem = {
    type: 'event',
    kind: 'system',
    text: 'A fictional event occurred.',
    failed: false,
    timestamp,
  }
  const sessions: Session[] = []
  const onAction = () => Promise.resolve(true)
  const onOpenInTerminal = () => undefined
  const onSelectDecision = () => undefined
  const onSelectSession = () => undefined
  const renderMessage = vi.mocked(renderConversationMessage)
  renderMessage.mockClear()
  const beforeMidnight = new Date(2030, 8, 11, 23, 59)
  const rowProps = {
    actions,
    actionsReady: true,
    isMobile: false,
    selected: false,
    decisionPosition: undefined,
    message,
    now: beforeMidnight,
    pendingAction: null,
    openInTerminalStatus: null,
    sessionEngine: 'claude' as const,
    sessions,
    onAction,
    onOpenInTerminal,
    onSelectDecision,
    onSelectSession,
  }
  const afterMidnight = new Date(2030, 8, 12, 0, 1)
  root = createRoot(container)
  act(() => {
    root?.render(<ConversationMessageRow {...rowProps} />)
  })
  act(() => {
    root?.render(<ConversationMessageRow {...rowProps} now={beforeMidnight} />)
  })
  act(() => {
    root?.render(<ConversationMessageRow {...rowProps} now={afterMidnight} />)
  })

  const getActual = () => ({
    renderCount: renderMessage.mock.calls.length,
    time: container.querySelector('time')?.textContent ?? null,
  })
  expect(getActual()).toEqual({
    renderCount: 2,
    time: formatMessageTime(timestamp, afterMidnight),
  })
})

it('passes the changed local day through the memoized message elements', () => {
  const timestamp = new Date(2030, 8, 11, 23, 58).toISOString()
  const beforeMidnight = new Date(2030, 8, 11, 23, 59)
  const afterMidnight = new Date(2030, 8, 12, 0, 1)
  const message: ChatItem = {
    type: 'event',
    kind: 'system',
    text: 'A fictional event occurred.',
    failed: false,
    timestamp,
  }
  const messages = [message]
  const sessions: Session[] = []
  const pendingPermissionActions = {}
  const onAction = () => Promise.resolve(true)
  const onSelectDecision = () => undefined
  const onSelectSession = () => undefined
  const render = () => {
    root ??= createRoot(container)
    act(() => {
      root?.render(
        <ConversationPaneView
          session={session}
          sessions={sessions}
          messages={messages}
          actions={null}
          pendingPermissionActions={pendingPermissionActions}
          loading={false}
          error={null}
          onAction={onAction}
          onSelectSession={onSelectSession}
          onSelectDecision={onSelectDecision}
        />,
      )
    })
  }
  vi.useFakeTimers()
  try {
    vi.setSystemTime(beforeMidnight)
    render()
    vi.setSystemTime(afterMidnight)
    render()

    const getActual = () => container.querySelector('time')?.textContent ?? null
    expect(getActual()).toBe(formatMessageTime(timestamp, afterMidnight))
  } finally {
    vi.useRealTimers()
  }
})

it('keeps a todo comment when a preceding conversation row arrives', () => {
  const todo: ChatItem = {
    type: 'todo',
    seq: 12,
    title: 'Review the deployment plan',
    detail: null,
    completed: false,
    comment: null,
  }
  renderConversationMessages([todo], 0)
  const textarea = requireElement(
    container.querySelector<HTMLTextAreaElement>('.chat-todo-comment'),
  )
  setTextareaValue(textarea, 'Keep the rollback step')

  renderConversationMessages(
    [{ type: 'assistant', text: 'A new reply arrived.' }, todo],
    1,
  )

  expectDraftRetained(textarea, '.chat-todo-comment', 'Keep the rollback step')
})

it('keeps a decision change comment when a preceding conversation row arrives', () => {
  const decision: ChatItem = {
    type: 'decision',
    seq: 14,
    title: 'Use a bounded retry policy',
    why: 'The service has no reliable duration estimate.',
    considered: ['Retry using the latest duration'],
    easyToChange: 'Adjust one configuration value.',
    review: null,
    createdAt: '2026-11-11T12:00:00Z',
  }
  renderConversationMessages([decision], 0)
  act(() => {
    requireElement(
      container.querySelector<HTMLButtonElement>('.decision-card-summary'),
    ).click()
  })
  act(() => {
    requireElement(
      Array.from(container.querySelectorAll<HTMLButtonElement>('button')).find(
        (button) => button.textContent.trim() === 'Change…',
      ) ?? null,
    ).click()
  })
  const textarea = requireElement(
    container.querySelector<HTMLTextAreaElement>(
      '.decision-card-change-form textarea',
    ),
  )
  setTextareaValue(textarea, 'Include a timeout limit')

  renderConversationMessages(
    [{ type: 'assistant', text: 'A new reply arrived.' }, decision],
    1,
  )

  expectDraftRetained(
    textarea,
    '.decision-card-change-form textarea',
    'Include a timeout limit',
  )
})

it('keeps a question answer draft when a preceding conversation row arrives', () => {
  const question: ChatItem = {
    type: 'question',
    requestId: 'question-request-3',
    questions: [
      {
        header: 'Retry behavior',
        question: 'Which behavior should be used?',
        choices: [{ label: 'Retry once' }, { label: 'Do not retry' }],
        multi_select: true,
        answer: null,
      },
    ],
  }
  renderConversationMessages([question], 0)
  const textarea = requireElement(
    container.querySelector<HTMLTextAreaElement>(
      '.chat-question-prompt textarea',
    ),
  )
  setTextareaValue(textarea, 'Use the bounded option')

  renderConversationMessages(
    [{ type: 'assistant', text: 'A new reply arrived.' }, question],
    1,
  )

  expectDraftRetained(
    textarea,
    '.chat-question-prompt textarea',
    'Use the bounded option',
  )
})

it('keeps repeated tool summaries attached to their batch when older rows arrive', () => {
  const earlierSummary: ChatItem = {
    type: 'tool_summary',
    summary: 'Read ×2',
    operations: [
      { tool: 'Read', target: 'one.rs', failed: false },
      { tool: 'Read', target: 'two.rs', failed: false },
    ],
  }
  const summary: ChatItem = {
    type: 'tool_summary',
    summary: 'Read ×1',
    operations: [{ tool: 'Read', target: 'one.rs', failed: false }],
  }
  renderConversationMessages([summary], 0)
  const details = requireElement(
    container.querySelector<HTMLDetailsElement>('.chat-tool-summary'),
  )
  act(() => {
    requireElement(details.querySelector<HTMLElement>('summary')).click()
  })

  renderConversationMessages([earlierSummary, summary], 0)

  const snapshot = () =>
    Array.from(
      container.querySelectorAll<HTMLDetailsElement>('.chat-tool-summary'),
      (currentDetails) => ({
        sameDetails: currentDetails === details,
        open: currentDetails.open,
        operationCount: currentDetails.querySelectorAll('li').length,
      }),
    )
  expect(snapshot()).toEqual([
    { sameDetails: false, open: false, operationCount: 2 },
    { sameDetails: true, open: true, operationCount: 1 },
  ])
})

it('keeps message rows memoized for elapsed and closed-task updates', () => {
  const message: ChatItem = {
    type: 'assistant',
    text: 'The conversation is unchanged.',
  }
  const messages = [message]
  const initialSession = {
    ...session,
    elapsedSeconds: 1,
    task: {
      id: 'fixture-linked-task',
      number: 17,
      title: 'Refresh linked sessions',
      priority: 1,
      isClosed: false,
    },
  }
  const refreshedSession = {
    ...initialSession,
    elapsedSeconds: 2,
    task: {
      id: 'fixture-linked-task',
      number: 17,
      title: 'Refresh linked sessions',
      priority: 1,
      isClosed: true,
    },
  }
  const initialSessions = [initialSession]
  const priorityChangedSession = {
    ...refreshedSession,
    task: { ...refreshedSession.task, priority: 2 },
  }
  const priorityChangedSessions = [priorityChangedSession]
  const onAction = () => Promise.resolve(true)
  const onSelectSession = () => undefined
  const renderMessage = vi.mocked(renderConversationMessage)
  renderMessage.mockClear()
  root = createRoot(container)

  act(() => {
    root?.render(
      <ConversationPaneView
        session={initialSession}
        sessions={initialSessions}
        messages={messages}
        actions={null}
        loading={false}
        error={null}
        onAction={onAction}
        onSelectSession={onSelectSession}
        isMobile
      />,
    )
  })
  act(() => {
    root?.render(
      <ConversationPaneView
        session={refreshedSession}
        sessions={[refreshedSession]}
        messages={messages}
        actions={null}
        loading={false}
        error={null}
        onAction={onAction}
        onSelectSession={onSelectSession}
        isMobile
      />,
    )
  })
  const renderCountAfterUnrelatedRefresh = renderMessage.mock.calls.length
  act(() => {
    root?.render(
      <ConversationPaneView
        session={priorityChangedSession}
        sessions={priorityChangedSessions}
        messages={messages}
        actions={null}
        loading={false}
        error={null}
        onAction={onAction}
        onSelectSession={onSelectSession}
        isMobile
      />,
    )
  })

  const getActual = () => ({
    afterUnrelatedRefresh: renderCountAfterUnrelatedRefresh,
    afterPriorityChange: renderMessage.mock.calls.length,
  })
  expect(getActual()).toEqual({
    afterUnrelatedRefresh: 1,
    afterPriorityChange: 2,
  })
})

it('keeps an unsent proposal comment when its position changes after refresh', async () => {
  const jsonResponse = (body: unknown) =>
    new Response(JSON.stringify(body), {
      headers: { 'Content-Type': 'application/json' },
    })
  const queuedMessage: ChatItem = {
    type: 'queued_user',
    content: [{ type: 'text', text: 'A fictional queued request.' }],
  }
  const fetcher = vi
    .fn<typeof fetch>()
    .mockResolvedValueOnce(jsonResponse(messagesResponse([proposal])))
    .mockResolvedValueOnce(jsonResponse(actions))
    .mockResolvedValueOnce(
      jsonResponse(messagesResponse([queuedMessage, proposal])),
    )
    .mockResolvedValueOnce(jsonResponse(actions))
  vi.stubGlobal('fetch', fetcher)
  root = createRoot(container)
  act(() => {
    root?.render(
      <ConversationPane
        session={session}
        refreshKey={0}
        sessionTreeRefreshKey={0}
        onSelectSession={() => undefined}
      />,
    )
  })
  await waitForState(
    () => container.querySelector('.chat-proposal-comment') !== null,
  )
  const comment = requireElement(
    container.querySelector<HTMLTextAreaElement>('.chat-proposal-comment'),
  )
  setTextareaValue(comment, 'Keep this draft')

  act(() => {
    root?.render(
      <ConversationPane
        session={session}
        refreshKey={1}
        sessionTreeRefreshKey={0}
        onSelectSession={() => undefined}
      />,
    )
  })
  await act(async () => {
    for (let index = 0; index < 12; index += 1) await Promise.resolve()
  })

  const snapshot = () => ({
    sameCommentNode:
      comment === container.querySelector('.chat-proposal-comment'),
    commentValue: container.querySelector<HTMLTextAreaElement>(
      '.chat-proposal-comment',
    )?.value,
    requestCount: fetcher.mock.calls.length,
  })
  expect(snapshot()).toEqual({
    sameCommentNode: true,
    commentValue: 'Keep this draft',
    requestCount: 4,
  })
})
