import { act, type ReactNode, useState } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { userEvent } from 'vitest/browser'

import {
  AsyncActionFeedbackProvider,
  useAsyncActionStarter,
  useOptionalAsyncActionFeedback,
} from '#async-action-feedback'
import { getCommandPaletteShortcut } from '#command-palette-shortcut'
import { renderConversationMessage } from '#components/conversation-message'
import { ConversationMessageRow } from '#components/conversation-message-row'
import { ConversationPane } from '#components/conversation-pane'
import { ConversationPaneView } from '#components/conversation-pane-view'
import {
  clearOffscreenConversationScrollCompensation,
  type ConversationAnchorSpacing,
  readConversationChildLayout,
} from '#components/conversation-scroll-anchor'
import { InboxConversationPanes } from '#components/inbox-conversation-panes'
import { MarkdownSessionLinksProvider } from '#components/markdown-message'
import { formatMessageTime } from '#components/message-time'
import { SessionConversationCacheProvider } from '#components/session-conversation-cache'
import { SessionStatusPanel } from '#components/session-status-panel'
import { clearSessionAttachments } from '#components/use-composer-message'
import { useConversationStatusPanel } from '#components/use-conversation-status-panel'
import { useSessionLinkTargets } from '#session-link-targets'
import type {
  BackgroundTask,
  ChatItem,
  RewindMode,
  Session,
  SessionActions,
  SessionTask,
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
  rewind: false,
}

const codexTerminalActions: SessionActions = {
  message: true,
  answerQuestion: false,
  permission: { approve: false, reject: false },
  interrupt: true,
  openInTerminal: true,
  rewind: false,
}

const endedSession: Session = {
  ...session,
  updatedAt: '2026-01-01T05:32:00.000Z',
}

const endedActions: SessionActions = {
  ...actions,
  message: false,
  answerQuestion: false,
  resume: true,
  interrupt: false,
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
  resolution: null,
  comment: null,
}

function messagesResponse(
  messages: ChatItem[],
  backgroundTasks: BackgroundTask[] = [],
  metadata: {
    start: number
    total: number
    epoch?: string
    olderCursor?: string
    epochReset?: boolean
    stableStart?: number
    stableEnd?: number
    stableDigest?: string
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
    ...(metadata.epoch === undefined ? {} : { epoch: metadata.epoch }),
    ...(metadata.olderCursor === undefined
      ? {}
      : { olderCursor: metadata.olderCursor }),
    ...(metadata.epochReset === undefined
      ? {}
      : { epochReset: metadata.epochReset }),
    start: metadata.start,
    total: metadata.total,
    ...(metadata.stableStart === undefined
      ? {}
      : { stableStart: metadata.stableStart }),
    ...(metadata.stableEnd === undefined
      ? {}
      : { stableEnd: metadata.stableEnd }),
    ...(metadata.stableDigest === undefined
      ? {}
      : { stableDigest: metadata.stableDigest }),
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

function normalizeQueuedMessagePostBody(body: BodyInit | null | undefined) {
  if (typeof body !== 'string') return body
  const parsed: unknown = JSON.parse(body)
  if (typeof parsed !== 'object' || parsed === null || Array.isArray(parsed)) {
    return body
  }
  const messageId: unknown = Reflect.get(parsed, 'messageId')
  if (typeof messageId !== 'string') return body
  return JSON.stringify({ ...parsed, messageId: '<message-id>' })
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

const permissionDenied: ChatItem = {
  type: 'permission_denied',
  seq: 7,
  toolUseId: 'fixture-denied-tool-call',
  tool: 'Shell',
  inputSummary: 'Run the fictional cleanup command.',
  reason: 'The operation is not allowed.',
  retryable: false,
  resolved: false,
  resolution: null,
}
let container: HTMLDivElement
let root: Root | null = null
const renderedSessionIds = new Set<string>()
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

function stubResizeObserver() {
  const observers: Array<{
    callback: ResizeObserverCallback
    targets: Set<Element>
    instance: ResizeObserver
  }> = []
  class ControlledResizeObserver implements ResizeObserver {
    private readonly record: (typeof observers)[number]

    constructor(callback: ResizeObserverCallback) {
      this.record = { callback, targets: new Set(), instance: this }
      observers.push(this.record)
    }

    observe(target: Element) {
      this.record.targets.add(target)
    }

    unobserve(target: Element) {
      this.record.targets.delete(target)
    }

    disconnect() {
      this.record.targets.clear()
    }
  }
  vi.stubGlobal('ResizeObserver', ControlledResizeObserver)

  return (target: Element, height = target.getBoundingClientRect().height) => {
    const bounds = target.getBoundingClientRect()
    const size = { inlineSize: bounds.width, blockSize: height }
    for (const observer of observers) {
      if (!observer.targets.has(target)) continue
      observer.callback(
        [
          {
            target,
            contentRect: new DOMRectReadOnly(
              bounds.x,
              bounds.y,
              bounds.width,
              height,
            ),
            borderBoxSize: [size],
            contentBoxSize: [size],
            devicePixelContentBoxSize: [size],
          },
        ],
        observer.instance,
      )
    }
  }
}

function mockScrollDimensions(
  scroll: HTMLDivElement,
  initialHeight: number,
  clientHeight: number,
) {
  let height = initialHeight
  Object.defineProperties(scroll, {
    scrollHeight: { configurable: true, get: () => height },
    clientHeight: { configurable: true, value: clientHeight },
    scrollTop: { configurable: true, writable: true, value: 0 },
  })
  return (nextHeight: number) => {
    height = nextHeight
  }
}

function scrollToPosition(scroll: HTMLDivElement, scrollTop: number) {
  act(() => {
    scroll.scrollTop = scrollTop
    scroll.dispatchEvent(new Event('scroll'))
  })
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

function normalizeAsyncActionLineTime(text: string | null | undefined) {
  return text?.replace(/ · \d{2}:\d{2}$/, ' · <time>') ?? null
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
  onRewind?: (
    messageId: string,
    mode: RewindMode,
    draft: string,
    timelineIndex?: number,
  ) => Promise<boolean>,
) {
  renderedSessionIds.add(viewSession.id)
  root ??= createRoot(container)
  act(() => {
    root?.render(
      <SessionLinkContextHarness
        sessions={viewSessions}
        onSelectSession={onSelectSession}
      >
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
          {...(onRewind === undefined ? {} : { onRewind })}
          onSelectSession={onSelectSession}
          isMobile={isMobile}
          isToday={today.isToday}
          onToggleToday={today.onToggleToday}
          statusPanelOpen={statusPanelOpen}
          mobileStatusPanel={mobileStatusPanel}
          {...(onToggleStatusPanel === undefined
            ? {}
            : { onToggleStatusPanel })}
        />
      </SessionLinkContextHarness>,
    )
  })
}

function getCompactingStatusSnapshot() {
  return {
    workingStatus: container.querySelector('.conversation-status-band-label')
      ?.textContent,
    detail:
      container.querySelector('.conversation-status-band-inline-detail')
        ?.textContent ?? null,
    inProgressItems: container.querySelectorAll('.tool-in-progress').length,
  }
}

it('keeps the phone composer reachable when several merge rows fill the viewport', () => {
  container.style.cssText =
    'position: fixed; inset: 0 auto auto 0; width: 390px; height: 260px'
  const pullRequests = [42, 43, 44].map((number) => ({
    number,
    state: 'Open' as const,
    owner: 'example-org',
    repo: `example-service-${String(number)}`,
    title: `Update service ${String(number)}`,
    url: `https://github.com/example-org/example-service-${String(number)}/pull/${String(number)}`,
    headSha: `fixture-head-${String(number)}`,
    ciState: 'Successful' as const,
    isCurrentBranch: number === 42,
  }))
  const currentPullRequest = pullRequests[0]
  if (currentPullRequest === undefined) {
    throw new Error('Expected current pull request')
  }
  renderView(
    () => Promise.resolve(true),
    { ...session, pullRequest: currentPullRequest, pullRequests },
    undefined,
    [],
    actions,
    true,
  )

  const snapshot = () => {
    const pane = container.querySelector('.conversation-pane')
    const slot = container.querySelector('.conversation-merge-band-slot')
    const sendButton = container.querySelector<HTMLButtonElement>(
      '.conversation-composer [aria-label="Send message"]',
    )
    const paneBounds = pane?.getBoundingClientRect()
    const sendButtonBounds = sendButton?.getBoundingClientRect()
    return {
      mergeRows: container.querySelectorAll('.merge-band-row').length,
      mergeBandOverflow:
        slot === null ? null : getComputedStyle(slot).overflowY,
      sendButtonVisible:
        paneBounds !== undefined &&
        sendButtonBounds !== undefined &&
        sendButtonBounds.top >= paneBounds.top &&
        sendButtonBounds.bottom <= paneBounds.bottom,
    }
  }
  expect(snapshot()).toEqual({
    mergeRows: 3,
    mergeBandOverflow: 'auto',
    sendButtonVisible: true,
  })
})

it('resumes an ended session and restores the composer after it becomes active', async () => {
  let resolveResume: ((success: boolean) => void) | undefined
  const resumeRequest = new Promise<boolean>((resolve) => {
    resolveResume = resolve
  })
  const calls: string[] = []
  const onAction: ActionHandler = (action) => {
    calls.push(action)
    return resumeRequest
  }
  renderView(onAction, endedSession, () => undefined, [], endedActions)

  const resumeButton = requireElement(
    container.querySelector<HTMLButtonElement>('.conversation-ended-resume'),
  )
  await act(async () => {
    await userEvent.click(resumeButton)
  })
  const pending = {
    label: resumeButton.textContent,
    disabled: resumeButton.disabled,
    composerVisible:
      container.querySelector('.conversation-composer textarea') !== null,
    title: container.querySelector('.conversation-ended-title')?.textContent,
    endedAt: container
      .querySelector('.conversation-ended time')
      ?.getAttribute('datetime'),
    hint: container.querySelector('.conversation-ended-hint')?.textContent,
  }

  await act(async () => {
    resolveResume?.(true)
    await resumeRequest
  })
  const afterRequest = {
    label: container.querySelector('.conversation-ended-resume')?.textContent,
    composerVisible:
      container.querySelector('.conversation-composer textarea') !== null,
  }

  renderView(onAction, endedSession, () => undefined, [], actions)
  const afterSessionResumes = {
    endedBannerVisible: container.querySelector('.conversation-ended') !== null,
    composerVisible:
      container.querySelector('.conversation-composer textarea') !== null,
  }

  const getActual = () => ({
    calls,
    pending,
    afterRequest,
    afterSessionResumes,
  })
  const expected = {
    calls: ['resume'],
    pending: {
      label: 'Resuming…',
      disabled: true,
      composerVisible: false,
      title: 'This session has ended',
      endedAt: '2026-01-01T05:32:00.000Z',
      hint: ' · Resume it to keep talking',
    },
    afterRequest: { label: 'Resuming…', composerVisible: false },
    afterSessionResumes: { endedBannerVisible: false, composerVisible: true },
  }
  expect(getActual()).toEqual(expected)
})

it('disables Resume when the session has no tmux location', () => {
  renderView(
    () => Promise.resolve(true),
    endedSession,
    () => undefined,
    [],
    { ...endedActions, resume: false },
  )

  const resumeButton = requireElement(
    container.querySelector<HTMLButtonElement>('.conversation-ended-resume'),
  )
  const actual = () => ({
    label: resumeButton.textContent,
    disabled: resumeButton.disabled,
    title: resumeButton.title,
    hint: container.querySelector('.conversation-ended-hint')?.textContent,
  })
  expect(actual()).toEqual({
    label: 'Resume',
    disabled: true,
    title: 'This session has no tmux location',
    hint: ' · Resume is unavailable because this session has no tmux location',
  })
})

it('explains why Resume is disabled when the working directory is missing', () => {
  renderView(
    () => Promise.resolve(true),
    endedSession,
    () => undefined,
    [],
    {
      ...endedActions,
      resume: false,
      resumeDisabledReason: 'workingDirectoryMissing',
    },
  )

  const resumeButton = requireElement(
    container.querySelector<HTMLButtonElement>('.conversation-ended-resume'),
  )
  const actual = () => ({
    label: resumeButton.textContent,
    disabled: resumeButton.disabled,
    title: resumeButton.title,
    hint: container.querySelector('.conversation-ended-hint')?.textContent,
  })
  expect(actual()).toEqual({
    label: 'Resume',
    disabled: true,
    title: 'This session’s working directory does not exist',
    hint: ' · Resume is unavailable because this session’s working directory does not exist',
  })
})

it.each([
  [
    'no tmux location',
    'noTmux',
    'Resume is unavailable because this session has no tmux location',
  ],
  [
    'a missing working directory',
    'workingDirectoryMissing',
    'Resume is unavailable because this session’s working directory does not exist',
  ],
] as const)(
  'shows the disabled reason on mobile with %s',
  (_condition, reason, hint) => {
    renderView(
      () => Promise.resolve(true),
      endedSession,
      () => undefined,
      [],
      { ...endedActions, resume: false, resumeDisabledReason: reason },
      true,
    )

    const resumeButton = requireElement(
      container.querySelector<HTMLButtonElement>('.conversation-ended-resume'),
    )
    const actual = () => ({
      mobile: container
        .querySelector('.conversation-pane')
        ?.getAttribute('data-mobile'),
      label: resumeButton.textContent,
      disabled: resumeButton.disabled,
      hint: container.querySelector('.conversation-ended-hint')?.textContent,
    })
    expect(actual()).toEqual({
      mobile: 'true',
      label: 'Resume session',
      disabled: true,
      hint: ` · ${hint}`,
    })
  },
)

type SettleResumeRequest = (
  resolve: (success: boolean) => void,
  reject: (reason: Error) => void,
) => void

const resumeFailureCases: [string, SettleResumeRequest][] = [
  [
    'a false result',
    (resolve) => {
      resolve(false)
    },
  ],
  [
    'a rejected request',
    (_resolve, reject) => {
      reject(new Error('fixture error'))
    },
  ],
]

it.each(resumeFailureCases)(
  're-enables Resume after %s',
  async (_name, settleRequest) => {
    let resolveResume: ((success: boolean) => void) | undefined
    let rejectResume: ((reason: Error) => void) | undefined
    const resumeRequest = new Promise<boolean>((resolve, reject) => {
      resolveResume = resolve
      rejectResume = reject
    })
    const calls: string[] = []
    const onAction: ActionHandler = (action) => {
      calls.push(action)
      return resumeRequest
    }
    renderView(onAction, endedSession, () => undefined, [], endedActions)

    const resumeButton = requireElement(
      container.querySelector<HTMLButtonElement>('.conversation-ended-resume'),
    )
    await act(async () => {
      await userEvent.click(resumeButton)
    })
    await act(async () => {
      if (resolveResume === undefined || rejectResume === undefined) {
        throw new Error('Resume request was not initialized')
      }
      settleRequest(resolveResume, rejectResume)
      await resumeRequest.catch(() => undefined)
    })

    const actual = () => ({
      calls,
      label: resumeButton.textContent,
      disabled: resumeButton.disabled,
    })
    expect(actual()).toEqual({
      calls: ['resume'],
      label: 'Resume',
      disabled: false,
    })
  },
)

it('allows retrying when a successful request does not start the session', async () => {
  vi.useFakeTimers()
  const calls: string[] = []
  const onAction: ActionHandler = (action) => {
    calls.push(action)
    return Promise.resolve(true)
  }
  renderView(onAction, endedSession, () => undefined, [], endedActions)

  const resumeButton = requireElement(
    container.querySelector<HTMLButtonElement>('.conversation-ended-resume'),
  )
  await act(async () => {
    resumeButton.click()
    await Promise.resolve()
  })
  await act(async () => {
    await vi.advanceTimersByTimeAsync(35_000)
  })
  await act(async () => {
    resumeButton.click()
    await Promise.resolve()
  })

  const actual = () => ({
    calls,
    label: resumeButton.textContent,
    disabled: resumeButton.disabled,
  })
  expect(actual()).toEqual({
    calls: ['resume', 'resume'],
    label: 'Resuming…',
    disabled: true,
  })
})

function SessionLinkContextHarness({
  children,
  sessions,
  onSelectSession,
}: {
  children: ReactNode
  sessions: readonly Session[]
  onSelectSession: (id: string) => void
}) {
  const sessionLinkTargets = useSessionLinkTargets(sessions)
  return (
    <MarkdownSessionLinksProvider
      sessions={sessionLinkTargets}
      onSelectSession={onSelectSession}
    >
      {children}
    </MarkdownSessionLinksProvider>
  )
}

function renderConversationMessages(
  messages: readonly ChatItem[],
  start: number,
  isMobile = false,
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
        isMobile={isMobile}
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
    vibeMode: { mode: 'disabled' },
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
    showAllWaiting: false,
    onShowMoreWaiting: () => undefined,
    onSelect: () => undefined,
    laterExpanded: false,
    onToggleLater: () => undefined,
    onLater: () => undefined,
    onToggleToday: () => undefined,
    todaySessions: [],
    onSelectToday: () => undefined,
    inProgressExpanded: false,
    onToggleInProgress: () => undefined,
    waitingExpanded: true,
    onToggleWaiting: () => undefined,
    doneExpanded: false,
    onToggleDone: () => undefined,
  }
  act(() => {
    root?.render(
      <SessionConversationCacheProvider>
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
        />
      </SessionConversationCacheProvider>,
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

async function openMobileActionsMenu() {
  const trigger = requireElement(
    container.querySelector<HTMLButtonElement>(
      'button[aria-label="More actions"]',
    ),
  )
  await act(async () => {
    trigger.click()
    await new Promise((resolve) => window.setTimeout(resolve, 0))
  })
}

function mobileActionMenuItem(label: string) {
  return requireElement(
    Array.from(
      document.body.querySelectorAll<HTMLElement>('[role="menuitem"]'),
    ).find((item) => item.textContent === label) ?? null,
  )
}

function mobileCopyStatus() {
  const status = container.querySelector<HTMLElement>(
    '.mobile-conversation-copy-status[role="status"]',
  )
  return status === null
    ? null
    : {
        text: status.textContent,
        hidden: status.classList.contains('sr-only'),
      }
}

it('toggles Starred from the mobile actions menu', async () => {
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

  await openMobileActionsMenu()
  const toggle = mobileActionMenuItem('Add to Starred')
  await act(async () => {
    toggle.click()
    await new Promise((resolve) => window.setTimeout(resolve, 0))
  })

  const snapshot = () => ({
    menuItem: toggle.textContent,
    calls: onToggleToday.mock.calls,
    menuClosed:
      container
        .querySelector('button[aria-label="More actions"]')
        ?.getAttribute('aria-expanded') === 'false',
  })
  expect(snapshot()).toEqual({
    menuItem: 'Add to Starred',
    calls: [[true]],
    menuClosed: true,
  })
})

it('labels a Starred mobile session for removal', async () => {
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

  await openMobileActionsMenu()

  const snapshot = () => ({
    menuItem: mobileActionMenuItem('Remove from Starred').textContent,
    calls: onToggleToday.mock.calls,
    starBeforeTitle:
      container.querySelector('.mobile-conversation-starred') !== null,
  })
  expect(snapshot()).toEqual({
    menuItem: 'Remove from Starred',
    calls: [],
    starBeforeTitle: true,
  })
})

it('places the session ID copy action in the mobile menu', async () => {
  renderView(() => Promise.resolve(true), session, undefined, [], actions, true)

  const titleRow = container.querySelector(
    '.mobile-conversation-title .session-heading-title-row',
  )
  await openMobileActionsMenu()
  const copyMenuItem = mobileActionMenuItem('Copy session ID')

  const snapshot = () => ({
    title: titleRow?.querySelector('h1')?.textContent,
    copyAction: copyMenuItem.textContent,
    copyControlBesideTitle:
      titleRow?.querySelector('.conversation-copy-session-id') !== null,
  })

  expect(snapshot()).toEqual({
    title: 'Choose a retry policy',
    copyAction: 'Copy session ID',
    copyControlBesideTitle: false,
  })
})

it.each([
  { description: 'desktop', isMobile: false },
  { description: 'mobile', isMobile: true },
])(
  'opens the Tasks tab from the header count on $description',
  async ({ isMobile }) => {
    const primaryTask: SessionTask = {
      id: 'fixture-primary-task',
      number: 2048,
      title: 'Improve request reliability',
      priority: 1,
    }
    const secondaryTask: SessionTask = {
      id: 'fixture-secondary-task',
      number: 2046,
      title: 'Add request tracing',
      priority: 4,
    }
    const backgroundTask: BackgroundTask = {
      id: 'fixture-background-task',
      description: 'Review request reliability',
      elapsedSeconds: 12,
      kind: 'subagent',
    }
    const sessionWithLinkedTasks: Session = {
      ...session,
      task: primaryTask,
      tasks: [primaryTask, secondaryTask],
    }
    vi.stubGlobal(
      'fetch',
      vi.fn<typeof fetch>((input) => {
        const url = requestUrl(input)
        if (url.endsWith('/messages?limit=100')) {
          return Promise.resolve(
            jsonResponse(messagesResponse([], [backgroundTask])),
          )
        }
        if (url.endsWith('/actions')) {
          return Promise.resolve(jsonResponse(actions))
        }
        return Promise.resolve(jsonResponse({}))
      }),
    )
    function TaskPanelHarness() {
      const [statusPanelOpen, setStatusPanelOpen] = useState(false)
      return (
        <ConversationPane
          session={sessionWithLinkedTasks}
          shouldSkipSessionTreeFetch
          initialSessionTree={[]}
          refreshKey={0}
          sessionTreeRefreshKey={0}
          onSelectSession={() => undefined}
          isMobile={isMobile}
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
    act(() => root?.render(<TaskPanelHarness />))
    const headerCount = await waitForState(
      () =>
        container.querySelector<HTMLButtonElement>('.session-task-links') !==
        null,
    )
    if (!headerCount) throw new Error('Expected the linked-task count')
    await act(async () => {
      for (let index = 0; index < 12; index += 1) await Promise.resolve()
    })
    await act(async () => {
      requireElement(
        container.querySelector<HTMLButtonElement>('.session-task-links'),
      ).click()
      await Promise.resolve()
    })

    const snapshot = () => ({
      count: container.querySelector('.session-task-links')?.textContent,
      statusPanel: container.querySelector('.session-status-panel') !== null,
      activeTab: container.querySelector('[role="tab"][aria-selected="true"]')
        ?.textContent,
      taskCount: container.querySelectorAll('.session-tasks-panel-list li')
        .length,
      headerPopover: document.body.querySelector('.session-task-links-menu'),
    })
    expect(snapshot()).toEqual({
      count: '+1',
      statusPanel: true,
      activeTab: 'Tasks2',
      taskCount: 2,
      headerPopover: null,
    })
  },
)

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

function mockConversationMessageBounds(
  scrollContainer: HTMLElement,
  messages: readonly HTMLElement[],
  bounds: readonly { top: number; height: number }[],
) {
  vi.spyOn(scrollContainer, 'getBoundingClientRect').mockReturnValue(
    DOMRect.fromRect({ x: 0, y: 100, width: 400, height: 300 }),
  )
  vi.spyOn(scrollContainer, 'clientHeight', 'get').mockReturnValue(300)
  messages.forEach((message, index) => {
    const position = bounds[index]
    if (position === undefined) return
    vi.spyOn(message, 'getBoundingClientRect').mockReturnValue(
      DOMRect.fromRect({
        x: 0,
        y: position.top,
        width: 300,
        height: position.height,
      }),
    )
  })
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

it('provides session links to cards through the conversation pane context', async () => {
  const linkedSession: Session = {
    ...session,
    id: '11111111-1111-4111-8111-111111111111',
    category: 'done',
    title: 'Linked session',
  }
  const todo: ChatItem = {
    type: 'todo',
    seq: 1,
    title: 'Review the session',
    detail: `Open ${linkedSession.id} for context.`,
    completed: false,
    resolution: null,
    comment: null,
  }
  const fetcher = vi.fn<typeof fetch>((input) => {
    const url = requestUrl(input)
    if (url.endsWith('/messages?limit=100')) {
      return Promise.resolve(jsonResponse(messagesResponse([todo])))
    }
    if (url.endsWith('/actions')) {
      return Promise.resolve(jsonResponse(actions))
    }
    return Promise.resolve(jsonResponse({}))
  })
  const onSelectSession = vi.fn<(id: string) => void>()
  vi.stubGlobal('fetch', fetcher)
  root = createRoot(container)
  act(() => {
    root?.render(
      <ConversationPane
        session={session}
        sessions={[linkedSession]}
        refreshKey={0}
        sessionTreeRefreshKey={0}
        onSelectSession={onSelectSession}
      />,
    )
  })

  await waitForState(
    () => container.querySelector('.chat-todo-detail a') !== null,
  )
  const link = requireElement(
    container.querySelector<HTMLAnchorElement>('.chat-todo-detail a'),
  )
  act(() => {
    link.click()
  })

  const getActual = () => ({
    href: link.getAttribute('href'),
    selectedSessions: onSelectSession.mock.calls.map(([id]) => id),
  })
  expect(getActual()).toEqual({
    href: `?session=${linkedSession.id}`,
    selectedSessions: [linkedSession.id],
  })
})

it('posts a message when randomUUID is unavailable', async () => {
  const messagesRequestPaths: string[] = []
  vi.stubGlobal('crypto', {
    getRandomValues: (bytes: Uint8Array) => bytes.fill(0),
  })
  const fetcher = vi.fn<typeof fetch>((input, init) => {
    const url = requestUrl(input)
    if (url.includes('/messages?')) {
      messagesRequestPaths.push(url)
      return Promise.resolve(jsonResponse(messagesResponse([])))
    }
    if (url.endsWith('/messages?from=0')) {
      return Promise.resolve(jsonResponse(messagesResponse([])))
    }
    if (url.endsWith('/actions')) {
      return Promise.resolve(jsonResponse(actions))
    }
    if (url.endsWith('/messages') && init?.method === 'POST') {
      return Promise.resolve(
        jsonResponse({
          queuedPromptId: '00000000-0000-4000-8000-000000000000',
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
        session={session}
        refreshKey={0}
        sessionTreeRefreshKey={0}
        onSelectSession={() => undefined}
      />,
    )
  })

  const composerReady = await waitForState(
    () =>
      container.querySelector<HTMLTextAreaElement>('textarea')?.disabled ===
      false,
  )
  const textarea = requireElement(
    container.querySelector<HTMLTextAreaElement>('textarea'),
  )
  setTextareaValue(textarea, 'Send without randomUUID')
  act(() => {
    dispatchKey(textarea, 'Enter')
  })
  const requestWasSent = await waitForState(() =>
    fetcher.mock.calls.some(
      ([input, init]) =>
        requestUrl(input).endsWith('/messages') && init?.method === 'POST',
    ),
  )
  const confirmationRequestWasSent = await waitForState(() =>
    messagesRequestPaths.some((url) => url.includes('pendingPromptIds=')),
  )
  const getRequestPaths = messagesRequestPaths.slice()
  const postRequest = fetcher.mock.calls
    .filter(
      ([input, init]) =>
        requestUrl(input).endsWith('/messages') && init?.method === 'POST',
    )
    .map(([input, init]) => ({
      url: requestUrl(input),
      body: typeof init?.body === 'string' ? init.body : null,
    }))
  const snapshot = () => ({
    composerReady,
    requestWasSent,
    confirmationRequestWasSent,
    getRequestPaths,
    postRequest,
    alert: container.querySelector('[role="alert"]')?.textContent ?? null,
  })
  expect(snapshot()).toEqual({
    composerReady: true,
    requestWasSent: true,
    confirmationRequestWasSent: true,
    getRequestPaths: [
      '/api/sessions/sample-session/messages?limit=100',
      '/api/sessions/sample-session/messages?from=0&pendingPromptIds=00000000-0000-4000-8000-000000000000',
    ],
    postRequest: [
      {
        url: '/api/sessions/sample-session/messages',
        body: JSON.stringify({
          content: 'Send without randomUUID',
          messageId: '00000000-0000-4000-8000-000000000000',
        }),
      },
    ],
    alert: null,
  })
})

it.each([
  {
    name: 'success',
    response: { exitCode: 0, succeeded: true, timedOut: false },
    buttonLabel: 'Succeeded',
    toast: 'Ran commandprintf safe',
    conversationLine: 'Ran command · <time>',
  },
  {
    name: 'nonzero exit',
    response: { exitCode: 1, succeeded: false, timedOut: false },
    buttonLabel: 'Failed',
    toast: 'Command failed · exit 1printf safeShow output',
    conversationLine: 'Command failed (exit 1) · <time>',
  },
  {
    name: 'timeout',
    response: { exitCode: 124, succeeded: false, timedOut: true },
    buttonLabel: 'Failed',
    toast: 'Command timed outprintf safeStopped after 30 s.Show output',
    conversationLine: 'Command timed out · <time>',
  },
])(
  'executes a bang command from a todo card and reports $name',
  async ({ response, buttonLabel, toast, conversationLine }) => {
    const markdown = 'Check `! printf safe`.'
    const todo: ChatItem = {
      type: 'todo',
      seq: 1,
      title: 'Review the command result',
      detail: markdown,
      completed: false,
      resolution: null,
      comment: null,
    }
    const commandRequests: Array<{ url: string; body: unknown }> = []
    let messageFetchCount = 0
    let commandOutputAvailable = false
    const commandResult: ChatItem = {
      type: 'user',
      timestamp: '2026-06-01T12:00:01Z',
      text: `${response.timedOut ? 'Command timed out after 30 seconds' : `Command exited with code ${String(response.exitCode)}`} in /workspace/example.

Command:
\`\`\`text
$ printf safe
\`\`\`

stdout:
\`\`\`text
(empty)
\`\`\`

stderr:
\`\`\`text
(empty)
\`\`\``,
    }
    let finishExecute: (response: Response) => void = () => {
      throw new Error('Command execution did not start')
    }
    const fetcher = vi.fn<typeof fetch>((input, init) => {
      const url = requestUrl(input)
      if (url.includes('/messages?')) {
        messageFetchCount += 1
        return Promise.resolve(
          jsonResponse(
            messagesResponse(
              commandOutputAvailable ? [todo, commandResult] : [todo],
            ),
          ),
        )
      }
      if (url.endsWith('/actions')) {
        return Promise.resolve(jsonResponse(actions))
      }
      if (url.endsWith('/commands/preview')) {
        const body = typeof init?.body === 'string' ? init.body : '{}'
        commandRequests.push({
          url,
          body: JSON.parse(body),
        })
        return Promise.resolve(jsonResponse({ cwd: '/workspace/example' }))
      }
      if (url.endsWith('/commands/execute')) {
        const body = typeof init?.body === 'string' ? init.body : '{}'
        commandRequests.push({
          url,
          body: JSON.parse(body),
        })
        return new Promise((resolve) => {
          finishExecute = resolve
        })
      }
      return Promise.resolve(jsonResponse({}))
    })
    vi.stubGlobal('fetch', fetcher)
    root = createRoot(container)
    act(() => {
      root?.render(
        <AsyncActionFeedbackProvider>
          <ConversationPane
            session={session}
            refreshKey={0}
            sessionTreeRefreshKey={0}
            onSelectSession={() => undefined}
          />
        </AsyncActionFeedbackProvider>,
      )
    })

    const runButton = requireElement(
      (await waitForState(() =>
        Boolean(container.querySelector('.chat-todo-detail button')),
      ))
        ? container.querySelector<HTMLButtonElement>('.chat-todo-detail button')
        : null,
    )
    const messageFetchCountBeforeCommand = messageFetchCount
    const conversationTimelineOrder = () =>
      Array.from(
        container.querySelectorAll(
          '.conversation-scroll .chat-todo-card, .conversation-scroll .async-action-item--line, .conversation-scroll .chat-user-group[data-command-result="true"]',
        ),
        (element) =>
          element.matches('.chat-todo-card')
            ? 'todo'
            : element.matches('.async-action-item--line')
              ? 'command'
              : 'output',
      )
    await act(async () => userEvent.click(runButton))
    const dialog = requireElement(
      (await waitForState(() =>
        Boolean(document.body.querySelector('[role="dialog"]')),
      ))
        ? document.body.querySelector<HTMLElement>('[role="dialog"]')
        : null,
    )
    const confirmButton = Array.from(dialog.querySelectorAll('button')).find(
      (button) => button.textContent === 'Run command',
    )
    if (!(confirmButton instanceof HTMLButtonElement)) {
      throw new Error('Expected the confirmation button')
    }
    await act(async () => userEvent.click(confirmButton))
    const commandStarted = await waitForState(
      () =>
        document.body.querySelector('[role="dialog"]') === null &&
        Boolean(document.body.querySelector('.async-action-item--running')),
    )
    const runningState = {
      dialogOpen: document.body.querySelector('[role="dialog"]') !== null,
      toast: document.body.querySelector('.async-action-toast-stack')
        ?.textContent,
      timelineOrder: conversationTimelineOrder(),
      conversationLine: normalizeAsyncActionLineTime(
        container.querySelector('.async-action-item--line')?.textContent,
      ),
    }
    await act(async () => {
      commandOutputAvailable = true
      finishExecute(jsonResponse(response))
      await Promise.resolve()
    })
    const completed = await waitForState(
      () =>
        runButton.textContent === buttonLabel &&
        messageFetchCount > messageFetchCountBeforeCommand &&
        document.body.querySelector('.async-action-toast-stack')
          ?.textContent === toast,
    )
    const finalState = {
      button: runButton.textContent,
      toast: document.body.querySelector('.async-action-toast-stack')
        ?.textContent,
      timelineOrder: conversationTimelineOrder(),
      conversationLine: normalizeAsyncActionLineTime(
        container.querySelector('.async-action-item--line')?.textContent,
      ),
      conversationLineButtons: container
        .querySelector('.async-action-item--line')
        ?.querySelectorAll('button').length,
    }
    if (!response.succeeded) {
      const showOutputButton = requireElement(
        Array.from(
          document.body.querySelectorAll<HTMLButtonElement>('button'),
        ).find((button) => button.textContent === 'Show output') ?? null,
      )
      await act(async () => userEvent.click(showOutputButton))
      await waitForState(
        () =>
          container
            .querySelector('.chat-user-group[data-command-result="true"]')
            ?.getAttribute('data-command-output-highlighted') === 'true',
      )
    }
    const outputNavigation = {
      highlighted:
        container
          .querySelector('.chat-user-group[data-command-result="true"]')
          ?.getAttribute('data-command-output-highlighted') ?? null,
      toastVisible:
        document.body.querySelector('.async-action-item--toast') !== null,
    }

    const getActual = () => ({
      commandStarted,
      runningState,
      commandCompleted: completed,
      conversationRefreshed: messageFetchCount > messageFetchCountBeforeCommand,
      finalState,
      outputNavigation,
      commandRequests,
    })
    expect(getActual()).toEqual({
      commandStarted: true,
      runningState: {
        dialogOpen: false,
        toast: 'Running commandprintf safe',
        timelineOrder: ['todo', 'command'],
        conversationLine: 'Running command… · <time>',
      },
      commandCompleted: true,
      conversationRefreshed: true,
      finalState: {
        button: buttonLabel,
        toast,
        timelineOrder: ['todo', 'command', 'output'],
        conversationLine,
        conversationLineButtons: 0,
      },
      outputNavigation: {
        highlighted: response.succeeded ? null : 'true',
        toastVisible: response.succeeded,
      },
      commandRequests: [
        {
          url: '/api/sessions/sample-session/commands/preview',
          body: { messageText: markdown, commands: ['printf safe'] },
        },
        {
          url: '/api/sessions/sample-session/commands/execute',
          body: { messageText: markdown, commands: ['printf safe'] },
        },
      ],
    })
  },
)

it('shows the selected command result instead of a later identical result', async () => {
  const commandResult = (second: string): ChatItem => ({
    type: 'user',
    timestamp: `2026-06-01T12:00:${second}Z`,
    text: `Command exited with code 1 in /workspace/example.

Command:
\`\`\`text
$ printf 'sample output'
\`\`\`

stdout:
\`\`\`text
(empty)
\`\`\`

stderr:
\`\`\`text
(empty)
\`\`\``,
  })
  const messages = [
    commandResult('01'),
    commandResult('02'),
    commandResult('03'),
  ]
  const scrollIntoView = vi.spyOn(HTMLElement.prototype, 'scrollIntoView')

  function CommandOutputActionHarness() {
    const startAsyncAction = useAsyncActionStarter(() => undefined)
    const feedback = useOptionalAsyncActionFeedback()
    return (
      <>
        <button
          type="button"
          onClick={() => {
            startAsyncAction({
              kind: 'command',
              sessionId: session.id,
              sessionTitle: session.title,
              command: "printf 'sample output'",
              run: () =>
                Promise.resolve({
                  status: 'commandFailed',
                  exitCode: 1,
                  commandOutputIndex: 12,
                }),
            })
          }}
        >
          Create failed command result
        </button>
        <output
          data-command-output-count={String(
            feedback?.pendingCommandOutputActionIds.length ?? 0,
          )}
        />
        <ConversationPaneView
          session={session}
          messages={messages}
          start={11}
          actions={actions}
          loading={false}
          error={null}
          onAction={() => Promise.resolve(true)}
          onSelectSession={() => undefined}
        />
      </>
    )
  }

  root = createRoot(container)
  act(() => {
    root?.render(
      <AsyncActionFeedbackProvider>
        <CommandOutputActionHarness />
      </AsyncActionFeedbackProvider>,
    )
  })

  const createActionButton = requireElement(
    container.querySelector<HTMLButtonElement>('button'),
  )
  await act(async () => userEvent.click(createActionButton))
  const showOutputButton = requireElement(
    (await waitForState(() =>
      Array.from(document.body.querySelectorAll('button')).some(
        (button) => button.textContent === 'Show output',
      ),
    ))
      ? (Array.from(
          document.body.querySelectorAll<HTMLButtonElement>('button'),
        ).find((button) => button.textContent === 'Show output') ?? null)
      : null,
  )
  await act(async () => userEvent.click(showOutputButton))
  await waitForState(
    () =>
      container
        .querySelector('.chat-user-group[data-conversation-index="12"]')
        ?.getAttribute('data-command-output-highlighted') === 'true',
  )

  const results = Array.from(
    container.querySelectorAll<HTMLElement>('[data-command-result="true"]'),
  )
  const snapshot = () => ({
    indices: results.map((result) => result.dataset['conversationIndex']),
    highlighted: results.map(
      (result) => result.dataset['commandOutputHighlighted'] ?? null,
    ),
    pendingOutputCount: container
      .querySelector('output')
      ?.getAttribute('data-command-output-count'),
    scrollTargets: scrollIntoView.mock.contexts.map((target) =>
      target instanceof HTMLElement
        ? target.dataset['conversationIndex']
        : null,
    ),
    lineButtons: container
      .querySelector('.async-action-item--line')
      ?.querySelectorAll('button').length,
  })
  expect(snapshot()).toEqual({
    indices: ['11', '12', '13'],
    highlighted: [null, 'true', null],
    pendingOutputCount: '0',
    scrollTargets: ['12'],
    lineButtons: 0,
  })
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
      vibeMode: { mode: 'disabled' },
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
      vibeMode: { mode: 'disabled' },
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
      container.querySelectorAll('.session-tree-row .session-title'),
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
              vibeMode: { mode: 'disabled' },
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
              vibeMode: { mode: 'disabled' },
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
        '#session-tree-row-sibling-session .session-title',
      )?.textContent === 'Updated sibling session',
  )
  renderInboxPanes(sessions, grandchildSession, sessionRefreshRevisions)
  await waitForState(() => container.querySelector('[role="alert"]') !== null)

  const snapshot = () => ({
    currentRow: container.querySelector('.session-tree-row.is-current')?.id,
    siblingTitle: container.querySelector(
      '#session-tree-row-sibling-session .session-title',
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
  for (const sessionId of renderedSessionIds) {
    clearSessionAttachments(sessionId)
  }
  renderedSessionIds.clear()
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
  await openMobileActionsMenu()
  const mobileCopyItem = mobileActionMenuItem('Copy session ID')
  await act(async () => {
    mobileCopyItem.click()
    await new Promise((resolve) => window.setTimeout(resolve, 0))
  })

  const snapshot = () => ({
    copiedText,
    writeCalls: writeText.mock.calls,
    desktopButtonText,
    mobileLiveStatus: mobileCopyStatus(),
  })
  expect(snapshot()).toEqual({
    copiedText: session.id,
    writeCalls: [[session.id], [session.id]],
    desktopButtonText: 'Copied',
    mobileLiveStatus: { text: 'Session ID copied', hidden: false },
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

  await openMobileActionsMenu()
  const copyMenuItem = mobileActionMenuItem('Copy session ID')
  let copyCallsAtClickReturn = 0
  act(() => {
    copyMenuItem.click()
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
    liveStatus: mobileCopyStatus(),
  })
  expect(snapshot()).toEqual({
    copiedText: session.id,
    copyCalls: [['copy']],
    copyCallsAtClickReturn: 1,
    temporaryTextareaCount: 1,
    liveStatus: { text: 'Session ID copied', hidden: false },
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

  await openMobileActionsMenu()
  const copyMenuItem = mobileActionMenuItem('Copy session ID')
  await act(async () => {
    copyMenuItem.click()
    await new Promise((resolve) => window.setTimeout(resolve, 0))
  })

  const snapshot = () => ({
    clipboardCalls: writeText.mock.calls,
    fallbackCalls: copyCommand.mock.calls,
    copiedText,
    liveStatus: mobileCopyStatus(),
  })
  expect(snapshot()).toEqual({
    clipboardCalls: [[session.id]],
    fallbackCalls: [['copy']],
    copiedText: session.id,
    liveStatus: { text: 'Session ID copied', hidden: false },
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

  await openMobileActionsMenu()
  const copyMenuItem = mobileActionMenuItem('Copy session ID')
  await act(async () => {
    copyMenuItem.click()
    await new Promise((resolve) => window.setTimeout(resolve, 0))
  })

  const snapshot = () => ({
    menuClosed:
      container
        .querySelector('button[aria-label="More actions"]')
        ?.getAttribute('aria-expanded') === 'false',
    liveStatus: mobileCopyStatus(),
  })
  expect(snapshot()).toEqual({
    menuClosed: true,
    liveStatus: { text: 'Could not copy session ID', hidden: false },
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

  await openMobileActionsMenu()
  const copyMenuItem = mobileActionMenuItem('Copy session ID')
  await act(async () => {
    copyMenuItem.click()
    await new Promise((resolve) => window.setTimeout(resolve, 0))
  })

  const snapshot = () => ({
    copyCalls: copyCommand.mock.calls,
    temporaryTextareaCount: document.body.querySelectorAll('textarea').length,
    liveStatus: mobileCopyStatus(),
  })
  expect(snapshot()).toEqual({
    copyCalls: [['copy']],
    temporaryTextareaCount: 1,
    liveStatus: { text: 'Could not copy session ID', hidden: false },
  })
})

it('sends a selected answer with its request ID', async () => {
  const onAction = vi.fn(() => Promise.resolve(true))
  renderView(onAction, session, undefined, [question])

  const choiceRow = Array.from(
    container.querySelectorAll<HTMLElement>('.chat-question-choice-row'),
  ).find(
    (row) =>
      row
        .querySelector('.chat-question-choice-text')
        ?.textContent.includes('Window B') === true,
  )
  const choice = requireElement(
    choiceRow?.querySelector<HTMLButtonElement>('button') ?? null,
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

it('does not show terminal guidance for a cancelled question', () => {
  const onAction = vi.fn(() => Promise.resolve(true))
  const cancelledQuestion: ChatItem = {
    type: 'question',
    cancelled: true,
    questions: [questionPrompt],
  }
  renderView(onAction, session, undefined, [cancelledQuestion])

  const snapshot = () => ({
    unansweredQuestionCount: container.querySelectorAll(
      '.chat-question-card[data-has-unanswered-question="true"]',
    ).length,
    terminalMessage: container.querySelector(
      '.conversation-terminal-fallback span',
    )?.textContent,
    choiceButtons: Array.from(
      container.querySelectorAll<HTMLButtonElement>(
        '.chat-question-choices button',
      ),
      (button) => button.disabled,
    ),
    actionCalls: onAction.mock.calls,
  })
  expect(snapshot()).toEqual({
    unansweredQuestionCount: 0,
    terminalMessage: undefined,
    choiceButtons: [true, true],
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
      card.querySelectorAll<HTMLElement>('.chat-question-choice-row'),
      (row) => {
        const button = requireElement(
          row.querySelector<HTMLButtonElement>('button'),
        )
        return {
          label: row.querySelector('.chat-question-choice-text')?.textContent,
          disabled: button.disabled,
          selected: button.classList.contains('is-selected'),
          dimmed: button.classList.contains('is-unselected'),
          check: row.querySelector('.chat-question-choice-check')?.textContent,
        }
      },
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

it('jumps between own messages with Alt-arrow shortcuts', () => {
  const onAction = vi.fn(() => Promise.resolve(true))
  renderView(onAction, session, undefined, [
    { type: 'user', text: 'An earlier message.' },
    {
      type: 'user_with_content',
      from: 'A linked session',
      content: [{ type: 'text', text: 'A message from another session.' }],
    },
    {
      type: 'queued_user',
      content: [{ type: 'text', text: 'A queued message.' }],
    },
    {
      type: 'user_with_content',
      content: [{ type: 'text', text: 'A later message.' }],
    },
  ])

  const scrollContainer = requireElement(
    container.querySelector<HTMLElement>('.conversation-scroll'),
  )
  const ownMessages = Array.from(
    container.querySelectorAll<HTMLElement>('[data-own-message="true"]'),
  )
  const earlierMessage = requireElement(ownMessages[0] ?? null)
  const laterMessage = requireElement(ownMessages[2] ?? null)
  const scrollIntoView = vi
    .spyOn(HTMLElement.prototype, 'scrollIntoView')
    .mockImplementation(() => undefined)
  mockConversationMessageBounds(scrollContainer, ownMessages, [
    { top: 40, height: 30 },
    { top: 120, height: 30 },
    { top: 500, height: 30 },
  ])

  const previousEvent = dispatchKeyInAct(window, 'ArrowUp', { altKey: true })
  const nextEvent = dispatchKeyInAct(window, 'ArrowDown', { altKey: true })

  const actual = () => ({
    ownMessageTexts: ownMessages.map(
      (message) => message.querySelector('.markdown-message')?.textContent,
    ),
    scrollTargets: scrollIntoView.mock.contexts,
    scrollOptions: scrollIntoView.mock.calls.map(([options]) => options),
    highlightedMessages: ownMessages.map(
      (message) =>
        message.getAttribute('data-own-message-highlighted') === 'true',
    ),
    defaultPrevented: [
      previousEvent.defaultPrevented,
      nextEvent.defaultPrevented,
    ],
  })
  expect(actual()).toEqual({
    ownMessageTexts: [
      'An earlier message.',
      'A queued message.',
      'A later message.',
    ],
    scrollTargets: [earlierMessage, laterMessage],
    scrollOptions: [
      { block: 'center', behavior: 'instant' },
      { block: 'center', behavior: 'instant' },
    ],
    highlightedMessages: [false, false, true],
    defaultPrevented: [true, true],
  })
})

it('removes the navigation highlight after one and a half seconds', async () => {
  vi.useFakeTimers()
  const onAction = vi.fn(() => Promise.resolve(true))
  renderView(onAction, session, undefined, [
    { type: 'user', text: 'An earlier message.' },
    { type: 'user', text: 'A later message.' },
  ])

  const scrollContainer = requireElement(
    container.querySelector<HTMLElement>('.conversation-scroll'),
  )
  const ownMessages = Array.from(
    container.querySelectorAll<HTMLElement>('[data-own-message="true"]'),
  )
  const laterMessage = requireElement(ownMessages[1] ?? null)
  vi.spyOn(HTMLElement.prototype, 'scrollIntoView').mockImplementation(
    () => undefined,
  )
  mockConversationMessageBounds(scrollContainer, ownMessages, [
    { top: 40, height: 30 },
    { top: 500, height: 30 },
  ])

  dispatchKeyInAct(window, 'ArrowDown', { altKey: true })
  const highlightedBeforeTimeout =
    laterMessage.getAttribute('data-own-message-highlighted') === 'true'
  await act(async () => {
    await vi.advanceTimersByTimeAsync(1_500)
  })

  const snapshot = () => ({
    highlightedBeforeTimeout,
    highlightedAfterTimeout:
      laterMessage.getAttribute('data-own-message-highlighted') === 'true',
  })
  expect(snapshot()).toEqual({
    highlightedBeforeTimeout: true,
    highlightedAfterTimeout: false,
  })
})

it('keeps the latest navigation highlight when an earlier timer expires', async () => {
  vi.useFakeTimers()
  const onAction = vi.fn(() => Promise.resolve(true))
  renderView(onAction, session, undefined, [
    { type: 'user', text: 'An earlier message.' },
    { type: 'user', text: 'A middle message.' },
    { type: 'user', text: 'A later message.' },
  ])

  const scrollContainer = requireElement(
    container.querySelector<HTMLElement>('.conversation-scroll'),
  )
  const ownMessages = Array.from(
    container.querySelectorAll<HTMLElement>('[data-own-message="true"]'),
  )
  vi.spyOn(HTMLElement.prototype, 'scrollIntoView').mockImplementation(
    () => undefined,
  )
  const messageBounds = ownMessages.map((message) =>
    vi.spyOn(message, 'getBoundingClientRect'),
  )
  vi.spyOn(scrollContainer, 'getBoundingClientRect').mockReturnValue(
    DOMRect.fromRect({ x: 0, y: 100, width: 400, height: 300 }),
  )
  vi.spyOn(scrollContainer, 'clientHeight', 'get').mockReturnValue(300)
  const setMessageBounds = (tops: readonly number[]) => {
    messageBounds.forEach((bounds, index) => {
      bounds.mockReturnValue(
        DOMRect.fromRect({
          x: 0,
          y: tops[index] ?? 0,
          width: 300,
          height: 30,
        }),
      )
    })
  }

  setMessageBounds([40, 200, 500])
  dispatchKeyInAct(window, 'ArrowUp', { altKey: true })
  await act(async () => {
    await vi.advanceTimersByTimeAsync(1_000)
  })
  dispatchKeyInAct(window, 'ArrowDown', { altKey: true })
  await act(async () => {
    await vi.advanceTimersByTimeAsync(500)
  })
  const highlightedAfterEarlierDeadline = ownMessages.map(
    (message) =>
      message.getAttribute('data-own-message-highlighted') === 'true',
  )

  setMessageBounds([40, 500, 200])
  dispatchKeyInAct(window, 'ArrowUp', { altKey: true })

  const snapshot = () => ({
    highlightedAfterEarlierDeadline,
    highlightedAfterNextNavigation: ownMessages.map(
      (message) =>
        message.getAttribute('data-own-message-highlighted') === 'true',
    ),
  })
  expect(snapshot()).toEqual({
    highlightedAfterEarlierDeadline: [false, false, true],
    highlightedAfterNextNavigation: [false, true, false],
  })
})

it('starts with the first own message when the viewport is above the conversation', () => {
  const onAction = vi.fn(() => Promise.resolve(true))
  renderView(onAction, session, undefined, [
    { type: 'user', text: 'An earlier message.' },
    {
      type: 'queued_user',
      content: [{ type: 'text', text: 'A queued message.' }],
    },
  ])

  const scrollContainer = requireElement(
    container.querySelector<HTMLElement>('.conversation-scroll'),
  )
  const ownMessages = Array.from(
    container.querySelectorAll<HTMLElement>('[data-own-message="true"]'),
  )
  const firstMessage = requireElement(ownMessages[0] ?? null)
  const scrollIntoView = vi
    .spyOn(HTMLElement.prototype, 'scrollIntoView')
    .mockImplementation(() => undefined)
  mockConversationMessageBounds(scrollContainer, ownMessages, [
    { top: 500, height: 30 },
    { top: 600, height: 30 },
  ])

  const previousEvent = dispatchKeyInAct(window, 'ArrowUp', { altKey: true })
  const nextEvent = dispatchKeyInAct(window, 'ArrowDown', { altKey: true })

  const actual = () => ({
    scrollTargets: scrollIntoView.mock.contexts,
    defaultPrevented: [
      previousEvent.defaultPrevented,
      nextEvent.defaultPrevented,
    ],
  })
  expect(actual()).toEqual({
    scrollTargets: [firstMessage],
    defaultPrevented: [false, true],
  })
})

it('leaves populated text fields alone and allows empty or unfocused shortcuts', () => {
  const onAction = vi.fn(() => Promise.resolve(true))
  renderView(onAction, session, undefined, [
    { type: 'user', text: 'An earlier message.' },
    {
      type: 'queued_user',
      content: [{ type: 'text', text: 'A queued message.' }],
    },
    {
      type: 'user',
      text: 'A later message.',
    },
  ])

  const scrollContainer = requireElement(
    container.querySelector<HTMLElement>('.conversation-scroll'),
  )
  const ownMessages = Array.from(
    container.querySelectorAll<HTMLElement>('[data-own-message="true"]'),
  )
  const earlierMessage = requireElement(ownMessages[0] ?? null)
  const laterMessage = requireElement(ownMessages[2] ?? null)
  const textarea = requireElement(
    container.querySelector<HTMLTextAreaElement>('textarea'),
  )
  const input = document.createElement('input')
  requireElement(container.querySelector('.conversation-pane')).append(input)
  const scrollIntoView = vi
    .spyOn(HTMLElement.prototype, 'scrollIntoView')
    .mockImplementation(() => undefined)
  mockConversationMessageBounds(scrollContainer, ownMessages, [
    { top: 40, height: 30 },
    { top: 120, height: 30 },
    { top: 500, height: 30 },
  ])

  setTextareaValue(textarea, 'Keep editing this draft')
  input.value = 'Keep editing this field'
  const populatedInputEvent = dispatchKeyInAct(input, 'ArrowDown', {
    altKey: true,
  })
  const populatedEvent = dispatchKeyInAct(textarea, 'ArrowUp', {
    altKey: true,
  })
  setTextareaValue(textarea, '')
  const emptyEvent = dispatchKeyInAct(textarea, 'ArrowUp', { altKey: true })
  const unfocusedEvent = dispatchKeyInAct(window, 'ArrowDown', {
    altKey: true,
  })

  const actual = () => ({
    scrollTargets: scrollIntoView.mock.contexts,
    defaultPrevented: [
      populatedInputEvent.defaultPrevented,
      populatedEvent.defaultPrevented,
      emptyEvent.defaultPrevented,
      unfocusedEvent.defaultPrevented,
    ],
  })
  expect(actual()).toEqual({
    scrollTargets: [earlierMessage, laterMessage],
    defaultPrevented: [false, false, true, true],
  })
})

it('ignores Alt-arrow shortcuts on mobile', () => {
  const onAction = vi.fn(() => Promise.resolve(true))
  renderView(
    onAction,
    session,
    undefined,
    [{ type: 'user', text: 'A message.' }],
    actions,
    true,
  )
  const scrollIntoView = vi
    .spyOn(HTMLElement.prototype, 'scrollIntoView')
    .mockImplementation(() => undefined)

  const previousEvent = dispatchKeyInAct(window, 'ArrowUp', { altKey: true })
  const nextEvent = dispatchKeyInAct(window, 'ArrowDown', { altKey: true })

  const actual = () => ({
    scrollTargets: scrollIntoView.mock.contexts,
    defaultPrevented: [
      previousEvent.defaultPrevented,
      nextEvent.defaultPrevented,
    ],
  })
  expect(actual()).toEqual({
    scrollTargets: [],
    defaultPrevented: [false, false],
  })
})

it('shows desktop newline shortcuts with the applicable interrupt hint', () => {
  const onAction = () => Promise.resolve(true)
  renderView(onAction)
  const interruptibleHint =
    container.querySelector('.composer-key-hint')?.textContent
  const ownMessageHint = container.querySelector(
    '.composer-key-hint2',
  )?.textContent
  renderView(onAction, session, undefined, [], { ...actions, interrupt: false })
  const nonInterruptibleHint =
    container.querySelector('.composer-key-hint')?.textContent

  const snapshot = () => ({
    interruptibleHint,
    nonInterruptibleHint,
    ownMessageHint,
  })
  expect(snapshot()).toEqual({
    interruptibleHint: '↵ Send · ⇧↵ / ⌥↵ New line · Esc Interrupt',
    nonInterruptibleHint: '↵ Send · ⇧↵ / ⌥↵ New line',
    ownMessageHint: '⌥↑ / ⌥↓ My messages',
  })
})

it('keeps Enter for newlines and exposes mobile conversation actions', async () => {
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
    keyboardHint: container.querySelector('.composer-key-hint')?.textContent,
    keyboardHint2:
      container.querySelector('.composer-key-hint2')?.textContent ?? null,
    choiceShortcutBadges: container.querySelectorAll(
      '.chat-question-choices kbd',
    ).length,
    interruptLabel: container
      .querySelector('.mobile-interrupt-button')
      ?.getAttribute('aria-label'),
    interruptBusy: container
      .querySelector('.mobile-interrupt-button')
      ?.getAttribute('aria-busy'),
  })
  const duringInterrupt = snapshot()
  await act(async () => {
    await Promise.resolve()
  })
  expect(duringInterrupt).toEqual({
    actions: [['interrupt']],
    defaultPrevented: [false, false],
    keyboardHint: '',
    keyboardHint2: null,
    choiceShortcutBadges: 0,
    interruptLabel: 'Interrupting…',
    interruptBusy: 'true',
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

it('sends one interrupt while its request is pending', async () => {
  let finishInterrupt: (succeeded: boolean) => void = () => undefined
  const pendingInterrupt = new Promise<boolean>((resolve) => {
    finishInterrupt = resolve
  })
  const onAction = vi.fn(() => pendingInterrupt)
  renderView(onAction, session, undefined, [], actions, true)
  const textarea = requireElement(
    container.querySelector<HTMLTextAreaElement>('textarea'),
  )
  dispatchKeyInAct(textarea, 'Escape')
  dispatchKeyInAct(textarea, 'Escape')
  const button = requireElement(
    container.querySelector<HTMLButtonElement>('.mobile-interrupt-button'),
  )
  act(() => {
    button.click()
  })
  const during = {
    actions: onAction.mock.calls,
    interruptDisabled: button.disabled,
  }

  await act(async () => {
    finishInterrupt(true)
    await pendingInterrupt
  })

  const snapshot = () => ({
    during,
    after: {
      actions: onAction.mock.calls,
      interruptDisabled: button.disabled,
    },
  })
  const expected = {
    during: { actions: [['interrupt']], interruptDisabled: true },
    after: { actions: [['interrupt']], interruptDisabled: false },
  }
  expect(snapshot()).toEqual(expected)
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

it('renders vibe mode changes as centered events with a time separator', () => {
  const offTimestamp = '2026-06-01T12:10:00Z'
  const onTimestamp = '2026-06-01T12:20:00Z'
  const events = [
    {
      type: 'event' as const,
      kind: 'armyknife' as const,
      text: 'Vibe mode turned off',
      failed: false,
      timestamp: offTimestamp,
    },
    {
      type: 'event' as const,
      kind: 'armyknife' as const,
      text: 'Vibe mode turned on',
      failed: false,
      timestamp: onTimestamp,
    },
  ]
  renderView(() => Promise.resolve(true), session, undefined, events)

  const actual = Array.from(
    container.querySelectorAll<HTMLElement>('.chat-event'),
  ).map((event) => ({
    text: event.querySelector('.chat-event-text')?.textContent ?? null,
    separator:
      event.querySelector('.chat-event-separator')?.textContent ?? null,
    time: event.querySelector('time')?.textContent ?? null,
    kind: event.getAttribute('data-kind'),
    alignSelf: getComputedStyle(event).alignSelf,
  }))
  expect(actual).toEqual([
    {
      text: 'Vibe mode turned off',
      separator: '·',
      time: formatMessageTime(offTimestamp),
      kind: 'armyknife',
      alignSelf: 'center',
    },
    {
      text: 'Vibe mode turned on',
      separator: '·',
      time: formatMessageTime(onTimestamp),
      kind: 'armyknife',
      alignSelf: 'center',
    },
  ])
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
        text: 'TOSENTBuild verificationPlease check the fictional output.画像はもうありません',
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
  const fetcher = vi.fn<typeof fetch>((input) => {
    const response = requestUrl(input).endsWith('/actions')
      ? actions
      : messagesResponse([
          {
            type: 'user_with_content',
            content: [
              { type: 'text', text: 'Inspect this document.' },
              { type: 'file', name: 'sample-layout.html' },
            ],
          },
        ])
    return Promise.resolve(jsonResponse(response))
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
  vi.spyOn(URL, 'createObjectURL').mockReturnValue('blob:proposal-action-image')
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

  const comment = requireElement(
    container.querySelector<HTMLTextAreaElement>('.chat-proposal-comment'),
  )
  const paste = new Event('paste', { bubbles: true, cancelable: true })
  Object.defineProperty(paste, 'clipboardData', {
    value: {
      files: [new File(['image'], 'screen.png', { type: 'image/png' })],
    },
  })
  await act(async () => {
    comment.dispatchEvent(paste)
    await new Promise((resolve) => window.setTimeout(resolve, 10))
  })
  const chooseButton = requireElement(
    container.querySelector<HTMLButtonElement>(
      '.chat-proposal-actions .aw-button--primary',
    ),
  )
  await waitForState(
    () => container.querySelector('.attachment-preview-item img') !== null,
  )

  act(() => {
    requireElement(
      container.querySelectorAll<HTMLInputElement>(
        '.chat-proposal-option input',
      )[1] ?? null,
    ).click()
  })
  await waitForState(() => !chooseButton.disabled)
  act(() => {
    chooseButton.click()
  })
  await waitForState(() =>
    requests.some((request) => request.method === 'POST'),
  )

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
      body: '{"choice":1,"comment":null,"expectedBody":{"title":"When should the setting take effect?","summary":"Choose how the change is introduced.","options":[{"label":"Use the existing flow","description":"Explain the change before applying it.","pros":["Small change"],"cons":["Adds one step"],"recommended":true},{"label":"Show the choice earlier","description":null,"pros":[],"cons":[],"recommended":false}]},"attachments":[{"name":"screen.png","mimeType":"image/png","size":5,"data":"aW1hZ2U="}]}',
    },
    placeholder: 'Message session',
    status: 'Proposal · chose Show the choice earlier',
  })
})

it('posts a todo rejection to its resolution route with the comment', async () => {
  vi.spyOn(URL, 'createObjectURL').mockReturnValue('blob:todo-action-image')
  const todo: ChatItem = {
    type: 'todo',
    seq: 4,
    title: 'Check the page on a phone',
    detail: 'Confirm the action remains visible.',
    completed: false,
    resolution: null,
    comment: null,
  }
  const rejectedTodo: ChatItem = {
    ...todo,
    completed: true,
    resolution: 'rejected',
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
          ? jsonResponse(messagesResponse([completed ? rejectedTodo : todo]))
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
  const comment = requireElement(
    container.querySelector<HTMLTextAreaElement>('.chat-todo-comment'),
  )
  const paste = new Event('paste', { bubbles: true, cancelable: true })
  Object.defineProperty(paste, 'clipboardData', {
    value: {
      files: [new File(['image'], 'screen.png', { type: 'image/png' })],
    },
  })
  await act(async () => {
    comment.dispatchEvent(paste)
    await new Promise((resolve) => window.setTimeout(resolve, 10))
  })
  await waitForState(
    () => container.querySelector('.attachment-preview-item img') !== null,
  )
  act(() => {
    requireElement(
      container
        .querySelectorAll<HTMLButtonElement>('.chat-todo-footer button')
        .item(1),
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
      url: '/api/sessions/sample-session/todos/4/resolve',
      method: 'POST',
      body: '{"resolution":"rejected","comment":"The page is readable.","expectedTitle":"Check the page on a phone","expectedDetail":"Confirm the action remains visible.","attachments":[{"name":"screen.png","mimeType":"image/png","size":5,"data":"aW1hZ2U="}]}',
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
    workingStatus: container.querySelector('.conversation-status-band-label')
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
    status: container.querySelector('.conversation-status-band-label')
      ?.textContent,
    detail: container.querySelector('.conversation-status-band-inline-detail')
      ?.textContent,
    waitingHourglassVisible:
      container.querySelector(
        '.conversation-status-band--waiting .conversation-status-band-hourglass',
      ) !== null,
  })

  expect(snapshot()).toEqual({
    status: 'Waiting',
    detail: 'waiting for an external build',
    waitingHourglassVisible: true,
  })
})

it.each([
  {
    kind: 'question',
    messages: [
      question,
      {
        ...question,
        requestId: 'ask-request-answered',
        questions: [{ ...questionPrompt, answer: 'Window A' }],
      },
      {
        ...question,
        requestId: 'ask-request-cancelled',
        cancelled: true,
        questions: [{ ...questionPrompt, answer: null }],
      },
    ],
    selector: '.chat-question-card[data-has-unanswered-question="true"]',
  },
  {
    kind: 'permission',
    messages: [permission, permissionDenied],
    selector: '.chat-permission-card:not(.chat-permission-denied-card)',
  },
])(
  'scrolls to the pending $kind card from its status row',
  async ({ kind, messages, selector }) => {
    renderView(
      vi.fn().mockResolvedValue(true),
      {
        ...session,
        category: 'needs_action',
        kind: kind === 'question' ? 'Question' : 'Approve',
      },
      undefined,
      messages,
    )
    const card = requireElement(container.querySelector<HTMLElement>(selector))
    const scrollIntoView = vi
      .spyOn(HTMLElement.prototype, 'scrollIntoView')
      .mockImplementation(() => undefined)
    const toggle = requireElement(
      container.querySelector<HTMLButtonElement>(
        '.conversation-status-band-toggle',
      ),
    )

    await act(async () => userEvent.click(toggle))

    const snapshot = () => ({
      scrollTargets: scrollIntoView.mock.contexts,
      scrollOptions: scrollIntoView.mock.calls.map(([options]) => options),
      arrow: toggle.querySelector('.conversation-status-band-chevron')
        ?.textContent,
    })
    expect(snapshot()).toEqual({
      scrollTargets: [card],
      scrollOptions: [{ behavior: 'smooth', block: 'center' }],
      arrow: '↑',
    })
  },
)

it('shows elapsed time for a waiting background task', () => {
  renderView(
    vi.fn().mockResolvedValue(true),
    {
      ...session,
      category: 'in_progress',
      activityStatus: 'stopped',
      waitingForBackgroundTasks: true,
    },
    undefined,
    [],
    actions,
    false,
    [
      {
        id: 'fixture-background-task',
        description: 'Check the sample build',
        elapsedSeconds: 42,
        kind: 'background_task',
      },
    ],
  )

  const snapshot = () => ({
    status: container.querySelector('.conversation-status-band-label')
      ?.textContent,
    detail: container.querySelector('.conversation-status-band-inline-detail')
      ?.textContent,
    waitingDotVisible:
      container.querySelector(
        '.conversation-status-band--background-tasks .conversation-status-band-dot',
      ) !== null,
  })

  expect(snapshot()).toEqual({
    status: 'Background tasks (1)',
    detail: 'Check the sample build · 42s',
    waitingDotVisible: true,
  })
})

it('shows background task descriptions and elapsed times on mobile', () => {
  renderView(
    vi.fn().mockResolvedValue(true),
    {
      ...session,
      category: 'in_progress',
      activityStatus: 'stopped',
      waitingForBackgroundTasks: true,
      elapsedSeconds: 92,
    },
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

  const toggle = requireElement(
    container.querySelector<HTMLButtonElement>(
      '.conversation-status-band-toggle',
    ),
  )
  const band = requireElement(
    container.querySelector<HTMLElement>('.conversation-status-band'),
  )
  const compactSnapshot = () => ({
    height: Math.round(band.getBoundingClientRect().height),
    padding: getComputedStyle(band).padding,
  })
  expect(compactSnapshot()).toEqual({ height: 44, padding: '0px' })
  act(() => {
    toggle.click()
  })

  const snapshot = () => ({
    status: container.querySelector('.conversation-status-band-label')
      ?.textContent,
    expanded: toggle.getAttribute('aria-expanded'),
    tasks: Array.from(
      container.querySelectorAll('.conversation-status-band-expanded p'),
      (item) => item.textContent,
    ),
  })
  expect(snapshot()).toEqual({
    status: 'Background tasks (2)',
    expanded: 'true',
    tasks: [
      'Watch the build pipeline · 2m 8s',
      'Review the generated report · 43s',
    ],
  })
})

it('shows the compacting status and summary instead of stale tool details', () => {
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

  expect(getCompactingStatusSnapshot()).toEqual({
    workingStatus: 'Compacting…',
    detail: '古い会話を要約しています',
    inProgressItems: 0,
  })
})

it('shows an active tool while a working session is marked as compacting', () => {
  renderView(
    vi.fn().mockResolvedValue(true),
    {
      ...session,
      category: 'in_progress',
      activityStatus: 'working',
      isCompacting: true,
      elapsedSeconds: 92,
    },
    undefined,
    [
      {
        type: 'tool_in_progress',
        tool: 'exec',
        target: 'cargo check --package fixture',
      },
    ],
  )

  expect(getCompactingStatusSnapshot()).toEqual({
    workingStatus: 'Working · 1m 32s',
    detail: 'Bash  cargo check --package fixture',
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
    workingStatus: container.querySelector('.conversation-status-band-label')
      ?.textContent,
    activeTool: container.querySelector(
      '.conversation-status-band-inline-detail',
    )?.textContent,
    toolSummary:
      container.querySelector('.chat-tool-summary')?.textContent ?? null,
    inProgressItems: container.querySelectorAll('.tool-in-progress').length,
  })

  expect(snapshot()).toEqual({
    workingStatus: 'Working · 1m 32s',
    activeTool: 'Bash  cargo test --manifest-path fixture/Cargo.toml',
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
    workingStatus: container.querySelector('.conversation-status-band'),
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

  const toggle = requireElement(
    container.querySelector<HTMLButtonElement>(
      '.conversation-status-band-toggle',
    ),
  )
  const snapshot = () => ({
    status: container.querySelector('.conversation-status-band-label')
      ?.textContent,
    waitingHourglassVisible:
      container.querySelector(
        '.conversation-status-band--waiting .conversation-status-band-hourglass',
      ) !== null,
    backgroundTasks: container.querySelector(
      '.conversation-status-band-inline-detail',
    )?.textContent,
    expanded: toggle.getAttribute('aria-expanded'),
  })

  expect(snapshot()).toEqual({
    status: 'Waiting',
    waitingHourglassVisible: true,
    backgroundTasks: 'Background tasks (1)',
    expanded: 'false',
  })
  act(() => {
    toggle.click()
  })
  expect(
    Array.from(
      container.querySelectorAll('.conversation-status-band-expanded p'),
      (detail) => detail.textContent,
    ),
  ).toEqual(['Watch the build pipeline · 2m 8s'])
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
        message: true,
        answerQuestion: false,
        interrupt: false,
        openInTerminal: false,
        rewind: false,
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
    workingStatus: container.querySelector('.conversation-status-band-label')
      ?.textContent,
    activeTool: container.querySelector(
      '.conversation-status-band-inline-detail',
    )?.textContent,
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
      '.conversation-status-band-tasks summary',
    )?.textContent,
    headerRunningCount: container.querySelector(
      '.conversation-status-panel-count',
    )?.textContent,
  })
  expect(snapshot()).toEqual({
    workingStatus: 'Working · 30s',
    activeTool: 'Bash  cargo test',
    peerMessage:
      'FROMFixture peer session (ended)A fictional peer update.fixture-peer-layout.html',
    peerImage: '/api/images/fixture-peer-image.png',
    peerFile: 'fixture-peer-layout.html',
    event: [{ text: 'A fictional event was blocked.', failed: 'true' }],
    backgroundTasks: undefined,
    headerRunningCount: undefined,
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
    workingStatus: container.querySelector('.conversation-status-band-label')
      ?.textContent,
    backgroundTaskCount: container.querySelector(
      '.conversation-status-band-inline-detail',
    )?.textContent,
    legacyTaskList:
      container.querySelector('.conversation-status-band-tasks') !== null,
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
      '▶cargo test -p fictional-crate2h 0mOutput',
    ],
    panelToggle: '2',
    workingStatus: 'Waiting',
    backgroundTaskCount: 'Background tasks (2)',
    legacyTaskList: false,
  })
})

it('updates running task elapsed time from the loaded conversation timestamp', async () => {
  vi.useFakeTimers()
  vi.setSystemTime(new Date('2026-11-10T00:00:00.000Z'))
  const fetcher = vi.fn<typeof fetch>((input) => {
    const url = requestUrl(input)
    if (url.endsWith('/messages?limit=100')) {
      return Promise.resolve(
        jsonResponse(
          messagesResponse(
            [],
            [
              {
                id: 'fixture-background-task',
                description: 'Build the sample bundle',
                elapsedSeconds: 61,
                kind: 'background_task',
              },
            ],
          ),
        ),
      )
    }
    if (url.endsWith('/actions')) {
      return Promise.resolve(jsonResponse(actions))
    }
    return Promise.resolve(jsonResponse({}))
  })
  vi.stubGlobal('fetch', fetcher)
  root = createRoot(container)
  act(() => {
    root?.render(
      <ConversationPane
        session={{ ...session, category: 'needs_action' }}
        shouldSkipSessionTreeFetch
        initialSessionTree={[]}
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
  const before = container.querySelector('.running-task-elapsed')?.textContent

  await act(async () => {
    await vi.advanceTimersByTimeAsync(1_000)
  })

  const snapshot = () => ({
    before,
    after: container.querySelector('.running-task-elapsed')?.textContent,
  })
  expect(snapshot()).toEqual({ before: '1m 1s', after: '1m 2s' })
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

it('jumps from a mobile Decisions tab to the selected conversation message', async () => {
  const decision: ChatItem = {
    type: 'decision',
    seq: 12,
    title: 'Keep retry limits predictable',
    why: 'A fixed limit prevents repeated delivery attempts.',
    considered: ['Retry until the request succeeds'],
    easyToChange: 'Update one worker setting.',
    review: null,
    createdAt: '2026-11-11T03:25:59Z',
  }
  const fetcher = vi.fn<typeof fetch>((input) => {
    const url = requestUrl(input)
    if (url.endsWith('/messages?limit=100')) {
      return Promise.resolve(jsonResponse(messagesResponse([decision])))
    }
    if (url.endsWith('/actions')) {
      return Promise.resolve(jsonResponse(actions))
    }
    return Promise.resolve(jsonResponse({}))
  })
  vi.stubGlobal('fetch', fetcher)

  function MobileConversationHarness() {
    const [statusPanelOpen, setStatusPanelOpen] = useState(true)
    return (
      <ConversationPane
        session={{ ...session, decisionCount: 1, unreviewedDecisionCount: 1 }}
        shouldSkipSessionTreeFetch
        initialSessionTree={[]}
        refreshKey={0}
        sessionTreeRefreshKey={0}
        onSelectSession={() => undefined}
        isMobile
        statusPanelOpen={statusPanelOpen}
        onCloseStatusPanel={() => {
          setStatusPanelOpen(false)
        }}
      />
    )
  }

  root = createRoot(container)
  act(() => root?.render(<MobileConversationHarness />))
  await waitForState(
    () => container.querySelector('.decision-card--panel') !== null,
  )
  const decisionRow = requireElement(
    container.querySelector<HTMLElement>('#decision-row-12'),
  )
  const scrollIntoView = vi.spyOn(decisionRow, 'scrollIntoView')
  await act(async () => {
    await userEvent.click(
      requireElement(
        container.querySelector<HTMLButtonElement>(
          '.decision-card--panel .decision-panel-card-heading',
        ),
      ),
    )
  })

  const snapshot = () => ({
    statusPanelVisible:
      container.querySelector('.mobile-session-status-overlay') !== null,
    selected: decisionRow.getAttribute('data-selected'),
    scrollCalls: scrollIntoView.mock.calls,
  })
  expect(snapshot()).toEqual({
    statusPanelVisible: false,
    selected: 'true',
    scrollCalls: [[{ block: 'center' }]],
  })
})

it('loads older messages before jumping to a pruned mobile decision', async () => {
  const decision: ChatItem = {
    type: 'decision',
    seq: 12,
    title: 'Keep retry limits predictable',
    why: 'A fixed limit prevents repeated delivery attempts.',
    considered: ['Retry until the request succeeds'],
    easyToChange: 'Update one worker setting.',
    review: null,
    createdAt: '2026-11-11T03:25:59Z',
  }
  const assistantMessage = (index: number): ChatItem => ({
    type: 'assistant',
    text: `Message ${String(index)}.`,
  })
  const latestPage = Array.from({ length: 100 }, (_, index) =>
    assistantMessage(index + 200),
  )
  const middlePage = Array.from({ length: 100 }, (_, index) =>
    assistantMessage(index + 100),
  )
  const oldestPage = Array.from({ length: 100 }, (_, index) =>
    assistantMessage(index),
  )
  oldestPage[99] = decision
  const fetcher = vi.fn<typeof fetch>((input) => {
    const url = requestUrl(input)
    if (url.endsWith('/messages?before=200&limit=100')) {
      return Promise.resolve(
        jsonResponse(
          messagesResponse(middlePage, [], {
            start: 100,
            total: 300,
            decisions: [decision],
          }),
        ),
      )
    }
    if (url.endsWith('/messages?before=100&limit=100')) {
      return Promise.resolve(
        jsonResponse(
          messagesResponse(oldestPage, [], {
            start: 0,
            total: 300,
            decisions: [decision],
          }),
        ),
      )
    }
    if (url.endsWith('/messages?limit=100')) {
      return Promise.resolve(
        jsonResponse(
          messagesResponse(latestPage, [], {
            start: 200,
            total: 300,
            decisions: [decision],
          }),
        ),
      )
    }
    if (url.endsWith('/actions')) {
      return Promise.resolve(jsonResponse(actions))
    }
    return Promise.resolve(jsonResponse({}))
  })
  vi.stubGlobal('fetch', fetcher)

  function MobileConversationHarness() {
    const [statusPanelOpen, setStatusPanelOpen] = useState(true)
    return (
      <ConversationPane
        session={{ ...session, decisionCount: 1, unreviewedDecisionCount: 1 }}
        shouldSkipSessionTreeFetch
        initialSessionTree={[]}
        refreshKey={0}
        sessionTreeRefreshKey={0}
        onSelectSession={() => undefined}
        isMobile
        statusPanelOpen={statusPanelOpen}
        onCloseStatusPanel={() => {
          setStatusPanelOpen(false)
        }}
      />
    )
  }

  root = createRoot(container)
  act(() => root?.render(<MobileConversationHarness />))
  await waitForState(
    () => container.querySelector('.decision-card--panel') !== null,
  )
  await act(async () => {
    await userEvent.click(
      requireElement(
        container.querySelector<HTMLButtonElement>(
          '.decision-card--panel .decision-panel-card-heading',
        ),
      ),
    )
  })
  await waitForState(() => container.querySelector('#decision-row-12') !== null)

  const snapshot = () => ({
    statusPanelVisible:
      container.querySelector('.mobile-session-status-overlay') !== null,
    decisionVisible: container.querySelector('#decision-row-12') !== null,
    olderRequests: fetcher.mock.calls
      .map(([input]) => requestUrl(input))
      .filter((url) => url.includes('/messages?before=')),
  })
  expect(snapshot()).toEqual({
    statusPanelVisible: false,
    decisionVisible: true,
    olderRequests: [
      '/api/sessions/sample-session/messages?before=200&limit=100',
      '/api/sessions/sample-session/messages?before=100&limit=100',
    ],
  })
})

it('clears a mobile decision jump when no older messages remain', async () => {
  const decision: ChatItem = {
    type: 'decision',
    seq: 12,
    title: 'Keep retry limits predictable',
    why: 'A fixed limit prevents repeated delivery attempts.',
    considered: ['Retry until the request succeeds'],
    easyToChange: 'Update one worker setting.',
    review: null,
    createdAt: '2026-11-11T03:25:59Z',
  }
  const message: ChatItem = { type: 'assistant', text: 'A current message.' }
  let showDecisionInConversation = false
  const fetcher = vi.fn<typeof fetch>((input) => {
    const url = requestUrl(input)
    if (url.endsWith('/messages?limit=100')) {
      return Promise.resolve(
        jsonResponse(
          messagesResponse(
            showDecisionInConversation ? [message, decision] : [message],
            [],
            { start: 0, total: 2, decisions: [decision] },
          ),
        ),
      )
    }
    if (url.includes('/messages?from=0')) {
      showDecisionInConversation = true
      return Promise.resolve(
        jsonResponse(
          messagesResponse([message, decision], [], {
            start: 0,
            total: 2,
            decisions: [decision],
          }),
        ),
      )
    }
    if (url.endsWith('/actions')) {
      return Promise.resolve(jsonResponse(actions))
    }
    return Promise.resolve(jsonResponse({}))
  })
  vi.stubGlobal('fetch', fetcher)
  const scrollIntoView = vi.spyOn(HTMLElement.prototype, 'scrollIntoView')

  function MobileConversationHarness() {
    const [statusPanelOpen, setStatusPanelOpen] = useState(true)
    const [refreshKey, setRefreshKey] = useState(0)
    return (
      <>
        <button
          type="button"
          aria-label="Refresh conversation"
          onClick={() => {
            setRefreshKey((current) => current + 1)
          }}
        />
        <ConversationPane
          session={{ ...session, decisionCount: 1, unreviewedDecisionCount: 1 }}
          shouldSkipSessionTreeFetch
          initialSessionTree={[]}
          refreshKey={refreshKey}
          sessionTreeRefreshKey={0}
          onSelectSession={() => undefined}
          isMobile
          statusPanelOpen={statusPanelOpen}
          onCloseStatusPanel={() => {
            setStatusPanelOpen(false)
          }}
        />
      </>
    )
  }

  root = createRoot(container)
  act(() => root?.render(<MobileConversationHarness />))
  await waitForState(
    () => container.querySelector('.decision-card--panel') !== null,
  )
  await act(async () => {
    await userEvent.click(
      requireElement(
        container.querySelector<HTMLButtonElement>(
          '.decision-card--panel .decision-panel-card-heading',
        ),
      ),
    )
  })
  await act(async () => {
    await userEvent.click(
      requireElement(
        container.querySelector<HTMLButtonElement>(
          'button[aria-label="Refresh conversation"]',
        ),
      ),
    )
  })
  await waitForState(() => container.querySelector('#decision-row-12') !== null)

  const snapshot = () => ({
    statusPanelVisible:
      container.querySelector('.mobile-session-status-overlay') !== null,
    decisionVisible: container.querySelector('#decision-row-12') !== null,
    scrollCalls: scrollIntoView.mock.calls,
  })
  expect(snapshot()).toEqual({
    statusPanelVisible: true,
    decisionVisible: true,
    scrollCalls: [],
  })
})

it('does not retry a mobile decision jump after older history fails', async () => {
  const decision: ChatItem = {
    type: 'decision',
    seq: 12,
    title: 'Keep retry limits predictable',
    why: 'A fixed limit prevents repeated delivery attempts.',
    considered: ['Retry until the request succeeds'],
    easyToChange: 'Update one worker setting.',
    review: null,
    createdAt: '2026-11-11T03:25:59Z',
  }
  const message: ChatItem = { type: 'assistant', text: 'A current message.' }
  let olderRequestCount = 0
  const fetcher = vi.fn<typeof fetch>((input) => {
    const url = requestUrl(input)
    if (url.endsWith('/messages?before=100&limit=100')) {
      olderRequestCount += 1
      if (olderRequestCount === 1) {
        return Promise.reject(new Error('History unavailable'))
      }
      return Promise.resolve(
        jsonResponse(
          messagesResponse([], [], {
            start: 0,
            total: 101,
            decisions: [decision],
          }),
        ),
      )
    }
    if (url.endsWith('/messages?limit=100')) {
      return Promise.resolve(
        jsonResponse(
          messagesResponse([message], [], {
            start: 100,
            total: 101,
            decisions: [decision],
          }),
        ),
      )
    }
    if (url.includes('/messages?from=')) {
      return Promise.resolve(
        jsonResponse(
          messagesResponse([message, decision], [], {
            start: 100,
            total: 102,
            decisions: [decision],
          }),
        ),
      )
    }
    if (url.endsWith('/actions')) {
      return Promise.resolve(jsonResponse(actions))
    }
    return Promise.resolve(jsonResponse({}))
  })
  vi.stubGlobal('fetch', fetcher)
  const scrollIntoView = vi.spyOn(HTMLElement.prototype, 'scrollIntoView')

  function MobileConversationHarness() {
    const [statusPanelOpen, setStatusPanelOpen] = useState(true)
    const [refreshKey, setRefreshKey] = useState(0)
    return (
      <>
        <button
          type="button"
          aria-label="Refresh conversation"
          onClick={() => {
            setRefreshKey((current) => current + 1)
          }}
        />
        <ConversationPane
          session={{ ...session, decisionCount: 1, unreviewedDecisionCount: 1 }}
          shouldSkipSessionTreeFetch
          initialSessionTree={[]}
          refreshKey={refreshKey}
          sessionTreeRefreshKey={0}
          onSelectSession={() => undefined}
          isMobile
          statusPanelOpen={statusPanelOpen}
          onCloseStatusPanel={() => {
            setStatusPanelOpen(false)
          }}
        />
      </>
    )
  }

  root = createRoot(container)
  act(() => root?.render(<MobileConversationHarness />))
  await waitForState(
    () => container.querySelector('.decision-card--panel') !== null,
  )
  await act(async () => {
    await userEvent.click(
      requireElement(
        container.querySelector<HTMLButtonElement>(
          '.decision-card--panel .decision-panel-card-heading',
        ),
      ),
    )
  })
  await waitForState(() => {
    const loadOlderButton = container.querySelector<HTMLButtonElement>(
      '.conversation-load-older',
    )
    return (
      loadOlderButton !== null &&
      !loadOlderButton.disabled &&
      loadOlderButton.textContent === 'Load older messages' &&
      container.querySelector('[role="alert"]')?.textContent ===
        'History unavailable'
    )
  })
  const afterFailure = {
    statusPanelVisible:
      container.querySelector('.mobile-session-status-overlay') !== null,
    olderRequestCount,
    loadOlderButton: container.querySelector('.conversation-load-older')
      ?.textContent,
  }
  await act(async () => {
    await userEvent.click(
      requireElement(
        container.querySelector<HTMLButtonElement>(
          'button[aria-label="Refresh conversation"]',
        ),
      ),
    )
  })
  await waitForState(() => container.querySelector('#decision-row-12') !== null)

  const afterRefresh = {
    statusPanelVisible:
      container.querySelector('.mobile-session-status-overlay') !== null,
    decisionVisible: container.querySelector('#decision-row-12') !== null,
    olderRequestCount,
    scrollCalls: scrollIntoView.mock.calls,
  }
  const expected = {
    afterFailure: {
      statusPanelVisible: true,
      olderRequestCount: 1,
      loadOlderButton: 'Load older messages',
    },
    afterRefresh: {
      statusPanelVisible: true,
      decisionVisible: true,
      olderRequestCount: 1,
      scrollCalls: [],
    },
  }
  const snapshot = () => ({ afterFailure, afterRefresh })
  expect(snapshot()).toEqual(expected)
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
    decisions: ['unreviewed', 'ok'],
    progress: '1 / 2 reviewed',
  })
})

it('opens a registered web UI when other status tabs are available', () => {
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
        webUis: [
          {
            name: 'Design editor',
            url: 'http://127.0.0.1:5000/design/sample/',
          },
        ],
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
    webUiPane: container.querySelector('.web-ui-pane') !== null,
    iframe: container.querySelector('iframe')?.getAttribute('src'),
  })
  expect(snapshot()).toEqual({
    tabs: [
      { label: 'Running1', selected: 'false' },
      { label: 'Decisions1', selected: 'false' },
      { label: 'Design editor', selected: 'true' },
    ],
    selectedTab: 'Design editor',
    webUiPane: true,
    iframe: 'http://localhost:5000/design/sample/',
  })
})

it('opens a registered deep link when the desktop status panel starts closed', async () => {
  const webUi = {
    name: 'Canvas editor',
    url: 'http://localhost:43123/studio/',
  }
  const message: ChatItem = {
    type: 'assistant',
    text: '[canvas page](http://localhost:43123/studio/pages/sample#page-fakehash)',
  }
  vi.stubGlobal(
    'fetch',
    vi.fn<typeof fetch>((input) => {
      const url = requestUrl(input)
      if (url.endsWith('/messages?limit=100')) {
        return Promise.resolve(jsonResponse(messagesResponse([message])))
      }
      if (url.endsWith('/actions')) {
        return Promise.resolve(jsonResponse(actions))
      }
      return Promise.resolve(jsonResponse({}))
    }),
  )
  function ConversationHarness() {
    const [statusPanelOpen, setStatusPanelOpen] = useState(false)
    return (
      <ConversationPane
        session={{ ...session, webUis: [webUi] }}
        shouldSkipSessionTreeFetch
        initialSessionTree={[]}
        refreshKey={0}
        sessionTreeRefreshKey={0}
        onSelectSession={() => undefined}
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
  act(() => root?.render(<ConversationHarness />))
  await waitForState(
    () => container.querySelector('.markdown-web-ui-link a') !== null,
  )
  const link = requireElement(
    container.querySelector<HTMLAnchorElement>('.markdown-web-ui-link a'),
  )
  await act(async () => userEvent.click(link))

  const snapshot = () => ({
    statusPanel: container.querySelector('.session-status-panel') !== null,
    selectedTab: container.querySelector('[role="tab"][aria-selected="true"]')
      ?.textContent,
    fullscreen: container
      .querySelector('.web-ui-pane')
      ?.classList.contains('is-fullscreen'),
    iframe: container.querySelector('iframe')?.getAttribute('src') ?? null,
  })
  expect(snapshot()).toEqual({
    statusPanel: true,
    selectedTab: 'Canvas editor',
    fullscreen: true,
    iframe: 'http://localhost:43123/studio/pages/sample#page-fakehash',
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
  const pullRequests = [
    {
      number: 42,
      state: 'Open' as const,
      owner: 'example-org',
      repo: 'example-service',
      title: 'Allow a bounded retry policy',
      headSha: 'fixture-retry-head',
      ciState: 'Successful' as const,
      isCurrentBranch: true,
    },
    {
      number: 43,
      state: 'Open' as const,
      owner: 'example-org',
      repo: 'example-infra',
      title: 'Record retry outcomes',
      headSha: 'fixture-metrics-head',
      ciState: 'Failed' as const,
      isCurrentBranch: false,
    },
  ]
  const sessionWithPullRequests: Session = {
    ...session,
    pullRequest: pullRequests[0] ?? null,
    pullRequests,
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
        session={sessionWithPullRequests}
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
  const toggle = requireElement(
    container.querySelector<HTMLButtonElement>(
      '.mobile-conversation-status-toggle',
    ),
  )
  toggle.focus()
  act(() => {
    toggle.click()
  })

  const opened = {
    overlay: container.querySelector('.mobile-session-status-overlay') !== null,
    dialogFocused:
      document.activeElement ===
      container.querySelector('.mobile-session-status-overlay [role="dialog"]'),
    selectedTab: container.querySelector('[role="tab"][aria-selected="true"]')
      ?.textContent,
    decisionHeading: container.querySelector('.decisions-panel-header h2')
      ?.textContent,
    backButtonFocused:
      document.activeElement ===
      container.querySelector('.session-status-panel-back'),
    pullRequestTabs: Array.from(
      container.querySelectorAll<HTMLButtonElement>(
        '.mobile-session-status-overlay [role="tab"]',
      ),
      (tab) => tab.textContent,
    ),
    pullRequestRows: Array.from(
      container.querySelectorAll<HTMLElement>(
        '.mobile-session-status-overlay .session-pr-list--panel .session-pr-row',
      ),
      (row) => ({
        title: row.querySelector('.session-pr-row-title')?.textContent ?? null,
        action: row.querySelector('.pr-merge-entry')?.textContent ?? null,
      }),
    ),
  }
  act(() => {
    requireElement(
      container.querySelector<HTMLButtonElement>(
        '.mobile-session-status-overlay #session-status-tab-pull-requests',
      ),
    ).click()
  })
  const pullRequestTab = {
    selectedTab: container.querySelector('[role="tab"][aria-selected="true"]')
      ?.textContent,
    pullRequestRows: Array.from(
      container.querySelectorAll<HTMLElement>(
        '.mobile-session-status-overlay .session-pr-list--panel .session-pr-row',
      ),
      (row) => ({
        title: row.querySelector('.session-pr-row-title')?.textContent ?? null,
        action: row.querySelector('.pr-merge-entry')?.textContent ?? null,
      }),
    ),
  }
  act(() => {
    requireElement(
      container.querySelector<HTMLButtonElement>(
        '.mobile-session-status-overlay .session-status-panel-back',
      ),
    ).click()
  })
  const closed = {
    overlay: container.querySelector('.mobile-session-status-overlay') !== null,
    conversation: container.querySelector('.conversation-pane') !== null,
    focusRestored: document.activeElement === toggle,
  }

  const snapshot = () => ({ opened, pullRequestTab, closed })
  expect(snapshot()).toEqual({
    opened: {
      overlay: true,
      dialogFocused: true,
      selectedTab: 'Decisions1',
      decisionHeading: 'Decisions in this session',
      backButtonFocused: false,
      pullRequestTabs: ['PRs2', 'Decisions1'],
      pullRequestRows: [],
    },
    pullRequestTab: {
      selectedTab: 'PRs2',
      pullRequestRows: [
        { title: '#42 Allow a bounded retry policy', action: 'Merge' },
        { title: '#43 Record retry outcomes', action: 'CI failing' },
      ],
    },
    closed: { overlay: false, conversation: true, focusRestored: true },
  })
})

it('marks phone navigation after an Activity action fails', async () => {
  function FailureHarness() {
    const startAsyncAction = useAsyncActionStarter(() => undefined)
    return (
      <>
        <button
          className="create-failed-activity-action"
          type="button"
          onClick={() => {
            startAsyncAction({
              kind: 'command',
              sessionId: session.id,
              sessionTitle: session.title,
              command: 'run fixture action',
              run: () =>
                Promise.resolve({
                  status: 'commandFailed',
                  exitCode: 1,
                  error: 'The fixture action failed.',
                }),
            })
          }}
        >
          Create failed Activity action
        </button>
        <ConversationPaneView
          session={session}
          messages={[]}
          start={0}
          actions={actions}
          loading={false}
          error={null}
          onAction={() => Promise.resolve(true)}
          onSelectSession={() => undefined}
          isMobile
        />
      </>
    )
  }

  root = createRoot(container)
  act(() => {
    root?.render(
      <AsyncActionFeedbackProvider>
        <FailureHarness />
      </AsyncActionFeedbackProvider>,
    )
  })
  const createFailedAction = requireElement(
    container.querySelector<HTMLButtonElement>(
      '.create-failed-activity-action',
    ),
  )
  await act(async () => userEvent.click(createFailedAction))
  expect(
    await waitForState(
      () =>
        container
          .querySelector<HTMLButtonElement>('.mobile-conversation-nav-toggle')
          ?.getAttribute('aria-label') ===
        'Open navigation, Activity has failed actions',
    ),
  ).toBe(true)

  const navigationButton = requireElement(
    container.querySelector<HTMLButtonElement>(
      '.mobile-conversation-nav-toggle',
    ),
  )
  const snapshot = () => ({
    label: navigationButton.getAttribute('aria-label'),
    failureMark:
      navigationButton.querySelector('.mobile-conversation-nav-failure') !==
      null,
  })
  expect(snapshot()).toEqual({
    label: 'Open navigation, Activity has failed actions',
    failureMark: true,
  })
})

it('omits the paused badge from the conversation header', () => {
  renderView(
    vi.fn().mockResolvedValue(true),
    { ...session, category: 'done', kind: null, isPaused: true },
    undefined,
    [],
    null,
  )

  const snapshot = () => ({
    title: container.querySelector('.conversation-header h1')?.textContent,
    pausedBadge:
      container.querySelector('.conversation-header [data-slot="chip"]') !==
      null,
  })
  expect(snapshot()).toEqual({
    title: 'Choose a retry policy',
    pausedBadge: false,
  })
})

it('omits the paused badge from the mobile conversation header', () => {
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
    title: container.querySelector('.mobile-conversation-title h1')
      ?.textContent,
    pausedBadge:
      container.querySelector(
        '.mobile-conversation-title [data-slot="chip"]',
      ) !== null,
  })
  expect(snapshot()).toEqual({
    mobile: 'true',
    title: 'Choose a retry policy',
    pausedBadge: false,
  })
})

it('shows only the task and short repository name in the mobile context row', () => {
  const task = {
    id: 'example-open-task',
    number: 2048,
    title: 'Improve request reliability across several delivery paths',
    priority: 1,
  }
  renderView(
    vi.fn().mockResolvedValue(true),
    {
      ...session,
      task,
      tasks: [task, { ...task, id: 'example-secondary-task', number: 2049 }],
      repo: 'example/long-running-service-name',
    },
    undefined,
    [],
    null,
    true,
  )

  const taskLink = container.querySelector<HTMLElement>(
    '.mobile-conversation-context a',
  )
  const repo = container.querySelector<HTMLElement>('.mobile-conversation-repo')
  const snapshot = () => ({
    task: taskLink?.textContent,
    repo: repo?.textContent,
    disclosure: container.querySelector('.mobile-conversation-context summary')
      ?.textContent,
    taskTextOverflow:
      taskLink === null ? null : window.getComputedStyle(taskLink).textOverflow,
    repoFlexShrink:
      repo === null ? null : window.getComputedStyle(repo).flexShrink,
    separators: Array.from(
      container.querySelectorAll(
        '.mobile-conversation-context > span[aria-hidden="true"]',
      ),
      (separator) => separator.textContent,
    ),
  })

  expect(snapshot()).toEqual({
    task: 'P1 #2048 Improve request reliability across several delivery paths',
    repo: 'long-running-service-name',
    disclosure: undefined,
    taskTextOverflow: 'ellipsis',
    repoFlexShrink: '0',
    separators: ['·'],
  })
})

it.each([
  { layout: 'desktop', isMobile: false },
  { layout: 'mobile', isMobile: true },
])(
  'does not show a conversation divider in the $layout pane',
  ({ isMobile }) => {
    renderView(
      vi.fn().mockResolvedValue(true),
      session,
      undefined,
      [],
      actions,
      isMobile,
    )

    const snapshot = () => ({
      visible: container.querySelector('.conversation-timeline') !== null,
    })
    expect(snapshot()).toEqual({ visible: false })
  },
)

it('offers the status panel from the mobile header without a task count', () => {
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
    count: undefined,
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

it('opens the Crit review fullscreen from the status band in one click', async () => {
  const fetcher = vi.fn<typeof fetch>((input) => {
    const url = requestUrl(input)
    if (url.endsWith('/messages?limit=100')) {
      return Promise.resolve(jsonResponse(messagesResponse([])))
    }
    if (url.endsWith('/actions')) return Promise.resolve(jsonResponse(actions))
    return Promise.resolve(jsonResponse({}))
  })
  vi.stubGlobal('fetch', fetcher)
  function StatusPanelHarness() {
    const [statusPanelOpen, setStatusPanelOpen] = useState(false)
    return (
      <ConversationPane
        session={{
          ...session,
          category: 'needs_action',
          activityStatus: 'stopped',
          isWaitingForCritReview: true,
          kind: 'Review',
          critUrl: 'https://crit.example.test/review/fixture',
          webUis: [],
        }}
        refreshKey={0}
        sessionTreeRefreshKey={0}
        onSelectSession={() => undefined}
        statusPanelOpen={statusPanelOpen}
        onToggleStatusPanel={() => {
          setStatusPanelOpen((open) => !open)
        }}
      />
    )
  }
  root = createRoot(container)
  act(() => root?.render(<StatusPanelHarness />))
  await act(async () => {
    for (let index = 0; index < 12; index += 1) await Promise.resolve()
  })

  const reviewButton = await waitForState(
    () =>
      container.querySelector<HTMLButtonElement>(
        '.conversation-status-band-review',
      ) !== null,
  )
  if (!reviewButton) throw new Error('Expected the Crit review action')
  await act(async () => {
    await userEvent.click(
      requireElement(
        container.querySelector<HTMLButtonElement>(
          '.conversation-status-band-review',
        ),
      ),
    )
  })
  const fullscreen = await waitForState(
    () => container.querySelector('.web-ui-pane.is-fullscreen') !== null,
  )

  const snapshot = () => ({
    fullscreen,
    selectedTab: container.querySelector('[role="tab"][aria-selected="true"]')
      ?.textContent,
    fullscreenControl: container
      .querySelector('.web-ui-pane-header button[aria-label*="fullscreen"]')
      ?.getAttribute('aria-label'),
    reviewUrl: container
      .querySelector<HTMLIFrameElement>('.web-ui-pane iframe')
      ?.getAttribute('src'),
  })
  expect(snapshot()).toEqual({
    fullscreen: true,
    selectedTab: 'crit',
    fullscreenControl: 'Exit crit fullscreen',
    reviewUrl: 'https://crit.example.test/review/fixture',
  })
})

it('opens the Review tab from the draft review band in one click', async () => {
  const fetcher = vi.fn<typeof fetch>((input) => {
    const url = requestUrl(input)
    if (url.endsWith('/messages?limit=100')) {
      return Promise.resolve(jsonResponse(messagesResponse([])))
    }
    if (url.endsWith('/actions')) return Promise.resolve(jsonResponse(actions))
    if (url.endsWith('/review-documents')) {
      return Promise.resolve(
        jsonResponse([
          {
            id: 'fixture-review-document',
            kind: 'issue_comment',
            target: { issueNumber: 1234 },
            isEditor: true,
            canSave: true,
            canFinish: true,
          },
        ]),
      )
    }
    if (url.endsWith('/review-documents/fixture-review-document')) {
      return Promise.resolve(
        jsonResponse({
          id: 'fixture-review-document',
          revision: 'fixture-revision',
          frontmatter: {},
          body: '',
        }),
      )
    }
    return Promise.resolve(jsonResponse({}))
  })
  vi.stubGlobal('fetch', fetcher)
  function StatusPanelHarness() {
    const [statusPanelOpen, setStatusPanelOpen] = useState(false)
    return (
      <ConversationPane
        session={{
          ...session,
          reviewDocumentId: 'fixture-review-document',
          reviewDocumentCount: 1,
          webUis: [],
        }}
        refreshKey={0}
        sessionTreeRefreshKey={0}
        onSelectSession={() => undefined}
        statusPanelOpen={statusPanelOpen}
        onToggleStatusPanel={() => {
          setStatusPanelOpen((open) => !open)
        }}
      />
    )
  }
  root = createRoot(container)
  act(() => root?.render(<StatusPanelHarness />))
  await act(async () => {
    for (let index = 0; index < 12; index += 1) await Promise.resolve()
  })

  const reviewButton = await waitForState(
    () =>
      container.querySelector<HTMLButtonElement>(
        '.conversation-status-band-review',
      ) !== null,
  )
  if (!reviewButton) throw new Error('Expected the draft review action')
  await act(async () => {
    await userEvent.click(
      requireElement(
        container.querySelector<HTMLButtonElement>(
          '.conversation-status-band-review',
        ),
      ),
    )
  })
  const reviewTab = await waitForState(
    () =>
      container.querySelector('[role="tab"][aria-selected="true"]')
        ?.textContent === 'Review1',
  )

  const snapshot = () => ({
    reviewTab,
    selectedTab: container.querySelector('[role="tab"][aria-selected="true"]')
      ?.textContent,
    reviewAction: container.querySelector('.conversation-status-band-review')
      ?.textContent,
  })
  expect(snapshot()).toEqual({
    reviewTab: true,
    selectedTab: 'Review1',
    reviewAction: 'Open drafts →',
  })
})

it('keeps a review action result at the conversation boundary where it started', async () => {
  const firstMessage: ChatItem = {
    type: 'assistant',
    text: 'The draft is ready for review.',
  }
  const existingToolSummary: ChatItem = {
    type: 'tool_summary',
    summary: 'Shell ×1',
    operations: [{ tool: 'exec', target: 'node fixture.js', failed: false }],
  }
  const initialMessages: ChatItem[] = [
    firstMessage,
    existingToolSummary,
    {
      type: 'tool_in_progress',
      tool: 'exec',
      target: 'node fixture.js',
    },
  ]
  const messagesAfterToolCompletes: ChatItem[] = [
    firstMessage,
    {
      type: 'tool_summary',
      summary: 'Shell ×2',
      operations: [
        { tool: 'exec', target: 'node fixture.js', failed: false },
        { tool: 'exec', target: 'node fixture.js', failed: false },
      ],
    },
  ]
  const laterMessage: ChatItem = {
    type: 'assistant',
    text: 'The issue check is complete.',
  }
  let showLaterMessage = false
  let completeReviewAction: ((response: Response) => void) | undefined
  const fetcher = vi.fn<typeof fetch>((input) => {
    const url = requestUrl(input)
    if (url.includes('/messages?')) {
      return Promise.resolve(
        jsonResponse(
          messagesResponse(
            showLaterMessage
              ? [...messagesAfterToolCompletes, laterMessage]
              : initialMessages,
          ),
        ),
      )
    }
    if (url.endsWith('/actions')) return Promise.resolve(jsonResponse(actions))
    if (url.endsWith('/review-documents')) {
      return Promise.resolve(
        jsonResponse([
          {
            id: 'fixture-review-document',
            kind: 'issue_comment',
            target: { issueNumber: 51 },
            isEditor: true,
            canSave: true,
            canFinish: true,
          },
        ]),
      )
    }
    if (url.endsWith('/review-documents/fixture-review-document')) {
      return Promise.resolve(
        jsonResponse({
          id: 'fixture-review-document',
          revision: 'fixture-revision',
          frontmatter: {},
          body: 'Fixture issue comment',
        }),
      )
    }
    if (url.endsWith('/review-document/action')) {
      return new Promise((resolve) => {
        completeReviewAction = resolve
      })
    }
    return Promise.resolve(jsonResponse({}))
  })
  vi.stubGlobal('fetch', fetcher)

  function StatusPanelHarness() {
    const [refreshKey, setRefreshKey] = useState(0)
    return (
      <>
        <button
          type="button"
          onClick={() => {
            showLaterMessage = true
            setRefreshKey((current) => current + 1)
          }}
        >
          Refresh conversation
        </button>
        <ConversationPane
          session={{
            ...session,
            category: 'in_progress',
            kind: 'Review',
            reviewDocumentId: 'fixture-review-document',
            reviewDocumentCount: 1,
            webUis: [],
          }}
          refreshKey={refreshKey}
          sessionTreeRefreshKey={0}
          onSelectSession={() => undefined}
          statusPanelOpen
        />
      </>
    )
  }

  root = createRoot(container)
  act(() => {
    root?.render(
      <AsyncActionFeedbackProvider>
        <StatusPanelHarness />
      </AsyncActionFeedbackProvider>,
    )
  })
  expect(
    await waitForState(() =>
      [...container.querySelectorAll<HTMLButtonElement>('[role="tab"]')].some(
        (button) => button.textContent === 'Review1',
      ),
    ),
  ).toBe(true)
  await act(async () => {
    await userEvent.click(
      requireElement(
        [...container.querySelectorAll<HTMLButtonElement>('[role="tab"]')].find(
          (button) => button.textContent === 'Review1',
        ) ?? null,
      ),
    )
  })
  expect(
    await waitForState(() =>
      [...container.querySelectorAll<HTMLButtonElement>('button')].some(
        (button) => button.textContent === 'Post comment',
      ),
    ),
  ).toBe(true)
  await act(async () => {
    await userEvent.click(
      requireElement(
        [...container.querySelectorAll<HTMLButtonElement>('button')].find(
          (button) => button.textContent === 'Post comment',
        ) ?? null,
      ),
    )
  })
  expect(
    await waitForState(
      () =>
        container.querySelector('.async-action-item--line')?.textContent ===
        'Post comment in progress',
    ),
  ).toBe(true)

  await act(async () => {
    await userEvent.click(
      requireElement(
        [...container.querySelectorAll<HTMLButtonElement>('button')].find(
          (button) => button.textContent === 'Refresh conversation',
        ) ?? null,
      ),
    )
  })
  expect(
    await waitForState(() =>
      [...container.querySelectorAll('.chat-assistant-message')].some(
        (message) => message.textContent === laterMessage.text,
      ),
    ),
  ).toBe(true)
  await act(async () => {
    completeReviewAction?.(
      jsonResponse({ success: true, revision: 'next-revision' }),
    )
    await Promise.resolve()
  })
  expect(
    await waitForState(
      () =>
        normalizeAsyncActionLineTime(
          container.querySelector('.async-action-item--line')?.textContent,
        ) === 'Post comment completed · <time>',
    ),
  ).toBe(true)

  const snapshot = () =>
    Array.from(
      container.querySelectorAll(
        '.conversation-scroll .chat-assistant-message, .conversation-scroll .chat-tool-summary, .conversation-scroll .async-action-item--line',
      ),
      (element) =>
        element.matches('.chat-tool-summary')
          ? 'tool summary'
          : normalizeAsyncActionLineTime(element.textContent),
    )
  expect(snapshot()).toEqual([
    'The draft is ready for review.',
    'tool summary',
    'Post comment completed · <time>',
    'The issue check is complete.',
  ])
})

it('returns from DraftFocus with the Back to conversation button', async () => {
  const reviewDocuments = [
    {
      id: 'fixture-focus-document',
      kind: 'issue_comment' as const,
      target: {
        repository: 'example/web-app',
        issueNumber: 412,
        title: 'Delivery checks fail on the fourth attempt',
      },
      isEditor: true,
      canSave: true,
      canFinish: true,
    },
  ]
  const fetcher = vi.fn<typeof fetch>((input) => {
    const url = requestUrl(input)
    if (url.endsWith('/messages?limit=100')) {
      return Promise.resolve(jsonResponse(messagesResponse([])))
    }
    if (url.endsWith('/actions')) return Promise.resolve(jsonResponse(actions))
    if (url.endsWith('/review-documents')) {
      return Promise.resolve(jsonResponse(reviewDocuments))
    }
    if (url.endsWith('/review-documents/fixture-focus-document')) {
      return Promise.resolve(
        jsonResponse({
          id: 'fixture-focus-document',
          revision: 'fixture-focus-revision',
          frontmatter: { priority: 'normal' },
          body: 'Readable fixture draft',
        }),
      )
    }
    return Promise.resolve(jsonResponse({}))
  })
  vi.stubGlobal('fetch', fetcher)

  function FocusConversationHarness() {
    const [statusPanelOpen, setStatusPanelOpen] = useState(false)
    return (
      <ConversationPane
        session={{
          ...session,
          kind: 'Review',
          reviewDocumentId: 'fixture-focus-document',
          reviewDocumentCount: 1,
          webUis: [],
        }}
        shouldSkipSessionTreeFetch
        initialSessionTree={[]}
        refreshKey={0}
        sessionTreeRefreshKey={0}
        onSelectSession={() => undefined}
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
  act(() => root?.render(<FocusConversationHarness />))
  const reviewActionReady = await waitForState(
    () =>
      container.querySelector<HTMLButtonElement>(
        '.conversation-status-band-review',
      ) !== null,
  )
  expect(reviewActionReady).toBe(true)
  await act(async () => {
    await userEvent.click(
      requireElement(
        container.querySelector<HTMLButtonElement>(
          '.conversation-status-band-review',
        ),
      ),
    )
  })
  const reviewLoaded = await waitForState(
    () =>
      container.querySelector('.review-document-issue-comment-body')
        ?.textContent === 'Readable fixture draft',
  )
  expect(reviewLoaded).toBe(true)
  await act(async () => {
    await userEvent.click(
      requireElement(
        container.querySelector<HTMLButtonElement>(
          '.review-document-full-width[aria-label="Open draft full width"]',
        ),
      ),
    )
  })
  const fullFocusReady = await waitForState(
    () => container.querySelector('.review-document-focus') !== null,
  )
  expect(fullFocusReady).toBe(true)
  await act(async () => {
    await userEvent.click(
      requireElement(
        container.querySelector<HTMLButtonElement>(
          '.review-document-focus-back[aria-label="Back to conversation"]',
        ),
      ),
    )
  })

  const snapshot = () => ({
    focusCount: container.querySelectorAll('.review-document-focus').length,
    reviewPanelCount: container.querySelectorAll('.review-document-panel')
      .length,
    statusPanelCount: container.querySelectorAll('.session-status-panel')
      .length,
  })
  expect(snapshot()).toEqual({
    focusCount: 0,
    reviewPanelCount: 1,
    statusPanelCount: 1,
  })
})

it('opens DraftFocus and exits Review after a draft load failure', async () => {
  const reviewDocuments = [
    {
      id: 'fixture-review-a',
      kind: 'issue_comment' as const,
      target: {
        repository: 'example/web-app',
        issueNumber: 412,
        title: 'Delivery checks fail on the fourth attempt',
      },
      isEditor: true,
      canSave: true,
      canFinish: true,
    },
    {
      id: 'fixture-review-b',
      kind: 'pr_draft' as const,
      target: { repository: 'example/web-app', branch: 'feature/retry-limit' },
      isEditor: true,
      canSave: true,
      canFinish: true,
    },
  ]
  const fetcher = vi.fn<typeof fetch>((input) => {
    const url = requestUrl(input)
    if (url.endsWith('/messages?limit=100')) {
      return Promise.resolve(jsonResponse(messagesResponse([])))
    }
    if (url.endsWith('/actions')) return Promise.resolve(jsonResponse(actions))
    if (url.endsWith('/review-documents')) {
      return Promise.resolve(jsonResponse(reviewDocuments))
    }
    if (url.endsWith('/review-documents/fixture-review-a')) {
      return Promise.resolve(
        jsonResponse({
          id: 'fixture-review-a',
          revision: 'fixture-revision-a',
          frontmatter: { priority: 'normal' },
          body: 'Readable fixture draft',
        }),
      )
    }
    if (url.endsWith('/review-documents/fixture-review-b')) {
      return Promise.resolve(jsonResponse({}, 503))
    }
    return Promise.resolve(jsonResponse({}))
  })
  vi.stubGlobal('fetch', fetcher)

  function ReviewConversationHarness() {
    const [statusPanelOpen, setStatusPanelOpen] = useState(false)
    return (
      <ConversationPane
        session={{
          ...session,
          kind: 'Review',
          reviewDocumentId: 'fixture-review-a',
          reviewDocumentCount: 2,
          webUis: [],
        }}
        shouldSkipSessionTreeFetch
        initialSessionTree={[]}
        refreshKey={0}
        sessionTreeRefreshKey={0}
        onSelectSession={() => undefined}
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
  act(() => root?.render(<ReviewConversationHarness />))
  const reviewActionReady = await waitForState(
    () =>
      container.querySelector<HTMLButtonElement>(
        '.conversation-status-band-review',
      ) !== null,
  )
  expect(reviewActionReady).toBe(true)
  await act(async () => {
    await userEvent.click(
      requireElement(
        container.querySelector<HTMLButtonElement>(
          '.conversation-status-band-review',
        ),
      ),
    )
  })
  const reviewLoaded = await waitForState(
    () =>
      container.querySelector('.review-document-issue-comment-body')
        ?.textContent === 'Readable fixture draft',
  )
  expect(reviewLoaded).toBe(true)
  await act(async () => {
    await userEvent.click(
      requireElement(
        container.querySelector<HTMLButtonElement>(
          '.review-document-full-width[aria-label="Open draft full width"]',
        ),
      ),
    )
  })
  const fullFocusReady = await waitForState(
    () => container.querySelector('.review-document-focus') !== null,
  )
  expect(fullFocusReady).toBe(true)
  const focusSnapshot = () => ({
    sessionTitle: container.querySelector('.review-document-focus-session')
      ?.textContent,
    draftTitle: container.querySelector('.review-document-focus-header strong')
      ?.textContent,
    columns: Array.from(
      container.querySelectorAll('.review-document-focus-column h2'),
      (heading) => heading.textContent,
    ),
    markdown: container.querySelector('.review-document-focus-markdown pre')
      ?.textContent,
    preview: container.querySelector(
      '.review-document-focus-preview .review-document-body',
    )?.textContent,
    statusPanelCount: container.querySelectorAll('.session-status-panel')
      .length,
    metadataCount: container.querySelectorAll('.review-document-frontmatter')
      .length,
    actionCount: container.querySelectorAll('.review-document-actions').length,
  })
  expect(focusSnapshot()).toEqual({
    sessionTitle: 'Choose a retry policy',
    draftTitle: 'Draft',
    columns: ['MARKDOWN', 'PREVIEW'],
    markdown: 'Readable fixture draft',
    preview: 'Readable fixture draft',
    statusPanelCount: 0,
    metadataCount: 0,
    actionCount: 0,
  })
  await act(async () => {
    await userEvent.keyboard('{Escape}')
  })
  const reviewRestored = await waitForState(
    () => container.querySelector('.review-document-panel') !== null,
  )
  expect(reviewRestored).toBe(true)
  const queuedDraftTab = requireElement(
    container.querySelectorAll<HTMLButtonElement>('.review-document-tab')[1] ??
      null,
  )
  await act(async () => userEvent.click(queuedDraftTab))
  const draftErrorShown = await waitForState(
    () => container.querySelector('[role="alert"]') !== null,
  )
  expect(draftErrorShown).toBe(true)
  const failureSnapshot = () => ({
    alert: container.querySelector('[role="alert"]')?.textContent,
  })
  expect(failureSnapshot()).toEqual({
    alert: 'Could not load the draft: Request failed (503)Retry draft',
  })
  const closeReviewPanel = requireElement(
    container.querySelector<HTMLButtonElement>(
      '.session-status-panel-close[aria-label="Close session status panel"]',
    ),
  )
  await act(async () => userEvent.click(closeReviewPanel))
  const reviewClosed = await waitForState(
    () => container.querySelector('.session-status-panel') === null,
  )
  expect(reviewClosed).toBe(true)
})

it('directs Codex questions and permissions to the terminal', () => {
  const onAction = vi.fn(() => Promise.resolve(true))
  const codexActions: SessionActions = {
    message: true,
    answerQuestion: false,
    interrupt: true,
    openInTerminal: true,
    rewind: false,
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
  setTextareaValue(textarea, 'Retain this draft')
  act(() => {
    dispatchKey(textarea, 'Enter')
  })
  const draftWhileSending = textarea.value
  await act(async () => {
    completeSend?.(false)
    await Promise.resolve()
  })

  const snapshot = () => ({
    draftWhileSending,
    draft: textarea.value,
    storedDraft: window.localStorage.getItem(
      'agent-watch-web:composer-draft:sample-session',
    ),
    actions: onAction.mock.calls,
  })
  expect(snapshot()).toEqual({
    draftWhileSending: '',
    draft: 'Retain this draft',
    storedDraft: 'Retain this draft',
    actions: [['messages', { content: 'Retain this draft' }]],
  })
})

it('keeps text entered during a failed send', async () => {
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
  setTextareaValue(textarea, 'Original request')
  act(() => {
    dispatchKey(textarea, 'Enter')
  })
  setTextareaValue(textarea, 'New request')

  await act(async () => {
    completeSend?.(false)
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
    draft: 'New request',
    storedDraft: 'New request',
    actions: [['messages', { content: 'Original request' }]],
  })
})

it('does not restore a sent draft after the user clears a newer draft', async () => {
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
  setTextareaValue(textarea, 'Original request')
  act(() => {
    dispatchKey(textarea, 'Enter')
  })
  setTextareaValue(textarea, 'New request')
  setTextareaValue(textarea, '')

  await act(async () => {
    completeSend?.(false)
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
    actions: [['messages', { content: 'Original request' }]],
  })
})

it('clears an IME composition restored after submitting from the send button', async () => {
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
  const sentDraft = 'Composed request'
  setTextareaValue(textarea, sentDraft)
  act(() => {
    textarea.dispatchEvent(
      new CompositionEvent('compositionstart', { bubbles: true }),
    )
  })
  const sendButton = requireElement(
    container.querySelector<HTMLButtonElement>('[aria-label="Send message"]'),
  )

  act(() => {
    sendButton.click()
  })
  const draftWhileSending = textarea.value
  act(() => {
    setTextareaValue(textarea, 'Intermediate composition')
    textarea.dispatchEvent(
      new CompositionEvent('compositionend', {
        bubbles: true,
        data: sentDraft,
      }),
    )
    setTextareaValue(textarea, sentDraft)
  })
  await act(async () => {
    completeSend?.(true)
    await Promise.resolve()
  })

  const snapshot = () => ({
    draftWhileSending,
    draft: textarea.value,
    storedDraft: window.localStorage.getItem(
      'agent-watch-web:composer-draft:sample-session',
    ),
    actions: onAction.mock.calls,
  })
  expect(snapshot()).toEqual({
    draftWhileSending: '',
    draft: '',
    storedDraft: null,
    actions: [['messages', { content: sentDraft }]],
  })
})

it('opens the forked session with the selected message when storage rejects its draft', async () => {
  const originalSession = { ...session, engine: 'claude' as const }
  const forkedSession = { ...originalSession, id: 'fixture-fork-session' }
  const selectedSessions = vi.fn()
  const selectedMessage: ChatItem = {
    type: 'user',
    text: 'Revisit this request.',
    rewindId: 'fixture-user-message',
  }
  let completeRewind: ((response: Response) => void) | undefined
  const originalSetItem = Storage.prototype.setItem.bind(window.localStorage)
  vi.spyOn(Storage.prototype, 'setItem').mockImplementation((key, value) => {
    if (key === `agent-watch-web:composer-draft:${forkedSession.id}`) {
      throw new Error('storage unavailable')
    }
    originalSetItem(key, value)
  })
  const fetcher = vi.fn<typeof fetch>((input) => {
    const url = requestUrl(input)
    if (url.endsWith('/messages?limit=100')) {
      return Promise.resolve(
        jsonResponse(
          url.includes(forkedSession.id)
            ? messagesResponse([])
            : messagesResponse([selectedMessage]),
        ),
      )
    }
    if (url.endsWith('/actions')) {
      return Promise.resolve(jsonResponse({ ...actions, rewind: true }))
    }
    if (url.endsWith('/rewind')) {
      return new Promise((resolve) => {
        completeRewind = resolve
      })
    }
    return Promise.resolve(jsonResponse({}))
  })
  vi.stubGlobal('fetch', fetcher)

  function RewindHarness() {
    const [currentSession, setCurrentSession] = useState(originalSession)
    return (
      <ConversationPane
        key={currentSession.id}
        session={currentSession}
        refreshKey={0}
        sessionTreeRefreshKey={0}
        onSelectSession={(id) => {
          selectedSessions(id)
          setCurrentSession(forkedSession)
        }}
      />
    )
  }

  root = createRoot(container)
  act(() => {
    root?.render(
      <AsyncActionFeedbackProvider>
        <RewindHarness />
      </AsyncActionFeedbackProvider>,
    )
  })
  expect(
    await waitForState(
      () => container.querySelector('.chat-rewind-menu summary') !== null,
    ),
  ).toBe(true)
  act(() => {
    container.querySelector<HTMLElement>('.chat-rewind-menu summary')?.click()
  })
  act(() => {
    container
      .querySelector<HTMLButtonElement>('.chat-rewind-menu button')
      ?.click()
  })
  expect(
    await waitForState(
      () =>
        container.querySelector('.async-action-toast-stack')?.textContent ===
        'Rewinding sessionChoose a retry policy',
    ),
  ).toBe(true)
  await act(async () => {
    completeRewind?.(jsonResponse({ sessionId: forkedSession.id }))
    await Promise.resolve()
  })
  expect(
    await waitForState(
      () =>
        container.querySelector<HTMLTextAreaElement>('textarea')?.value ===
        selectedMessage.text,
    ),
  ).toBe(true)

  const snapshot = () => ({
    selectedSessions: selectedSessions.mock.calls,
    rewindRequests: fetcher.mock.calls
      .filter(([, init]) => init?.method === 'POST')
      .map(([input, init]) => ({
        url: requestUrl(input),
        body:
          typeof init?.body === 'string'
            ? (JSON.parse(init.body) as unknown)
            : null,
      })),
    draft: window.localStorage.getItem(
      `agent-watch-web:composer-draft:${forkedSession.id}`,
    ),
    composer: container.querySelector<HTMLTextAreaElement>('textarea')?.value,
    feedback: container.querySelector('.async-action-toast-stack')?.textContent,
  })
  expect(snapshot()).toEqual({
    selectedSessions: [['fixture-fork-session']],
    rewindRequests: [
      {
        url: '/api/sessions/sample-session/rewind',
        body: {
          messageId: 'fixture-user-message',
          mode: 'conversation_and_code',
        },
      },
    ],
    draft: null,
    composer: 'Revisit this request.',
    feedback: 'Rewound sessionChoose a retry policy',
  })
})

it('shows rewind request errors without switching sessions or offering Retry', async () => {
  const originalSession = { ...session, engine: 'claude' as const }
  const forkedSession = { ...originalSession, id: 'fixture-fork-session' }
  const selectedSessions = vi.fn()
  const earlierMessage: ChatItem = {
    type: 'assistant',
    text: 'The request context is ready.',
  }
  const selectedMessage: ChatItem = {
    type: 'user',
    text: 'Revisit this request.',
    rewindId: 'fixture-user-message',
  }
  const laterMessage: ChatItem = {
    type: 'assistant',
    text: 'I will continue with the follow-up.',
  }
  const fetcher = vi.fn<typeof fetch>((input) => {
    const url = requestUrl(input)
    if (url.endsWith('/messages?limit=100')) {
      return Promise.resolve(
        jsonResponse(
          messagesResponse([earlierMessage, selectedMessage, laterMessage]),
        ),
      )
    }
    if (url.endsWith('/actions')) {
      return Promise.resolve(jsonResponse({ ...actions, rewind: true }))
    }
    if (url.endsWith('/rewind')) {
      return Promise.resolve(
        jsonResponse({ error: 'The rewind could not be completed.' }, 500),
      )
    }
    return Promise.resolve(jsonResponse({}))
  })
  vi.stubGlobal('fetch', fetcher)

  function RewindHarness() {
    const [currentSession, setCurrentSession] = useState(originalSession)
    return (
      <ConversationPane
        key={currentSession.id}
        session={currentSession}
        refreshKey={0}
        sessionTreeRefreshKey={0}
        onSelectSession={(id) => {
          selectedSessions(id)
          setCurrentSession(forkedSession)
        }}
      />
    )
  }

  root = createRoot(container)
  act(() => {
    root?.render(
      <AsyncActionFeedbackProvider>
        <RewindHarness />
      </AsyncActionFeedbackProvider>,
    )
  })
  expect(
    await waitForState(
      () => container.querySelector('.chat-rewind-menu summary') !== null,
    ),
  ).toBe(true)
  act(() => {
    container.querySelector<HTMLElement>('.chat-rewind-menu summary')?.click()
  })
  await act(async () => {
    container
      .querySelector<HTMLButtonElement>('.chat-rewind-menu button')
      ?.click()
    await Promise.resolve()
  })
  expect(
    await waitForState(
      () =>
        container.querySelector('.async-action-toast-stack')?.textContent ===
        'Rewind failedChoose a retry policyThe rewind could not be completed.',
    ),
  ).toBe(true)

  const snapshot = () => ({
    selectedSessions: selectedSessions.mock.calls,
    rewindRequests: fetcher.mock.calls
      .filter(([, init]) => init?.method === 'POST')
      .map(([input, init]) => ({
        url: requestUrl(input),
        body:
          typeof init?.body === 'string'
            ? (JSON.parse(init.body) as unknown)
            : null,
      })),
    retryButtons: Array.from(
      container.querySelectorAll<HTMLButtonElement>(
        '.async-action-toast-stack .async-action-controls button',
      ),
      (button) => button.textContent,
    ),
    timelineOrder: Array.from(
      container.querySelectorAll(
        '.conversation-scroll .chat-assistant-message, .conversation-scroll .chat-user-group, .conversation-scroll .async-action-item--line',
      ),
      (element) =>
        element.matches('.async-action-item--line')
          ? 'rewind'
          : element.matches('.chat-user-group')
            ? 'selected request'
            : element.textContent,
    ),
    composer: container.querySelector<HTMLTextAreaElement>('textarea')?.value,
  })
  expect(snapshot()).toEqual({
    selectedSessions: [],
    rewindRequests: [
      {
        url: '/api/sessions/sample-session/rewind',
        body: {
          messageId: 'fixture-user-message',
          mode: 'conversation_and_code',
        },
      },
    ],
    retryButtons: [],
    timelineOrder: [
      'The request context is ready.',
      'rewind',
      'selected request',
      'I will continue with the follow-up.',
    ],
    composer: '',
  })
})

it('sends pasted file data with the message and keeps it when sending fails', async () => {
  const onAction = vi.fn(() => Promise.resolve(false))
  renderView(onAction, { ...session, id: crypto.randomUUID() })
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
        container.querySelector('.composer-attachment-name')?.textContent ===
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
      container.querySelectorAll('.composer-attachment-name'),
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

it('warns before leaving after its conversation pane unmounts', async () => {
  const sessionId = crypto.randomUUID()
  renderView(
    vi.fn(() => Promise.resolve(false)),
    {
      ...session,
      id: sessionId,
    },
  )
  const textarea = requireElement(
    container.querySelector<HTMLTextAreaElement>('textarea'),
  )
  const pasteEvent = new Event('paste', { bubbles: true, cancelable: true })
  Object.defineProperty(pasteEvent, 'clipboardData', {
    value: {
      files: [
        new File(['payload'], 'fixture-attachment.bin', {
          type: 'application/octet-stream',
        }),
      ],
    },
  })
  act(() => {
    textarea.dispatchEvent(pasteEvent)
  })
  expect(
    await waitForState(
      () => container.querySelector('.composer-attachment') !== null,
    ),
  ).toBe(true)
  const event = new Event('beforeunload', { cancelable: true })

  act(() => {
    root?.unmount()
    root = null
    window.dispatchEvent(event)
  })

  expect(event.defaultPrevented).toBe(true)
})

it('warns before leaving while an attachment is being processed', async () => {
  let resolveOptimization: ((file: File) => void) | undefined
  optimizeImageAttachmentMock.mockImplementationOnce(
    () =>
      new Promise((resolve) => {
        resolveOptimization = resolve
      }),
  )
  renderView(
    vi.fn(() => Promise.resolve(false)),
    {
      ...session,
      id: crypto.randomUUID(),
    },
  )
  const textarea = requireElement(
    container.querySelector<HTMLTextAreaElement>('textarea'),
  )
  const file = new File(['payload'], 'fixture-pending.bin', {
    type: 'application/octet-stream',
  })
  const pasteEvent = new Event('paste', { bubbles: true, cancelable: true })
  Object.defineProperty(pasteEvent, 'clipboardData', {
    value: { files: [file] },
  })
  act(() => {
    textarea.dispatchEvent(pasteEvent)
  })
  await waitForState(() => resolveOptimization !== undefined)

  const duringProcessing = new Event('beforeunload', { cancelable: true })
  act(() => {
    window.dispatchEvent(duringProcessing)
  })
  await act(async () => {
    if (resolveOptimization === undefined) {
      throw new Error('Expected attachment optimization to start')
    }
    resolveOptimization(file)
    await Promise.resolve()
  })
  await waitForState(
    () => container.querySelector('.composer-attachment') !== null,
  )
  const whileQueued = new Event('beforeunload', { cancelable: true })
  act(() => {
    window.dispatchEvent(whileQueued)
  })

  const snapshot = () => ({
    duringProcessing: duringProcessing.defaultPrevented,
    whileQueued: whileQueued.defaultPrevented,
  })
  expect(snapshot()).toEqual({ duringProcessing: true, whileQueued: true })
})

it('attaches dropped files through the composer and sends their data', async () => {
  const onAction = vi.fn(() => Promise.resolve(true))
  renderView(onAction, { ...session, id: crypto.randomUUID() })
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
        container
          .querySelector('.composer-attachment-image')
          ?.getAttribute('alt') === file.name,
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
      container.querySelectorAll('.composer-attachment-name'),
      (element) => element.textContent,
    ),
    actionCalls: onAction.mock.calls,
  })
  expect(snapshot()).toEqual({
    targetVisibleDuringDrag: 'Drop images to attach',
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

it('attaches files dropped on a proposal comment without activating the conversation drop target', async () => {
  const onAction = vi.fn(() => Promise.resolve(true))
  renderView(onAction, { ...session, id: crypto.randomUUID() }, undefined, [
    proposal,
  ])
  const file = new File(['payload'], 'dropped.png', { type: 'image/png' })
  const dataTransfer = { types: ['Files'], files: [file], dropEffect: 'none' }
  const comment = requireElement(
    container.querySelector<HTMLTextAreaElement>('.chat-proposal-comment'),
  )
  const dispatchFileEvent = (
    type: 'dragenter' | 'dragleave' | 'dragover' | 'drop',
  ) => {
    const event = new Event(type, { bubbles: true, cancelable: true })
    Object.defineProperty(event, 'dataTransfer', { value: dataTransfer })
    act(() => {
      comment.dispatchEvent(event)
    })
    return event
  }

  const dragEnter = dispatchFileEvent('dragenter')
  const targetAfterDragEnter = container.querySelector(
    '.chat-proposal-card .file-drop-indicator',
  )
  const dragLeave = dispatchFileEvent('dragleave')
  const targetAfterDragLeave = container.querySelector(
    '.chat-proposal-card .file-drop-indicator',
  )
  dispatchFileEvent('dragenter')
  const dragOver = dispatchFileEvent('dragover')
  const targetAfterDragOver = container.querySelector(
    '.chat-proposal-card .file-drop-indicator',
  )
  const drop = dispatchFileEvent('drop')
  expect(
    await waitForState(
      () =>
        container.querySelector(
          '.chat-proposal-card .attachment-preview-item span',
        )?.textContent === file.name,
    ),
  ).toBe(true)

  const snapshot = () => ({
    dragEnterPrevented: dragEnter.defaultPrevented,
    dragLeavePrevented: dragLeave.defaultPrevented,
    dragOverPrevented: dragOver.defaultPrevented,
    dropPrevented: drop.defaultPrevented,
    targetAfterDragEnter: targetAfterDragEnter?.textContent ?? null,
    targetAfterDragLeave: targetAfterDragLeave?.textContent ?? null,
    targetAfterDragOver: targetAfterDragOver?.textContent ?? null,
    targetAfterDrop:
      container.querySelector('.conversation-drop-target')?.textContent ?? null,
    commentAttachmentNames: Array.from(
      container.querySelectorAll(
        '.chat-proposal-card .attachment-preview-item span',
      ),
      (element) => element.textContent,
    ),
    composerAttachmentCount: container.querySelectorAll(
      '.conversation-composer .composer-attachment',
    ).length,
    actionCalls: onAction.mock.calls,
  })
  expect(snapshot()).toEqual({
    dragEnterPrevented: true,
    dragLeavePrevented: false,
    dragOverPrevented: true,
    dropPrevented: true,
    targetAfterDragEnter: 'Drop files to attach',
    targetAfterDragLeave: null,
    targetAfterDragOver: 'Drop files to attach',
    targetAfterDrop: null,
    commentAttachmentNames: ['dropped.png'],
    composerAttachmentCount: 0,
    actionCalls: [],
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
    targetAfterEnter: 'Drop images to attach',
    targetAfterLeave: null,
  })
})

it('clears pasted attachments after a successful send', async () => {
  const onAction = vi.fn(() => Promise.resolve(true))
  renderView(onAction, { ...session, id: crypto.randomUUID() })
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
        container.querySelector('.composer-attachment-name')?.textContent ===
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
      container.querySelectorAll('.composer-attachment-name'),
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
  renderView(onAction, { ...session, id: crypto.randomUUID() })
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
      container.querySelectorAll('.composer-attachment-name'),
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
  renderView(onAction, { ...session, id: crypto.randomUUID() })
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
      container.querySelectorAll('.composer-attachment-name'),
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
  const currentSession = { ...session, id: crypto.randomUUID() }
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
        rewind: false,
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
        session={currentSession}
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
        container.querySelector('.composer-attachment-name')?.textContent ===
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
        container.querySelector('.composer-attachment-name')?.textContent ===
          file.name,
    ),
  ).toBe(true)

  const snapshot = () => ({
    error: container.querySelector('[role="alert"]')?.textContent ?? null,
    attachmentName: container.querySelector('.composer-attachment-name')
      ?.textContent,
    postBody: normalizeQueuedMessagePostBody(fetcher.mock.calls[2]?.[1]?.body),
  })
  expect(snapshot()).toEqual({
    error: 'file attachment data is invalid',
    attachmentName: 'document.pdf',
    postBody: JSON.stringify({
      content: '',
      messageId: '<message-id>',
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
  const currentSession = { ...session, id: crypto.randomUUID() }
  const fetcher = vi.fn<typeof fetch>((input) =>
    Promise.resolve(
      requestUrl(input).endsWith('/actions')
        ? jsonResponse({
            message: true,
            answerQuestion: false,
            interrupt: false,
            openInTerminal: true,
            rewind: false,
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
        session={currentSession}
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
        container
          .querySelector('.composer-attachment-image')
          ?.getAttribute('alt') === 'photo.jpg' && !sendButton.disabled,
    ),
  ).toBe(true)
  act(() => {
    sendButton.click()
  })
  expect(
    await waitForState(() => fetcher.mock.calls[2]?.[1]?.body !== undefined),
  ).toBe(true)

  const sentBody = () => fetcher.mock.calls[2]?.[1]?.body
  expect(normalizeQueuedMessagePostBody(sentBody())).toEqual(
    JSON.stringify({
      content: 'Inspect this photo.',
      messageId: '<message-id>',
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

it('sends a message while session actions are still loading', async () => {
  const onAction = vi.fn(() => Promise.resolve(true))
  renderView(onAction, session, undefined, [], null)
  const textarea = requireElement(
    container.querySelector<HTMLTextAreaElement>('textarea'),
  )
  const sendButton = requireElement(
    container.querySelector<HTMLButtonElement>('[aria-label="Send message"]'),
  )
  const beforeTyping = {
    composerDisabled: textarea.disabled,
    sendDisabled: sendButton.disabled,
  }
  setTextareaValue(textarea, 'Send while actions load')
  const beforeSending = {
    composerDisabled: textarea.disabled,
    sendDisabled: sendButton.disabled,
  }
  act(() => {
    dispatchKey(textarea, 'Enter')
  })
  await act(async () => {
    await Promise.resolve()
  })

  const snapshot = () => ({
    beforeTyping,
    beforeSending,
    afterSending: {
      draft: textarea.value,
      actionCalls: onAction.mock.calls,
    },
  })
  expect(snapshot()).toEqual({
    beforeTyping: { composerDisabled: false, sendDisabled: true },
    beforeSending: { composerDisabled: false, sendDisabled: false },
    afterSending: {
      draft: '',
      actionCalls: [['messages', { content: 'Send while actions load' }]],
    },
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
  const beforeUnload = new Event('beforeunload', { cancelable: true })
  act(() => {
    window.dispatchEvent(beforeUnload)
  })

  const snapshot = () => ({
    draft: textarea.value,
    composerVisible: textarea.isConnected,
    unloadPrevented: beforeUnload.defaultPrevented,
    storageNotice: container.querySelector('.conversation-error[role="status"]')
      ?.textContent,
  })
  expect(snapshot()).toEqual({
    draft: 'Keep editing in memory',
    composerVisible: true,
    unloadPrevented: true,
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
    status: container.querySelector('.composer-status')?.textContent ?? null,
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
      status: container.querySelector('.composer-status')?.textContent ?? null,
    },
  })
  const expected = {
    duringSend: {
      actionCalls: [['messages', { content: 'Send once' }]],
      buttonDisabled: true,
      defaultPrevented: [true, true],
      draft: '',
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
  const setHeight = mockScrollDimensions(scroll, 300, 100)
  renderView(onAction, session, undefined, [firstMessage])
  scrollToPosition(scroll, 200)
  const initialBottom = scroll.scrollTop

  act(() => {
    scroll.dispatchEvent(new Event('wheel'))
  })
  setHeight(500)
  renderView(onAction, session, undefined, [
    firstMessage,
    { type: 'assistant', text: 'Second message' },
  ])
  const afterNewMessage = scroll.scrollTop

  act(() => {
    scroll.scrollTop = 40
    scroll.dispatchEvent(new Event('scroll'))
  })
  setHeight(600)
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
  setHeight(700)
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

it('keeps the answered proposal in place as the conversation continues', () => {
  const onAction = vi.fn(() => Promise.resolve(true))
  const firstMessage: ChatItem = {
    type: 'assistant',
    text: 'Earlier message',
  }
  const latestMessage: ChatItem = {
    type: 'assistant',
    text: 'Latest message',
  }
  const pendingProposal: ChatItem = {
    type: 'proposal',
    seq: 1,
    title: 'Choose a response',
    summary: null,
    options: [
      {
        label: 'Option A',
        description: null,
        pros: [],
        cons: [],
        recommended: false,
      },
    ],
    answer: null,
    resolved: false,
    resolution: null,
    comment: null,
  }
  const resolvedProposal: ChatItem = {
    ...pendingProposal,
    answer: { choice: 0, comment: null },
    resolved: true,
  }
  renderView(onAction, session, undefined, [
    firstMessage,
    latestMessage,
    pendingProposal,
  ])
  const scroll = requireElement(
    container.querySelector<HTMLDivElement>('.conversation-scroll'),
  )
  const setHeight = mockScrollDimensions(scroll, 300, 100)
  renderView(onAction, session, undefined, [
    firstMessage,
    latestMessage,
    pendingProposal,
  ])
  scrollToPosition(scroll, 100)
  const proposalRow = requireElement(
    scroll.querySelector<HTMLElement>('.chat-proposal-card'),
  )

  setHeight(500)
  renderView(onAction, session, undefined, [
    firstMessage,
    latestMessage,
    resolvedProposal,
    { type: 'assistant', text: 'The answer is being used' },
  ])

  const snapshot = () => ({
    scrollTop: scroll.scrollTop,
    proposalPosition: Array.from(scroll.children).indexOf(proposalRow),
    proposalPreserved:
      scroll.querySelector('.chat-proposal-card') === proposalRow,
  })
  expect(snapshot()).toEqual({
    scrollTop: 100,
    proposalPosition: 2,
    proposalPreserved: true,
  })
})

it('follows the bottom when an uncached conversation with a pending card loads', async () => {
  const pendingProposal: ChatItem = {
    ...proposal,
    seq: 64,
    title: 'Choose a recovery strategy',
    summary: 'Review the available recovery steps.',
  }
  const firstPageMessages: ChatItem[] = [
    ...Array.from({ length: 12 }, () => ({
      type: 'assistant' as const,
      text: 'Earlier history fills the conversation. '.repeat(10),
    })),
    pendingProposal,
  ]
  let resolveMessages: ((response: Response) => void) | undefined
  const firstPageResponse = new Promise<Response>((resolve) => {
    resolveMessages = resolve
  })
  const fetcher = vi.fn<typeof fetch>((input) => {
    const url = requestUrl(input)
    if (url.endsWith('/messages?limit=100')) {
      return firstPageResponse
    }
    if (url.endsWith('/actions')) return Promise.resolve(jsonResponse(actions))
    return Promise.resolve(jsonResponse({}))
  })
  vi.stubGlobal('fetch', fetcher)
  root = createRoot(container)
  act(() => {
    root?.render(
      <ConversationPane
        session={session}
        shouldSkipSessionTreeFetch
        initialSessionTree={[]}
        refreshKey={0}
        sessionTreeRefreshKey={0}
        onSelectSession={() => undefined}
      />,
    )
  })
  await waitForState(() => resolveMessages !== undefined)
  const scroll = requireElement(
    container.querySelector<HTMLDivElement>('.conversation-scroll'),
  )
  const setHeight = mockScrollDimensions(scroll, 100, 100)
  setHeight(500)

  await act(async () => {
    resolveMessages?.(
      jsonResponse(
        messagesResponse(firstPageMessages, [], {
          start: 0,
          total: firstPageMessages.length,
          epoch: 'fixture-epoch-amber',
        }),
      ),
    )
    await firstPageResponse
  })
  await waitForState(
    () => container.querySelector('.chat-proposal-card') !== null,
  )

  const snapshot = () => ({ scrollTop: scroll.scrollTop })
  expect(snapshot()).toEqual({ scrollTop: 400 })
})

it('keeps a full-height resolved todo at the bottom without spacer padding', () => {
  const earlierContext: ChatItem = {
    type: 'assistant',
    text: 'Earlier timeline context. '.repeat(40),
  }
  const pendingTodo: ChatItem = {
    type: 'todo',
    seq: 64,
    title: 'Review the recovery plan',
    detail: 'Review the recovery steps before proceeding. '.repeat(40),
    completed: false,
    resolution: null,
    comment: null,
  }
  const messages = [earlierContext, pendingTodo]
  renderConversationMessages(messages, 5)
  const scroll = requireElement(
    container.querySelector<HTMLDivElement>('.conversation-scroll'),
  )
  scroll.style.height = '360px'
  scroll.style.flex = 'none'
  renderConversationMessages(messages.slice(), 5)
  act(() => {
    requireElement(
      scroll.querySelector<HTMLButtonElement>('.collapsible-text__toggle'),
    ).click()
  })
  scrollToPosition(scroll, scroll.scrollHeight - scroll.clientHeight)
  const pendingCardRect = requireElement(
    scroll.querySelector<HTMLElement>('.chat-todo-card'),
  ).getBoundingClientRect()
  const earlierContextVisible = (() => {
    const rect = requireElement(
      scroll.querySelector<HTMLElement>('.chat-assistant-message'),
    ).getBoundingClientRect()
    const bounds = scroll.getBoundingClientRect()
    return rect.top < bounds.bottom && rect.bottom > bounds.top
  })()
  const initialState = {
    todoFillsViewport: pendingCardRect.height >= scroll.clientHeight,
    todoVisible:
      pendingCardRect.top < scroll.getBoundingClientRect().bottom &&
      pendingCardRect.bottom > scroll.getBoundingClientRect().top,
    earlierContextVisible,
    atBottom:
      scroll.scrollHeight - scroll.scrollTop - scroll.clientHeight <= 24,
  }

  const resolvedTodo: ChatItem = {
    ...pendingTodo,
    completed: true,
    resolution: 'completed',
  }
  renderConversationMessages([earlierContext, resolvedTodo], 5)

  const card = requireElement(
    scroll.querySelector<HTMLElement>('.chat-todo-card'),
  )
  const bounds = scroll.getBoundingClientRect()
  const paddingBottom = Number.parseFloat(
    window.getComputedStyle(scroll).paddingBottom,
  )
  const rowGap = Number.parseFloat(window.getComputedStyle(scroll).rowGap)
  const snapshot = () => ({
    cardVisible:
      card.getBoundingClientRect().top < bounds.bottom &&
      card.getBoundingClientRect().bottom > bounds.top,
    blankSpaceAfterCard: Math.round(
      bounds.bottom -
        paddingBottom -
        rowGap -
        card.getBoundingClientRect().bottom,
    ),
    hasExtraPadding:
      scroll.style.getPropertyValue(
        '--conversation-anchor-extra-padding-bottom',
      ) !== '',
  })
  const result = () => ({ initialState, afterResolution: snapshot() })
  const expected = {
    initialState: {
      todoFillsViewport: true,
      todoVisible: true,
      earlierContextVisible: false,
      atBottom: true,
    },
    afterResolution: {
      cardVisible: true,
      blankSpaceAfterCard: 0,
      hasExtraPadding: false,
    },
  }
  expect(result()).toEqual(expected)
})

it('preserves visible rows when clearing compensation above the scroll limit', () => {
  const scroll = document.createElement('div')
  scroll.style.cssText =
    'display: flex; flex-direction: column; height: 200px; width: 400px; overflow-y: auto'
  const children = Array.from({ length: 10 }, (_, index) => {
    const child = document.createElement('div')
    child.style.cssText =
      'box-sizing: border-box; flex: 0 0 100px; height: 100px'
    child.textContent = `Timeline row ${String(index)}`
    scroll.append(child)
    return child
  })
  const compensatedChild = children[1]
  if (compensatedChild === undefined) throw new Error('Expected a timeline row')
  compensatedChild.style.marginBlockStart = '120px'
  const anchorOffsets = new Map<HTMLElement, ConversationAnchorSpacing>([
    [
      compensatedChild,
      {
        offset: 120,
        baseMarginBlockStart: 0,
        originalMarginBlockStart: '',
      },
    ],
  ])
  container.append(scroll)

  scroll.scrollTop = scroll.scrollHeight - scroll.clientHeight - 30
  const bounds = scroll.getBoundingClientRect()
  const visibleBefore = children.flatMap((child) => {
    const rect = child.getBoundingClientRect()
    return rect.top < bounds.bottom && rect.bottom > bounds.top
      ? [{ element: child, top: rect.top }]
      : []
  })
  const compensationOffscreenBefore =
    compensatedChild.getBoundingClientRect().bottom <= bounds.top
  const distanceFromScrollLimit =
    scroll.scrollHeight - scroll.scrollTop - scroll.clientHeight
  clearOffscreenConversationScrollCompensation(
    scroll,
    readConversationChildLayout(scroll),
    anchorOffsets,
    0,
    new Set(),
    false,
    (element, scrollTop) => {
      element.scrollTop = scrollTop
    },
  )
  const visibleAfter = new Map(
    children.flatMap((child) => {
      const rect = child.getBoundingClientRect()
      return rect.top < bounds.bottom && rect.bottom > bounds.top
        ? [[child, rect.top] as const]
        : []
    }),
  )

  const actual = () => ({
    compensationOffscreenBefore,
    distanceFromScrollLimit,
    compensationCleared:
      compensatedChild.style.marginBlockStart === '' &&
      anchorOffsets.size === 0,
    visibleRowsBefore: visibleBefore.map(({ element }) =>
      children.indexOf(element),
    ),
    visibleRowsStillVisible: visibleBefore.map(({ element }) =>
      visibleAfter.has(element),
    ),
    visibleRowTopDeltas: visibleBefore.map(({ element, top }) =>
      Math.round((visibleAfter.get(element) ?? top) - top),
    ),
  })
  const expected = {
    compensationOffscreenBefore: true,
    distanceFromScrollLimit: 30,
    compensationCleared: true,
    visibleRowsBefore: [7, 8, 9],
    visibleRowsStillVisible: [true, true, true],
    visibleRowTopDeltas: [0, 0, 0],
  }
  expect(actual()).toEqual(expected)
})

it('keeps following when a hidden active tool becomes visible', () => {
  const hiddenTool: ChatItem = {
    type: 'tool_in_progress',
    tool: 'exec',
    target: 'Run the build command.',
  }
  const visibleTool: ChatItem = {
    type: 'tool_summary',
    summary: '1 action',
    operations: [
      { tool: 'exec', target: 'Run the build command.', failed: false },
    ],
  }
  const onAction = vi.fn(() => Promise.resolve(true))
  renderView(onAction, { ...session, category: 'in_progress' }, undefined, [
    hiddenTool,
  ])
  const scroll = requireElement(
    container.querySelector<HTMLDivElement>('.conversation-scroll'),
  )
  const setHeight = mockScrollDimensions(scroll, 300, 100)
  renderView(onAction, { ...session, category: 'in_progress' }, undefined, [
    hiddenTool,
  ])
  scrollToPosition(scroll, 200)

  setHeight(500)
  renderView(onAction, { ...session, category: 'needs_action' }, undefined, [
    visibleTool,
  ])
  const afterToolBecomesVisible = scroll.scrollTop

  setHeight(600)
  renderView(onAction, { ...session, category: 'needs_action' }, undefined, [
    visibleTool,
    { type: 'assistant', text: 'The next step is ready.' },
  ])

  const snapshot = () => ({
    afterToolBecomesVisible,
    afterNewMessage: scroll.scrollTop,
  })
  expect(snapshot()).toEqual({
    afterToolBecomesVisible: 400,
    afterNewMessage: 500,
  })
})

it('keeps an expanded to-do in view when a new message arrives', () => {
  const notifyResize = stubResizeObserver()
  const todo: ChatItem = {
    type: 'todo',
    seq: 12,
    title: 'Review the deployment sequence',
    detail: Array.from(
      { length: 16 },
      () => 'Check the deployment sequence before moving to the next step.',
    ).join('\n\n'),
    completed: false,
    resolution: null,
    comment: null,
  }
  const onAction = vi.fn(() => Promise.resolve(true))
  renderView(onAction, session, undefined, [todo])
  const scroll = requireElement(
    container.querySelector<HTMLDivElement>('.conversation-scroll'),
  )
  const setHeight = mockScrollDimensions(scroll, 300, 100)
  renderView(onAction, session, undefined, [todo])
  scrollToPosition(scroll, 200)
  const initialBottom = scroll.scrollTop
  const messageElement = requireElement(scroll.lastElementChild)
  act(() => {
    notifyResize(messageElement)
  })

  const expandButton = requireElement(
    container.querySelector<HTMLButtonElement>(
      '.collapsible-text__toggle[aria-expanded="false"]',
    ),
  )
  act(() => {
    expandButton.click()
  })
  setHeight(600)
  act(() => {
    notifyResize(
      messageElement,
      messageElement.getBoundingClientRect().height + 10,
    )
  })
  const afterExpanding = scroll.scrollTop

  setHeight(800)
  renderView(onAction, session, undefined, [
    todo,
    { type: 'assistant', text: 'The next step is ready.' },
  ])

  const snapshot = () => ({
    initialBottom,
    afterExpanding,
    afterNewMessage: scroll.scrollTop,
  })
  expect(snapshot()).toEqual({
    initialBottom: 200,
    afterExpanding: 200,
    afterNewMessage: 200,
  })
})

it.each([
  {
    card: 'to-do',
    original: {
      type: 'todo',
      seq: 12,
      title: 'Review the deployment sequence',
      detail: null,
      completed: false,
      resolution: null,
      comment: null,
    } satisfies ChatItem,
    updated: {
      type: 'todo',
      seq: 12,
      title: 'Review the deployment sequence',
      detail: null,
      folded: true,
      completed: false,
      resolution: null,
      comment: null,
    } satisfies ChatItem,
  },
  {
    card: 'to-do',
    original: {
      type: 'todo',
      seq: 12,
      title: 'Review the deployment sequence',
      detail: null,
      completed: false,
      resolution: null,
      comment: null,
    } satisfies ChatItem,
    updated: {
      type: 'todo',
      seq: 12,
      title: 'Review the deployment sequence',
      detail: null,
      completed: true,
      resolution: 'completed',
      comment: null,
    } satisfies ChatItem,
  },
  {
    card: 'proposal',
    original: proposal,
    updated: {
      ...proposal,
      answer: { choice: 0, comment: null },
      resolved: true,
    },
  },
])(
  'keeps the scroll position when a $card is updated',
  ({ original, updated }) => {
    const onAction = vi.fn(() => Promise.resolve(true))
    renderView(onAction, session, undefined, [original])
    const scroll = requireElement(
      container.querySelector<HTMLDivElement>('.conversation-scroll'),
    )
    const setHeight = mockScrollDimensions(scroll, 300, 100)
    renderView(onAction, session, undefined, [original])
    scrollToPosition(scroll, 200)
    act(() => {
      requireElement(scroll.firstElementChild).dispatchEvent(
        new Event('click', { bubbles: true }),
      )
    })

    setHeight(500)
    renderView(onAction, session, undefined, [updated])

    const snapshot = () => ({ scrollTop: scroll.scrollTop })
    expect(snapshot()).toEqual({ scrollTop: 200 })
  },
)

it('keeps following the bottom when a message grows after it is added', async () => {
  const notifyResize = stubResizeObserver()
  const onAction = vi.fn(() => Promise.resolve(true))
  const firstMessage: ChatItem = { type: 'assistant', text: 'First message' }
  renderView(onAction, session, undefined, [firstMessage])
  const scroll = requireElement(
    container.querySelector<HTMLDivElement>('.conversation-scroll'),
  )
  const setHeight = mockScrollDimensions(scroll, 300, 100)
  renderView(onAction, session, undefined, [firstMessage])
  scrollToPosition(scroll, 200)

  setHeight(500)
  renderView(onAction, session, undefined, [
    firstMessage,
    { type: 'assistant', text: 'A message with an image' },
  ])
  await act(async () => {
    await Promise.resolve()
  })
  const afterNewMessage = scroll.scrollTop
  const lastMessage = requireElement(scroll.lastElementChild)
  setHeight(700)
  act(() => {
    notifyResize(lastMessage, lastMessage.getBoundingClientRect().height + 10)
  })

  const snapshot = () => ({
    afterNewMessage,
    afterMessageGrowth: scroll.scrollTop,
  })
  expect(snapshot()).toEqual({
    afterNewMessage: 400,
    afterMessageGrowth: 600,
  })
})

it('keeps a native comment resize in place while the pointer is down', () => {
  const notifyResize = stubResizeObserver()
  const todo: ChatItem = {
    type: 'todo',
    seq: 21,
    title: 'Review the release checklist',
    detail: null,
    completed: false,
    resolution: null,
    comment: null,
  }
  const onAction = vi.fn(() => Promise.resolve(true))
  renderView(onAction, session, undefined, [todo])
  const scroll = requireElement(
    container.querySelector<HTMLDivElement>('.conversation-scroll'),
  )
  const setHeight = mockScrollDimensions(scroll, 300, 100)
  renderView(onAction, session, undefined, [todo])
  scrollToPosition(scroll, 200)
  const messageElement = requireElement(scroll.lastElementChild)
  act(() => {
    notifyResize(messageElement)
  })

  const comment = requireElement(
    container.querySelector<HTMLTextAreaElement>('.chat-todo-comment'),
  )
  act(() => {
    comment.dispatchEvent(new Event('pointerdown', { bubbles: true }))
  })
  setHeight(320)
  comment.style.height = '60px'
  act(() => {
    notifyResize(
      messageElement,
      messageElement.getBoundingClientRect().height + 10,
    )
  })
  setHeight(360)
  comment.style.height = '80px'
  act(() => {
    notifyResize(
      messageElement,
      messageElement.getBoundingClientRect().height + 20,
    )
  })
  act(() => {
    window.dispatchEvent(new Event('pointerup'))
  })
  setHeight(380)
  comment.style.height = '90px'
  act(() => {
    notifyResize(
      messageElement,
      messageElement.getBoundingClientRect().height + 30,
    )
  })

  const snapshot = () => ({ scrollTop: scroll.scrollTop })
  expect(snapshot()).toEqual({ scrollTop: 200 })
})

it('keeps following after a textarea text-selection gesture', () => {
  const todo: ChatItem = {
    type: 'todo',
    seq: 22,
    title: 'Review the release checklist',
    detail: null,
    completed: false,
    resolution: null,
    comment: null,
  }
  const onAction = vi.fn(() => Promise.resolve(true))
  renderView(onAction, session, undefined, [todo])
  const scroll = requireElement(
    container.querySelector<HTMLDivElement>('.conversation-scroll'),
  )
  const setHeight = mockScrollDimensions(scroll, 300, 100)
  renderView(onAction, session, undefined, [todo])
  scrollToPosition(scroll, 200)

  const comment = requireElement(
    container.querySelector<HTMLTextAreaElement>('.chat-todo-comment'),
  )
  act(() => {
    comment.dispatchEvent(new Event('pointerdown', { bubbles: true }))
  })
  setHeight(500)
  renderView(onAction, session, undefined, [
    todo,
    { type: 'assistant', text: 'The next step is ready.' },
  ])
  act(() => {
    window.dispatchEvent(new Event('pointerup'))
  })

  const snapshot = () => ({ scrollTop: scroll.scrollTop })
  expect(snapshot()).toEqual({ scrollTop: 400 })
})

it('keeps a reader in place when an observed message grows', () => {
  const notifyResize = stubResizeObserver()
  const onAction = vi.fn(() => Promise.resolve(true))
  const message: ChatItem = { type: 'assistant', text: 'A long message' }
  renderView(onAction, session, undefined, [message])
  const scroll = requireElement(
    container.querySelector<HTMLDivElement>('.conversation-scroll'),
  )
  const setHeight = mockScrollDimensions(scroll, 300, 100)
  renderView(onAction, session, undefined, [message])
  scrollToPosition(scroll, 40)
  const lastMessage = requireElement(scroll.lastElementChild)
  act(() => {
    notifyResize(lastMessage)
  })

  setHeight(450)
  act(() => {
    notifyResize(lastMessage, lastMessage.getBoundingClientRect().height + 10)
  })

  const snapshot = () => ({ scrollTop: scroll.scrollTop })
  expect(snapshot()).toEqual({ scrollTop: 40 })
})

it('keeps an attached proposal in place when a new message arrives', async () => {
  const notifyResize = stubResizeObserver()
  const onAction = vi.fn(() => Promise.resolve(true))
  renderView(onAction, session, undefined, [proposal])
  const scroll = requireElement(
    container.querySelector<HTMLDivElement>('.conversation-scroll'),
  )
  const setHeight = mockScrollDimensions(scroll, 300, 100)
  renderView(onAction, session, undefined, [proposal])
  scrollToPosition(scroll, 200)
  const lastMessage = requireElement(scroll.lastElementChild)
  act(() => {
    notifyResize(lastMessage)
  })

  const comment = requireElement(
    container.querySelector<HTMLTextAreaElement>('.chat-proposal-comment'),
  )
  const dropEvent = new Event('drop', { bubbles: true, cancelable: true })
  Object.defineProperty(dropEvent, 'dataTransfer', {
    value: {
      types: ['Files'],
      files: [new File(['payload'], 'fixture-attachment.bin')],
      dropEffect: 'none',
    },
  })
  act(() => {
    comment.dispatchEvent(dropEvent)
  })
  expect(
    await waitForState(
      () =>
        container.querySelector(
          '.chat-proposal-card .attachment-preview-item span',
        )?.textContent === 'fixture-attachment.bin',
    ),
  ).toBe(true)

  setHeight(600)
  act(() => {
    notifyResize(lastMessage, lastMessage.getBoundingClientRect().height + 10)
  })
  const afterAttachment = scroll.scrollTop
  setHeight(800)
  renderView(onAction, session, undefined, [
    proposal,
    { type: 'assistant', text: 'The next step is ready.' },
  ])

  const snapshot = () => ({
    attachmentName: container.querySelector(
      '.chat-proposal-card .attachment-preview-item span',
    )?.textContent,
    afterAttachment,
    afterNewMessage: scroll.scrollTop,
  })
  expect(snapshot()).toEqual({
    attachmentName: 'fixture-attachment.bin',
    afterAttachment: 200,
    afterNewMessage: 200,
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
        rewind: false,
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

it('keeps a message send error through refreshes until a send succeeds', async () => {
  let refreshConversation: () => void = () => undefined
  let sendCount = 0
  const fetcher = vi.fn<typeof fetch>((input, init) => {
    const url = requestUrl(input)
    if (init?.method === 'POST' && url.endsWith('/messages')) {
      sendCount += 1
      return Promise.resolve(
        sendCount === 1
          ? jsonResponse({ error: 'The message could not be sent.' }, 503)
          : jsonResponse({ queuedPromptId: 'fixture-queued-prompt' }),
      )
    }
    if (url.includes('/messages?')) {
      return Promise.resolve(jsonResponse(messagesResponse([permission])))
    }
    if (url.endsWith('/actions')) return Promise.resolve(jsonResponse(actions))
    return Promise.resolve(jsonResponse({}))
  })
  const messageRefreshCount = () =>
    fetcher.mock.calls.filter(
      ([input, init]) =>
        init?.method !== 'POST' && requestUrl(input).includes('/messages?'),
    ).length
  vi.stubGlobal('fetch', fetcher)

  function ConversationHarness() {
    const [refreshKey, setRefreshKey] = useState(0)
    refreshConversation = () => {
      setRefreshKey((current) => current + 1)
    }
    return (
      <ConversationPane
        session={session}
        shouldSkipSessionTreeFetch
        initialSessionTree={[]}
        refreshKey={refreshKey}
        sessionTreeRefreshKey={0}
        onSelectSession={() => undefined}
      />
    )
  }

  root = createRoot(container)
  act(() => {
    root?.render(
      <SessionConversationCacheProvider>
        <ConversationHarness />
      </SessionConversationCacheProvider>,
    )
  })
  await waitForState(
    () =>
      container.querySelector<HTMLTextAreaElement>('textarea')?.disabled ===
      false,
  )
  const textarea = requireElement(
    container.querySelector<HTMLTextAreaElement>('textarea'),
  )
  setTextareaValue(textarea, 'Retain this prompt')
  await act(async () => {
    dispatchKey(textarea, 'Enter')
    for (let index = 0; index < 12; index += 1) await Promise.resolve()
  })
  await waitForState(
    () =>
      container.querySelector('[role="alert"]')?.textContent ===
      'The message could not be sent.',
  )
  const afterFailedSend = {
    alert: container.querySelector('[role="alert"]')?.textContent ?? null,
    draft: textarea.value,
  }

  await act(async () => {
    await userEvent.click(
      requireElement(
        container.querySelector<HTMLButtonElement>(
          '.permission-actions button',
        ),
      ),
    )
  })
  const unrelatedActionRefreshCompleted = await waitForState(
    () => messageRefreshCount() === 2,
  )
  const afterUnrelatedAction = {
    completed: unrelatedActionRefreshCompleted,
    alert: container.querySelector('[role="alert"]')?.textContent ?? null,
    draft: textarea.value,
  }

  const refreshCountBeforeExplicitRefresh = messageRefreshCount()
  await act(async () => {
    refreshConversation()
    for (let index = 0; index < 12; index += 1) await Promise.resolve()
  })
  const explicitRefreshCompleted = await waitForState(
    () => messageRefreshCount() > refreshCountBeforeExplicitRefresh,
  )
  const afterRefresh = {
    completed: explicitRefreshCompleted,
    alert: container.querySelector('[role="alert"]')?.textContent ?? null,
    draft: textarea.value,
  }

  await act(async () => {
    dispatchKey(textarea, 'Enter')
    for (let index = 0; index < 12; index += 1) await Promise.resolve()
  })
  await waitForState(
    () => sendCount === 2 && container.querySelector('[role="alert"]') === null,
  )

  const snapshot = () => ({
    afterFailedSend,
    afterUnrelatedAction,
    afterRefresh,
    afterSuccessfulSend: {
      alert: container.querySelector('[role="alert"]')?.textContent ?? null,
      draft: textarea.value,
    },
  })
  expect(snapshot()).toEqual({
    afterFailedSend: {
      alert: 'The message could not be sent.',
      draft: 'Retain this prompt',
    },
    afterUnrelatedAction: {
      completed: true,
      alert: 'The message could not be sent.',
      draft: 'Retain this prompt',
    },
    afterRefresh: {
      completed: true,
      alert: 'The message could not be sent.',
      draft: 'Retain this prompt',
    },
    afterSuccessfulSend: { alert: null, draft: '' },
  })
})

it('shows permission errors alongside a retained message send error', async () => {
  let sendCount = 0
  let permissionRequestCount = 0
  const fetcher = vi.fn<typeof fetch>((input, init) => {
    const url = requestUrl(input)
    if (init?.method === 'POST' && url.endsWith('/messages')) {
      sendCount += 1
      return Promise.resolve(
        sendCount === 1
          ? jsonResponse({ error: 'The message could not be sent.' }, 503)
          : jsonResponse({ queuedPromptId: 'fixture-queued-prompt' }),
      )
    }
    if (init?.method === 'POST' && url.endsWith('/permission/approve')) {
      permissionRequestCount += 1
      return Promise.resolve(
        permissionRequestCount === 1
          ? jsonResponse(
              { error: 'The permission could not be approved.' },
              503,
            )
          : jsonResponse({}),
      )
    }
    if (url.includes('/messages?')) {
      return Promise.resolve(jsonResponse(messagesResponse([permission])))
    }
    if (url.endsWith('/actions')) return Promise.resolve(jsonResponse(actions))
    return Promise.resolve(jsonResponse({}))
  })
  vi.stubGlobal('fetch', fetcher)

  root = createRoot(container)
  act(() => {
    root?.render(
      <SessionConversationCacheProvider>
        <ConversationPane
          session={session}
          shouldSkipSessionTreeFetch
          initialSessionTree={[]}
          refreshKey={0}
          sessionTreeRefreshKey={0}
          onSelectSession={() => undefined}
        />
      </SessionConversationCacheProvider>,
    )
  })
  await waitForState(
    () =>
      container.querySelector<HTMLTextAreaElement>('textarea')?.disabled ===
      false,
  )
  const textarea = requireElement(
    container.querySelector<HTMLTextAreaElement>('textarea'),
  )
  setTextareaValue(textarea, 'Retain this prompt')
  await act(async () => {
    dispatchKey(textarea, 'Enter')
    for (let index = 0; index < 12; index += 1) await Promise.resolve()
  })
  await waitForState(
    () =>
      container.querySelector('[role="alert"]')?.textContent ===
      'The message could not be sent.',
  )

  const approveButton = () =>
    requireElement(
      container.querySelector<HTMLButtonElement>(
        '.chat-permission-card .permission-actions button',
      ),
    )
  await act(async () => userEvent.click(approveButton()))
  await waitForState(
    () =>
      permissionRequestCount === 1 &&
      container.querySelector('[role="alert"]')?.textContent ===
        'The permission could not be approved.',
  )
  const afterFailedApproval =
    container.querySelector('[role="alert"]')?.textContent ?? null

  await act(async () => userEvent.click(approveButton()))
  await waitForState(
    () =>
      permissionRequestCount === 2 &&
      container.querySelector('[role="alert"]')?.textContent ===
        'The message could not be sent.',
  )
  const afterSuccessfulApproval =
    container.querySelector('[role="alert"]')?.textContent ?? null

  await act(async () => {
    dispatchKey(textarea, 'Enter')
    for (let index = 0; index < 12; index += 1) await Promise.resolve()
  })
  const successfulSendCompleted = await waitForState(
    () => sendCount === 2 && container.querySelector('[role="alert"]') === null,
  )

  const snapshot = () => ({
    afterFailedApproval,
    afterSuccessfulApproval,
    successfulSendCompleted,
  })
  expect(snapshot()).toEqual({
    afterFailedApproval: 'The permission could not be approved.',
    afterSuccessfulApproval: 'The message could not be sent.',
    successfulSendCompleted: true,
  })
})

it('shows an attachment size warning over a retained message send error', async () => {
  let sendCount = 0
  const attachmentSession = { ...session, id: crypto.randomUUID() }
  renderedSessionIds.add(attachmentSession.id)
  const fetcher = vi.fn<typeof fetch>((input, init) => {
    const url = requestUrl(input)
    if (init?.method === 'POST' && url.endsWith('/messages')) {
      sendCount += 1
      return Promise.resolve(
        jsonResponse({ error: 'The message could not be sent.' }, 503),
      )
    }
    if (url.includes('/messages?')) {
      return Promise.resolve(jsonResponse(messagesResponse([])))
    }
    if (url.endsWith('/actions')) return Promise.resolve(jsonResponse(actions))
    return Promise.resolve(jsonResponse({}))
  })
  vi.stubGlobal('fetch', fetcher)

  root = createRoot(container)
  act(() => {
    root?.render(
      <SessionConversationCacheProvider>
        <ConversationPane
          session={attachmentSession}
          shouldSkipSessionTreeFetch
          initialSessionTree={[]}
          refreshKey={0}
          sessionTreeRefreshKey={0}
          onSelectSession={() => undefined}
        />
      </SessionConversationCacheProvider>,
    )
  })
  await waitForState(
    () =>
      container.querySelector<HTMLTextAreaElement>('textarea')?.disabled ===
      false,
  )
  const textarea = requireElement(
    container.querySelector<HTMLTextAreaElement>('textarea'),
  )
  setTextareaValue(textarea, 'Retain this prompt')
  await act(async () => {
    dispatchKey(textarea, 'Enter')
    for (let index = 0; index < 12; index += 1) await Promise.resolve()
  })
  await waitForState(
    () =>
      container.querySelector('[role="alert"]')?.textContent ===
      'The message could not be sent.',
  )

  const firstFile = new File(['first'], 'first.bin', {
    type: 'application/octet-stream',
  })
  const limitCrossingFile = new File(['second'], 'limit-crossing.bin', {
    type: 'application/octet-stream',
  })
  Object.defineProperty(firstFile, 'size', { value: 30 * 1024 * 1024 })
  Object.defineProperty(limitCrossingFile, 'size', { value: 21 * 1024 * 1024 })
  const pasteEvent = new Event('paste', { bubbles: true, cancelable: true })
  Object.defineProperty(pasteEvent, 'clipboardData', {
    value: { files: [firstFile, limitCrossingFile] },
  })
  act(() => {
    void textarea.dispatchEvent(pasteEvent)
  })
  const limitWarning =
    'limit-crossing.bin would exceed the 50 MiB attachment limit'
  const warningDisplayed = await waitForState(
    () =>
      container.querySelectorAll('.composer-attachment').length === 2 &&
      container.querySelector('[role="alert"]')?.textContent === limitWarning,
  )

  await act(async () => {
    await userEvent.click(
      requireElement(
        container.querySelector<HTMLButtonElement>(
          '[aria-label="Remove limit-crossing.bin"]',
        ),
      ),
    )
  })
  const retainedErrorRestored = await waitForState(
    () =>
      container.querySelectorAll('.composer-attachment').length === 1 &&
      container.querySelector('[role="alert"]')?.textContent ===
        'The message could not be sent.',
  )

  await act(async () => {
    dispatchKey(textarea, 'Enter')
    for (let index = 0; index < 12; index += 1) await Promise.resolve()
  })

  const snapshot = () => ({
    warningDisplayed,
    retainedErrorRestored,
    alert: container.querySelector('[role="alert"]')?.textContent ?? null,
    sendCount,
  })
  expect(snapshot()).toEqual({
    warningDisplayed: true,
    retainedErrorRestored: true,
    alert: 'The message could not be sent.',
    sendCount: 1,
  })
})

it('keeps a retained message error visible after an empty attachment error', async () => {
  let sendCount = 0
  let encodingFailureCount = 0
  const attachmentSession = { ...session, id: crypto.randomUUID() }
  renderedSessionIds.add(attachmentSession.id)
  const fetcher = vi.fn<typeof fetch>((input, init) => {
    const url = requestUrl(input)
    if (init?.method === 'POST' && url.endsWith('/messages')) {
      sendCount += 1
      return Promise.resolve(
        jsonResponse({ error: 'The message could not be sent.' }, 503),
      )
    }
    if (url.includes('/messages?')) {
      return Promise.resolve(jsonResponse(messagesResponse([])))
    }
    if (url.endsWith('/actions')) return Promise.resolve(jsonResponse(actions))
    return Promise.resolve(jsonResponse({}))
  })
  vi.stubGlobal('fetch', fetcher)

  root = createRoot(container)
  act(() => {
    root?.render(
      <SessionConversationCacheProvider>
        <ConversationPane
          session={attachmentSession}
          shouldSkipSessionTreeFetch
          initialSessionTree={[]}
          refreshKey={0}
          sessionTreeRefreshKey={0}
          onSelectSession={() => undefined}
        />
      </SessionConversationCacheProvider>,
    )
  })
  await waitForState(
    () =>
      container.querySelector<HTMLTextAreaElement>('textarea')?.disabled ===
      false,
  )
  const textarea = requireElement(
    container.querySelector<HTMLTextAreaElement>('textarea'),
  )
  setTextareaValue(textarea, 'Retain this prompt')
  await act(async () => {
    dispatchKey(textarea, 'Enter')
    for (let index = 0; index < 12; index += 1) await Promise.resolve()
  })
  await waitForState(
    () =>
      container.querySelector('[role="alert"]')?.textContent ===
      'The message could not be sent.',
  )

  const brokenFile = new File(['payload'], 'broken.bin', {
    type: 'application/octet-stream',
  })
  vi.spyOn(Blob.prototype, 'arrayBuffer').mockImplementation(() => {
    encodingFailureCount += 1
    return Promise.reject(new Error(''))
  })
  const pasteEvent = new Event('paste', { bubbles: true, cancelable: true })
  Object.defineProperty(pasteEvent, 'clipboardData', {
    value: { files: [brokenFile] },
  })
  act(() => {
    void textarea.dispatchEvent(pasteEvent)
  })
  await waitForState(
    () => container.querySelector('.composer-attachment') !== null,
  )
  await waitForState(
    () =>
      container
        .querySelector('.conversation-composer')
        ?.getAttribute('aria-busy') === 'false',
  )

  await act(async () => {
    dispatchKey(textarea, 'Enter')
    for (let index = 0; index < 12; index += 1) await Promise.resolve()
  })
  const retainedErrorAfterEncodingFailure = await waitForState(
    () =>
      encodingFailureCount > 0 &&
      container.querySelector('[role="alert"]')?.textContent ===
        'The message could not be sent.',
  )

  const snapshot = () => ({
    retainedErrorAfterEncodingFailure,
    alert: container.querySelector('[role="alert"]')?.textContent ?? null,
    encodingFailed: encodingFailureCount > 0,
    sendCount,
  })
  expect(snapshot()).toEqual({
    retainedErrorAfterEncodingFailure: true,
    alert: 'The message could not be sent.',
    encodingFailed: true,
    sendCount: 1,
  })
})

it('keeps a permission error until the same request succeeds', async () => {
  let permissionRequestCount = 0
  const fetcher = vi.fn<typeof fetch>((input, init) => {
    const url = requestUrl(input)
    if (init?.method === 'POST' && url.endsWith('/permission/approve')) {
      permissionRequestCount += 1
      return Promise.resolve(
        permissionRequestCount === 1
          ? jsonResponse(
              { error: 'The permission could not be approved.' },
              503,
            )
          : jsonResponse({}),
      )
    }
    if (url.includes('/messages?')) {
      return Promise.resolve(
        jsonResponse(messagesResponse([permission, otherPermission])),
      )
    }
    if (url.endsWith('/actions')) return Promise.resolve(jsonResponse(actions))
    return Promise.resolve(jsonResponse({}))
  })
  vi.stubGlobal('fetch', fetcher)

  root = createRoot(container)
  act(() => {
    root?.render(
      <SessionConversationCacheProvider>
        <ConversationPane
          session={session}
          shouldSkipSessionTreeFetch
          initialSessionTree={[]}
          refreshKey={0}
          sessionTreeRefreshKey={0}
          onSelectSession={() => undefined}
        />
      </SessionConversationCacheProvider>,
    )
  })
  await waitForState(
    () =>
      container.querySelectorAll<HTMLButtonElement>(
        '.chat-permission-card .permission-actions button',
      ).length === 4 &&
      Array.from(
        container.querySelectorAll<HTMLButtonElement>(
          '.chat-permission-card .permission-actions button',
        ),
      ).every((button) => !button.disabled),
  )
  const approveButtons = () =>
    container.querySelectorAll<HTMLButtonElement>(
      '.chat-permission-card .permission-actions button',
    )
  await act(async () => {
    await userEvent.click(requireElement(approveButtons().item(0)))
  })
  await waitForState(
    () =>
      container.querySelector('[role="alert"]')?.textContent ===
      'The permission could not be approved.',
  )
  const afterFailedRequest =
    container.querySelector('[role="alert"]')?.textContent ?? null

  await act(async () => {
    await userEvent.click(requireElement(approveButtons().item(2)))
  })
  const unrelatedRequestCompleted = await waitForState(
    () =>
      permissionRequestCount === 2 &&
      fetcher.mock.calls.filter(
        ([input, init]) =>
          init?.method !== 'POST' && requestUrl(input).includes('/messages?'),
      ).length === 2,
  )
  const afterDifferentRequestSucceeded = {
    completed: unrelatedRequestCompleted,
    alert: container.querySelector('[role="alert"]')?.textContent ?? null,
  }

  await act(async () => {
    await userEvent.click(requireElement(approveButtons().item(0)))
  })
  const sameRequestCompleted = await waitForState(
    () =>
      permissionRequestCount === 3 &&
      container.querySelector('[role="alert"]') === null,
  )

  const snapshot = () => ({
    afterFailedRequest,
    afterDifferentRequestSucceeded,
    afterSameRequestSucceeded: sameRequestCompleted,
  })
  expect(snapshot()).toEqual({
    afterFailedRequest: 'The permission could not be approved.',
    afterDifferentRequestSucceeded: {
      completed: true,
      alert: 'The permission could not be approved.',
    },
    afterSameRequestSucceeded: true,
  })
})

it('clears a proposal error after an edited answer succeeds', async () => {
  let answerCount = 0
  const answerComments: string[] = []
  const fetcher = vi.fn<typeof fetch>((input, init) => {
    const url = requestUrl(input)
    if (init?.method === 'POST' && url.endsWith('/proposals/3/answer')) {
      answerCount += 1
      if (typeof init.body === 'string') {
        const body: unknown = JSON.parse(init.body)
        if (typeof body === 'object' && body !== null) {
          const comment = 'comment' in body ? body.comment : undefined
          if (typeof comment === 'string') answerComments.push(comment)
        }
      }
      return Promise.resolve(
        answerCount === 1
          ? jsonResponse(
              { error: 'The proposal answer could not be saved.' },
              503,
            )
          : jsonResponse({}),
      )
    }
    if (url.includes('/messages?')) {
      return Promise.resolve(jsonResponse(messagesResponse([proposal])))
    }
    if (url.endsWith('/actions')) return Promise.resolve(jsonResponse(actions))
    return Promise.resolve(jsonResponse({}))
  })
  vi.stubGlobal('fetch', fetcher)

  root = createRoot(container)
  act(() => {
    root?.render(
      <SessionConversationCacheProvider>
        <ConversationPane
          session={session}
          shouldSkipSessionTreeFetch
          initialSessionTree={[]}
          refreshKey={0}
          sessionTreeRefreshKey={0}
          onSelectSession={() => undefined}
        />
      </SessionConversationCacheProvider>,
    )
  })
  await waitForState(
    () => container.querySelector('.chat-proposal-comment') !== null,
  )
  const comment = requireElement(
    container.querySelector<HTMLTextAreaElement>('.chat-proposal-comment'),
  )
  setTextareaValue(comment, 'First answer')
  act(() => {
    requireElement(
      container.querySelector<HTMLInputElement>('.chat-proposal-option input'),
    ).click()
  })
  const submit = requireElement(
    container.querySelector<HTMLButtonElement>(
      '.chat-proposal-actions .aw-button--primary',
    ),
  )
  await act(async () => {
    await userEvent.click(submit)
  })
  await waitForState(
    () =>
      container.querySelector('[role="alert"]')?.textContent ===
      'The proposal answer could not be saved.',
  )
  const afterFailedAnswer =
    container.querySelector('[role="alert"]')?.textContent ?? null

  const updatedComment = requireElement(
    container.querySelector<HTMLTextAreaElement>('.chat-proposal-comment'),
  )
  setTextareaValue(updatedComment, 'Revised answer')
  const updatedSubmit = requireElement(
    container.querySelector<HTMLButtonElement>(
      '.chat-proposal-actions .aw-button--primary',
    ),
  )
  await act(async () => userEvent.click(updatedSubmit))
  const editedAnswerCompleted = await waitForState(
    () =>
      answerCount === 2 && container.querySelector('[role="alert"]') === null,
  )

  const snapshot = () => ({
    afterFailedAnswer,
    editedAnswerCompleted,
    submittedComments: answerComments,
  })
  expect(snapshot()).toEqual({
    afterFailedAnswer: 'The proposal answer could not be saved.',
    editedAnswerCompleted: true,
    submittedComments: ['First answer', 'Revised answer'],
  })
})

it('clears a todo error after a retry with an edited comment succeeds', async () => {
  const todo: ChatItem = {
    type: 'todo',
    seq: 4,
    title: 'Check a status panel',
    detail: 'Confirm the panel closes after resolution.',
    completed: false,
    resolution: null,
    comment: null,
  }
  let resolutionCount = 0
  const submittedComments: string[] = []
  const fetcher = vi.fn<typeof fetch>((input, init) => {
    const url = requestUrl(input)
    if (init?.method === 'POST' && url.endsWith('/todos/4/resolve')) {
      resolutionCount += 1
      if (typeof init.body === 'string') {
        const body: unknown = JSON.parse(init.body)
        if (typeof body === 'object' && body !== null && 'comment' in body) {
          const comment = body.comment
          if (typeof comment === 'string') submittedComments.push(comment)
        }
      }
      return Promise.resolve(
        resolutionCount === 1
          ? jsonResponse({ error: 'The todo could not be resolved.' }, 503)
          : jsonResponse({}),
      )
    }
    if (url.includes('/messages?')) {
      return Promise.resolve(jsonResponse(messagesResponse([todo])))
    }
    if (url.endsWith('/actions')) return Promise.resolve(jsonResponse(actions))
    return Promise.resolve(jsonResponse({}))
  })
  vi.stubGlobal('fetch', fetcher)

  root = createRoot(container)
  act(() => {
    root?.render(
      <SessionConversationCacheProvider>
        <ConversationPane
          session={session}
          shouldSkipSessionTreeFetch
          initialSessionTree={[]}
          refreshKey={0}
          sessionTreeRefreshKey={0}
          onSelectSession={() => undefined}
        />
      </SessionConversationCacheProvider>,
    )
  })
  await waitForState(
    () => container.querySelector('.chat-todo-comment') !== null,
  )
  const comment = requireElement(
    container.querySelector<HTMLTextAreaElement>('.chat-todo-comment'),
  )
  setTextareaValue(comment, 'First resolution')
  const markDone = requireElement(
    container.querySelector<HTMLButtonElement>(
      '.chat-todo-footer .aw-button--primary',
    ),
  )
  await act(async () => userEvent.click(markDone))
  await waitForState(
    () =>
      container.querySelector('[role="alert"]')?.textContent ===
      'The todo could not be resolved.',
  )
  const afterFailedResolution =
    container.querySelector('[role="alert"]')?.textContent ?? null

  const updatedComment = requireElement(
    container.querySelector<HTMLTextAreaElement>('.chat-todo-comment'),
  )
  setTextareaValue(updatedComment, 'Revised resolution')
  const updatedMarkDone = requireElement(
    container.querySelector<HTMLButtonElement>(
      '.chat-todo-footer .aw-button--primary',
    ),
  )
  await act(async () => userEvent.click(updatedMarkDone))
  const editedResolutionCompleted = await waitForState(
    () =>
      resolutionCount === 2 &&
      container.querySelector('[role="alert"]') === null,
  )

  const snapshot = () => ({
    afterFailedResolution,
    editedResolutionCompleted,
    submittedComments,
  })
  expect(snapshot()).toEqual({
    afterFailedResolution: 'The todo could not be resolved.',
    editedResolutionCompleted: true,
    submittedComments: ['First resolution', 'Revised resolution'],
  })
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
      message: true,
      answerQuestion: false,
      interrupt: false,
      openInTerminal: true,
      rewind: false,
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
        rewind: false,
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
        rewind: false,
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
      '.conversation-status-band-tasks summary',
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
    rewind: false,
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
        rewind: false,
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
        rewind: false,
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
        rewind: false,
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

it('reuses a cached conversation across an A to B to A session switch', async () => {
  const firstSession = { ...session, id: 'cache-session-a', title: 'Session A' }
  const secondSession = {
    ...session,
    id: 'cache-session-b',
    title: 'Session B',
  }
  const firstMessages = Array.from({ length: 21 }, (_, index) => ({
    type: 'assistant' as const,
    text: `A message ${String(index + 1)}`,
  })) satisfies ChatItem[]
  const secondMessages = [
    { type: 'assistant', text: 'B message' },
  ] satisfies ChatItem[]
  const initialFirstResponse = {
    ...messagesResponse(firstMessages, [], { start: 0, total: 21 }),
    stableStart: 0,
    stableEnd: 1,
    stableDigest: 'digest-a',
  }
  const initialSecondResponse = messagesResponse(secondMessages)
  const refreshedFirstMessages = [
    ...firstMessages.slice(1),
    { type: 'assistant', text: 'A message 22' },
  ] satisfies ChatItem[]
  let resolveFirstRefresh!: (response: Response) => void
  const delayedFirstRefresh = new Promise<Response>((resolve) => {
    resolveFirstRefresh = resolve
  })
  const fetcher = vi.fn<typeof fetch>((input) => {
    const url = requestUrl(input)
    if (url.includes('/sessions/cache-session-a/messages?limit=100')) {
      return Promise.resolve(jsonResponse(initialFirstResponse))
    }
    if (url.includes('/sessions/cache-session-b/messages?limit=100')) {
      return Promise.resolve(jsonResponse(initialSecondResponse))
    }
    if (
      url.includes(
        '/sessions/cache-session-a/messages?from=0&stableStart=0&stableEnd=1&stableDigest=digest-a',
      )
    ) {
      return delayedFirstRefresh
    }
    if (url.endsWith('/actions')) return Promise.resolve(jsonResponse(actions))
    if (url.endsWith('/tree')) {
      return Promise.resolve(
        jsonResponse({ revision: 1, sessions: [] satisfies SessionTreeNode[] }),
      )
    }
    return Promise.resolve(jsonResponse({}))
  })
  vi.stubGlobal('fetch', fetcher)

  renderInboxPanes([firstSession, secondSession], firstSession)
  await waitForState(() => container.textContent.includes('A message 1'))
  renderInboxPanes([firstSession, secondSession], secondSession)
  await waitForState(() => container.textContent.includes('B message'))
  renderInboxPanes([firstSession, secondSession], firstSession)
  await waitForState(() =>
    fetcher.mock.calls.some(([input]) =>
      requestUrl(input).includes('&stableDigest=digest-a'),
    ),
  )
  const cachedMessageWasShown = container.textContent.includes('A message 1')
  resolveFirstRefresh(
    jsonResponse(
      messagesResponse(refreshedFirstMessages, [], { start: 1, total: 22 }),
    ),
  )
  await waitForState(() => container.textContent.includes('A message 22'))

  const snapshot = () => ({
    cachedMessageWasShown,
    requests: fetcher.mock.calls
      .map(([input]) => requestUrl(input))
      .filter((url) => url.includes('/messages') || url.endsWith('/actions')),
    messages: Array.from(
      container.querySelectorAll('.chat-assistant-message'),
      (element) => element.textContent,
    ),
  })
  expect(snapshot()).toEqual({
    cachedMessageWasShown: true,
    requests: [
      '/api/sessions/cache-session-a/messages?limit=100',
      '/api/sessions/cache-session-a/actions',
      '/api/sessions/cache-session-b/messages?limit=100',
      '/api/sessions/cache-session-b/actions',
      '/api/sessions/cache-session-a/messages?from=0&stableStart=0&stableEnd=1&stableDigest=digest-a',
      '/api/sessions/cache-session-a/actions',
    ],
    messages: [
      ...firstMessages,
      { type: 'assistant', text: 'A message 22' },
    ].map((message) => message.text),
  })
})

it('shows cached permission actions while the messages refresh is pending', async () => {
  const firstSession = { ...session, id: 'cached-permission-session' }
  const secondSession = { ...session, id: 'next-session' }
  let messagesRequestCount = 0
  let actionsRequestCount = 0
  let resolveMessages: ((response: Response) => void) | undefined
  let resolveActions: ((response: Response) => void) | undefined
  const fetcher = vi.fn<typeof fetch>((input) => {
    const url = requestUrl(input)
    if (url.includes(`/sessions/${firstSession.id}/messages`)) {
      messagesRequestCount += 1
      if (messagesRequestCount === 1) {
        return Promise.resolve(jsonResponse(messagesResponse([permission])))
      }
      return new Promise<Response>((resolve) => {
        resolveMessages = resolve
      })
    }
    if (url.includes(`/sessions/${secondSession.id}/messages`)) {
      return Promise.resolve(jsonResponse(messagesResponse([])))
    }
    if (url === `/api/sessions/${firstSession.id}/actions`) {
      actionsRequestCount += 1
      if (actionsRequestCount > 1) {
        return new Promise<Response>((resolve) => {
          resolveActions = resolve
        })
      }
      return Promise.resolve(jsonResponse(actions))
    }
    if (url.endsWith('/actions')) return Promise.resolve(jsonResponse(actions))
    if (url.endsWith('/tree')) {
      return Promise.resolve(jsonResponse({ revision: 1, sessions: [] }))
    }
    return Promise.resolve(jsonResponse({}))
  })
  vi.stubGlobal('fetch', fetcher)

  renderInboxPanes([firstSession, secondSession], firstSession)
  await waitForState(
    () => container.querySelector('.chat-permission-card') !== null,
  )
  renderInboxPanes([firstSession, secondSession], secondSession)
  await waitForState(() =>
    fetcher.mock.calls.some(([input]) =>
      requestUrl(input).includes(`/sessions/${secondSession.id}/messages`),
    ),
  )
  renderInboxPanes([firstSession, secondSession], firstSession)
  await waitForState(() => resolveActions !== undefined)

  const stateBeforeActions = {
    permissionCard: container.querySelector('.chat-permission-card') !== null,
    composerDisabled: requireElement(
      container.querySelector<HTMLTextAreaElement>('textarea'),
    ).disabled,
    permissionButtons: container.querySelectorAll('.permission-actions button')
      .length,
    messagesPending: resolveMessages !== undefined,
  }
  resolveActions?.(jsonResponse(actions))
  await waitForState(
    () => container.querySelectorAll('.permission-actions button').length === 2,
  )

  const stateAfterActions = {
    composerDisabled: requireElement(
      container.querySelector<HTMLTextAreaElement>('textarea'),
    ).disabled,
    permissionButtons: Array.from(
      container.querySelectorAll<HTMLButtonElement>(
        '.permission-actions button',
      ),
      (button) => ({ label: button.textContent, disabled: button.disabled }),
    ),
    messagesPending: resolveMessages !== undefined,
  }
  await act(async () => {
    resolveMessages?.(jsonResponse(messagesResponse([permission])))
    for (let index = 0; index < 12; index += 1) await Promise.resolve()
  })

  const snapshot = () => ({ stateBeforeActions, stateAfterActions })
  const expected = {
    stateBeforeActions: {
      permissionCard: true,
      composerDisabled: false,
      permissionButtons: 0,
      messagesPending: true,
    },
    stateAfterActions: {
      composerDisabled: false,
      permissionButtons: [
        { label: 'Approve', disabled: false },
        { label: 'Reject', disabled: false },
      ],
      messagesPending: true,
    },
  }
  expect(snapshot()).toEqual(expected)
})

it('keeps a cleared actions request error from returning after messages load', async () => {
  const failedSession = { ...session, id: 'actions-error-session' }
  let resolveMessages: ((response: Response) => void) | undefined
  const fetcher = vi.fn<typeof fetch>((input) => {
    const url = requestUrl(input)
    if (url.endsWith('/messages?limit=100')) {
      return new Promise<Response>((resolve) => {
        resolveMessages = resolve
      })
    }
    if (url.endsWith('/actions')) {
      return Promise.resolve(
        jsonResponse({ error: 'Actions unavailable' }, 503),
      )
    }
    if (url.endsWith('/tree')) {
      return Promise.resolve(jsonResponse({ revision: 1, sessions: [] }))
    }
    return Promise.resolve(jsonResponse({}))
  })
  vi.stubGlobal('fetch', fetcher)

  root ??= createRoot(container)
  act(() => {
    root?.render(
      <SessionConversationCacheProvider>
        <ConversationPane
          session={failedSession}
          sessionTreeRefreshKey={0}
          refreshKey={0}
          onSelectSession={() => undefined}
        />
      </SessionConversationCacheProvider>,
    )
  })
  await waitForState(
    () =>
      container.querySelector('[role="alert"]')?.textContent ===
      'Actions unavailable',
  )
  await waitForState(() => resolveMessages !== undefined)
  const textarea = requireElement(
    container.querySelector<HTMLTextAreaElement>('textarea'),
  )
  const afterActionsFailure = {
    alert: container.querySelector('[role="alert"]')?.textContent ?? null,
    composerDisabled: textarea.disabled,
  }
  const pasteEvent = new Event('paste', { bubbles: true, cancelable: true })
  Object.defineProperty(pasteEvent, 'clipboardData', {
    value: {
      files: [
        new File(['contents'], 'clipboard.bin', {
          type: 'application/octet-stream',
        }),
      ],
    },
  })
  act(() => {
    void textarea.dispatchEvent(pasteEvent)
  })
  await waitForState(() => container.querySelector('[role="alert"]') === null)
  const afterClear =
    container.querySelector('[role="alert"]')?.textContent ?? null
  await act(async () => {
    resolveMessages?.(jsonResponse(messagesResponse([])))
    for (let index = 0; index < 12; index += 1) await Promise.resolve()
  })

  const snapshot = () => ({
    afterActionsFailure,
    afterClear,
    afterMessagesLoad:
      container.querySelector('[role="alert"]')?.textContent ?? null,
  })
  expect(snapshot()).toEqual({
    afterActionsFailure: {
      alert: 'Actions unavailable',
      composerDisabled: false,
    },
    afterClear: null,
    afterMessagesLoad: null,
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

it('loads older messages from a tail cursor and anchors when the epoch changes', async () => {
  const latestMessage: ChatItem = {
    type: 'assistant',
    text: 'Latest fixture message.',
  }
  const earlierMessage: ChatItem = {
    type: 'assistant',
    text: 'Earlier fixture message.',
  }
  const fetcher = vi.fn<typeof fetch>((input) => {
    const url = requestUrl(input)
    if (url.endsWith('/messages?limit=100')) {
      return Promise.resolve(
        jsonResponse(
          messagesResponse([latestMessage], [], {
            start: 0,
            total: 1,
            epoch: 'fixture-epoch-amber',
            olderCursor: 'fixture-epoch-amber',
          }),
        ),
      )
    }
    if (
      url.endsWith(
        '/messages?olderCursor=fixture-epoch-amber&limit=100&epoch=fixture-epoch-amber',
      )
    ) {
      return Promise.resolve(
        jsonResponse(
          messagesResponse([earlierMessage], [], {
            start: 1,
            total: 2,
            epoch: 'fixture-epoch-cobalt',
          }),
        ),
      )
    }
    if (url.endsWith('/actions')) return Promise.resolve(jsonResponse(actions))
    return Promise.resolve(jsonResponse({}))
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
  await waitForState(
    () => container.querySelectorAll('.chat-assistant-message').length === 1,
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
    scroll.scrollTop = 12
    container
      .querySelector<HTMLButtonElement>('.conversation-load-older')
      ?.click()
  })
  await waitForState(
    () => container.querySelectorAll('.chat-assistant-message').length === 2,
  )

  const snapshot = () => ({
    messageRequests: fetcher.mock.calls
      .map(([url]) => requestUrl(url))
      .filter((url) => url.includes('/messages?')),
    messages: Array.from(
      container.querySelectorAll('.chat-assistant-message'),
      (element) => element.textContent,
    ),
    scrollTop: scroll.scrollTop,
    olderButton: container.querySelector('.conversation-load-older'),
  })
  expect(snapshot()).toEqual({
    messageRequests: [
      '/api/sessions/sample-session/messages?limit=100',
      '/api/sessions/sample-session/messages?olderCursor=fixture-epoch-amber&limit=100&epoch=fixture-epoch-amber',
    ],
    messages: ['Earlier fixture message.', 'Latest fixture message.'],
    scrollTop: 52,
    olderButton: null,
  })
})

it('keeps a merge result after the messages present when an older page is prepended', async () => {
  const initialMessages: ChatItem[] = [
    { type: 'assistant', text: 'First visible message.' },
    { type: 'assistant', text: 'Latest visible message.' },
  ]
  const olderMessage: ChatItem = {
    type: 'assistant',
    text: 'Older message.',
  }
  const fetcher = vi.fn<typeof fetch>((input) => {
    const url = requestUrl(input)
    if (url.endsWith('/messages?limit=100')) {
      return Promise.resolve(
        jsonResponse(
          messagesResponse(initialMessages, [], {
            start: 0,
            total: 3,
            epoch: 'fixture-epoch-amber',
            olderCursor: 'fixture-older-page',
          }),
        ),
      )
    }
    if (
      url.endsWith(
        '/messages?olderCursor=fixture-older-page&limit=100&epoch=fixture-epoch-amber',
      )
    ) {
      return Promise.resolve(
        jsonResponse(
          messagesResponse([olderMessage], [], {
            start: 1,
            total: 3,
            epoch: 'fixture-epoch-cobalt',
          }),
        ),
      )
    }
    if (url.endsWith('/actions')) {
      return Promise.resolve(jsonResponse(actions))
    }
    return Promise.resolve(jsonResponse({}))
  })
  vi.stubGlobal('fetch', fetcher)

  function MergeResultStarter() {
    const feedback = useOptionalAsyncActionFeedback()
    return (
      <>
        <button
          type="button"
          onClick={() => {
            feedback?.start({
              kind: 'merge',
              sessionId: session.id,
              sessionTitle: session.title,
              pullRequestNumber: 9061,
              timelineIndex: 2,
              run: () => Promise.resolve({ status: 'merged' as const }),
            })
            feedback?.start({
              kind: 'merge',
              sessionId: 'fixture-other-session',
              sessionTitle: 'Another session',
              pullRequestNumber: 9062,
              timelineIndex: 2,
              run: () => Promise.resolve({ status: 'merged' as const }),
            })
          }}
        >
          Start merge action
        </button>
        <output
          data-other-session-timeline-index={String(
            feedback?.actions.find(
              (action) => action.sessionId === 'fixture-other-session',
            )?.timelineIndex ?? '',
          )}
        />
      </>
    )
  }

  root = createRoot(container)
  act(() => {
    root?.render(
      <AsyncActionFeedbackProvider>
        <MergeResultStarter />
        <ConversationPane
          session={session}
          shouldSkipSessionTreeFetch
          initialSessionTree={[]}
          refreshKey={0}
          sessionTreeRefreshKey={0}
          onSelectSession={() => undefined}
        />
      </AsyncActionFeedbackProvider>,
    )
  })
  await waitForState(
    () => container.querySelectorAll('.chat-assistant-message').length === 2,
  )
  await act(async () => {
    await userEvent.click(
      requireElement(
        [...container.querySelectorAll<HTMLButtonElement>('button')].find(
          (button) => button.textContent === 'Start merge action',
        ) ?? null,
      ),
    )
  })
  await waitForState(
    () =>
      container.querySelector('.async-action-item--line')?.textContent ===
      'Merged PR #9061',
  )
  await act(async () => {
    await userEvent.click(
      requireElement(
        container.querySelector<HTMLButtonElement>('.conversation-load-older'),
      ),
    )
  })
  await waitForState(
    () => container.querySelectorAll('.chat-assistant-message').length === 3,
  )

  const snapshot = () => ({
    timeline: Array.from(
      container.querySelectorAll(
        '.conversation-scroll .chat-assistant-message, .conversation-scroll .async-action-item--line',
      ),
      (element) => normalizeAsyncActionLineTime(element.textContent),
    ),
    otherSessionTimelineIndex: container
      .querySelector('[data-other-session-timeline-index]')
      ?.getAttribute('data-other-session-timeline-index'),
  })
  expect(snapshot()).toEqual({
    timeline: [
      'Older message.',
      'First visible message.',
      'Latest visible message.',
      'Merged PR #9061 · <time>',
    ],
    otherSessionTimelineIndex: '2',
  })
})

it('coalesces refresh-key changes while keeping the in-flight response', async () => {
  const messageResponses: Array<(response: Response) => void> = []
  let refreshKey = 0
  let actionsRequestCount = 0
  const fetcher = vi.fn<typeof fetch>((input) => {
    const url = requestUrl(input)
    if (url.includes('/messages?')) {
      return new Promise<Response>((resolve) => {
        messageResponses.push(resolve)
      })
    }
    if (url.endsWith('/actions')) {
      actionsRequestCount += 1
      return Promise.resolve(jsonResponse(actions))
    }
    return Promise.resolve(jsonResponse({}))
  })
  vi.stubGlobal('fetch', fetcher)
  root = createRoot(container)
  const renderConversation = () => {
    root?.render(
      <ConversationPane
        session={session}
        shouldSkipSessionTreeFetch
        initialSessionTree={[]}
        refreshKey={refreshKey}
        sessionTreeRefreshKey={0}
        onSelectSession={() => undefined}
      />,
    )
  }
  const visibleMessages = () =>
    Array.from(
      container.querySelectorAll('.chat-assistant-message'),
      (element) => element.textContent,
    )

  act(renderConversation)
  await waitForState(() => messageResponses.length === 1)
  refreshKey = 1
  act(renderConversation)
  refreshKey = 2
  act(renderConversation)
  const requestsBeforeResponse = messageResponses.length
  const actionsRequestsBeforeResponse = actionsRequestCount
  const resolveFirstResponse = messageResponses[0]
  if (resolveFirstResponse === undefined)
    throw new Error('Expected the initial message response')
  await act(async () => {
    resolveFirstResponse(
      jsonResponse(
        messagesResponse([{ type: 'assistant', text: 'Initial message.' }], []),
      ),
    )
    await Promise.resolve()
  })
  await waitForState(() => messageResponses.length === 2)

  const snapshot = () => ({
    requestUrls: fetcher.mock.calls
      .map(([url]) => requestUrl(url))
      .filter((url) => url.includes('/messages?')),
    requestsBeforeResponse,
    requestsAfterResponse: messageResponses.length,
    actionsRequestsBeforeResponse,
    actionsRequestCount,
    visibleMessages: visibleMessages(),
    loadingStatus:
      container.querySelector('.conversation-status')?.textContent ?? null,
  })
  expect(snapshot()).toEqual({
    requestUrls: [
      '/api/sessions/sample-session/messages?limit=100',
      '/api/sessions/sample-session/messages?from=0',
    ],
    requestsBeforeResponse: 1,
    requestsAfterResponse: 2,
    actionsRequestsBeforeResponse: 3,
    actionsRequestCount: 4,
    visibleMessages: ['Initial message.'],
    loadingStatus: null,
  })
})

it('discards a message response after switching away and back to its session', async () => {
  const messageResponses: Array<(response: Response) => void> = []
  let selectedSession = session
  const otherSession = { ...session, id: 'other-session' }
  const fetcher = vi.fn<typeof fetch>((input) => {
    const url = requestUrl(input)
    if (url.includes('/messages?')) {
      return new Promise<Response>((resolve) => {
        messageResponses.push(resolve)
      })
    }
    if (url.endsWith('/actions')) return Promise.resolve(jsonResponse(actions))
    return Promise.resolve(jsonResponse({}))
  })
  vi.stubGlobal('fetch', fetcher)
  root = createRoot(container)
  const renderConversation = () => {
    root?.render(
      <ConversationPane
        session={selectedSession}
        shouldSkipSessionTreeFetch
        initialSessionTree={[]}
        refreshKey={0}
        sessionTreeRefreshKey={0}
        onSelectSession={() => undefined}
      />,
    )
  }
  const resolveMessageResponse = async (index: number, text: string) => {
    const resolve = messageResponses[index]
    if (resolve === undefined) throw new Error('Expected a message response')
    await act(async () => {
      resolve(jsonResponse(messagesResponse([{ type: 'assistant', text }], [])))
      await Promise.resolve()
    })
  }
  const visibleMessages = () =>
    Array.from(
      container.querySelectorAll('.chat-assistant-message'),
      (element) => element.textContent,
    )

  act(renderConversation)
  await waitForState(() => messageResponses.length === 1)
  selectedSession = otherSession
  act(renderConversation)
  selectedSession = session
  act(renderConversation)
  await resolveMessageResponse(0, 'Previous visit message.')
  await waitForState(() => messageResponses.length === 2)
  const afterPreviousVisitResponse = visibleMessages()
  await resolveMessageResponse(1, 'Current session message.')
  await waitForState(() => visibleMessages()[0] === 'Current session message.')

  const snapshot = () => ({
    requestUrls: fetcher.mock.calls
      .map(([url]) => requestUrl(url))
      .filter((url) => url.includes('/messages?')),
    afterPreviousVisitResponse,
    visibleMessages: visibleMessages(),
  })
  expect(snapshot()).toEqual({
    requestUrls: [
      '/api/sessions/sample-session/messages?limit=100',
      '/api/sessions/sample-session/messages?limit=100',
    ],
    afterPreviousVisitResponse: [],
    visibleMessages: ['Current session message.'],
  })
})

it('keeps the current page when older history ends and clears its scroll anchor', async () => {
  const resetMessages: ChatItem[] = [
    { type: 'assistant', text: 'Reset fixture response.' },
    { type: 'assistant', text: 'Current fixture response.' },
  ]
  let refreshKey = 0
  const fetcher = vi.fn<typeof fetch>((input) => {
    const url = requestUrl(input)
    if (url.endsWith('/messages?limit=100')) {
      return Promise.resolve(
        jsonResponse(
          messagesResponse([question], [], {
            start: 0,
            total: 1,
            epoch: 'fixture-epoch-amber',
            olderCursor: 'fixture-epoch-amber',
            activeQuestionIndex: 0,
          }),
        ),
      )
    }
    if (
      url.endsWith(
        '/messages?olderCursor=fixture-epoch-amber&limit=100&epoch=fixture-epoch-amber',
      )
    ) {
      return Promise.resolve(
        jsonResponse(
          messagesResponse([], [], {
            start: 0,
            total: 1,
            epoch: 'fixture-epoch-amber',
            activeQuestionIndex: null,
          }),
        ),
      )
    }
    if (url.endsWith('/messages?from=0&epoch=fixture-epoch-amber')) {
      return Promise.resolve(
        jsonResponse(
          messagesResponse(resetMessages, [], {
            start: 0,
            total: 2,
            epoch: 'fixture-epoch-cobalt',
            epochReset: true,
          }),
        ),
      )
    }
    if (url.endsWith('/actions')) return Promise.resolve(jsonResponse(actions))
    return Promise.resolve(jsonResponse({}))
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
      />,
    )
  }
  act(renderConversation)
  await waitForState(
    () => container.querySelectorAll('.chat-question-card').length === 1,
  )
  const scroll = requireElement(
    container.querySelector<HTMLDivElement>('.conversation-scroll'),
  )
  Object.defineProperties(scroll, {
    scrollHeight: {
      configurable: true,
      get: () =>
        100 +
        (container.querySelectorAll('.chat-question-card').length +
          container.querySelectorAll('.chat-assistant-message').length) *
          40,
    },
    clientHeight: { configurable: true, value: 100 },
    scrollTop: { configurable: true, writable: true, value: 0 },
  })
  act(() => {
    scroll.scrollTop = 10
    scroll.dispatchEvent(new Event('scroll'))
  })
  await waitForState(
    () => container.querySelector('.conversation-load-older') === null,
  )

  refreshKey = 1
  act(renderConversation)
  await waitForState(
    () => container.querySelectorAll('.chat-assistant-message').length === 2,
  )

  const snapshot = () => ({
    messageRequests: fetcher.mock.calls
      .map(([url]) => requestUrl(url))
      .filter((url) => url.includes('/messages?')),
    questionCards: container.querySelectorAll('.chat-question-card').length,
    messages: Array.from(
      container.querySelectorAll('.chat-assistant-message'),
      (element) => element.textContent,
    ),
    scrollTop: scroll.scrollTop,
    olderButton: container.querySelector('.conversation-load-older'),
  })
  expect(snapshot()).toEqual({
    messageRequests: [
      '/api/sessions/sample-session/messages?limit=100',
      '/api/sessions/sample-session/messages?olderCursor=fixture-epoch-amber&limit=100&epoch=fixture-epoch-amber',
      '/api/sessions/sample-session/messages?from=0&epoch=fixture-epoch-amber',
    ],
    questionCards: 0,
    messages: ['Reset fixture response.', 'Current fixture response.'],
    scrollTop: 10,
    olderButton: null,
  })
})

it('replaces loaded messages when an older cursor resets the epoch', async () => {
  const currentMessage: ChatItem = {
    type: 'assistant',
    text: 'Current fixture response.',
  }
  const resetMessages: ChatItem[] = [
    { type: 'assistant', text: 'Reset fixture response.' },
  ]
  const fetcher = vi.fn<typeof fetch>((input) => {
    const url = requestUrl(input)
    if (url.endsWith('/messages?limit=100')) {
      return Promise.resolve(
        jsonResponse(
          messagesResponse([currentMessage], [], {
            start: 0,
            total: 1,
            epoch: 'fixture-epoch-amber',
            olderCursor: 'fixture-epoch-amber',
          }),
        ),
      )
    }
    if (
      url.endsWith(
        '/messages?olderCursor=fixture-epoch-amber&limit=100&epoch=fixture-epoch-amber',
      )
    ) {
      return Promise.resolve(
        jsonResponse(
          messagesResponse(resetMessages, [], {
            start: 0,
            total: 1,
            epoch: 'fixture-epoch-cobalt',
            epochReset: true,
          }),
        ),
      )
    }
    if (url.endsWith('/actions')) return Promise.resolve(jsonResponse(actions))
    return Promise.resolve(jsonResponse({}))
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
  await waitForState(
    () => container.querySelectorAll('.chat-assistant-message').length === 1,
  )
  act(() => {
    container
      .querySelector<HTMLButtonElement>('.conversation-load-older')
      ?.click()
  })
  await waitForState(() =>
    container.textContent.includes('Reset fixture response.'),
  )

  const snapshot = () => ({
    messageRequests: fetcher.mock.calls
      .map(([url]) => requestUrl(url))
      .filter((url) => url.includes('/messages?')),
    messages: Array.from(
      container.querySelectorAll('.chat-assistant-message'),
      (element) => element.textContent,
    ),
    olderButton: container.querySelector('.conversation-load-older'),
  })
  expect(snapshot()).toEqual({
    messageRequests: [
      '/api/sessions/sample-session/messages?limit=100',
      '/api/sessions/sample-session/messages?olderCursor=fixture-epoch-amber&limit=100&epoch=fixture-epoch-amber',
    ],
    messages: ['Reset fixture response.'],
    olderButton: null,
  })
})

it('keeps an epoch refresh prefix and replaces it after the epoch changes', async () => {
  const initialMessages: ChatItem[] = [
    { type: 'assistant', text: 'Earlier fixture message.' },
    { type: 'assistant', text: 'Latest fixture message.' },
  ]
  const stableEpochMessages: ChatItem[] = [
    { type: 'assistant', text: 'Newer fixture message.' },
    { type: 'assistant', text: 'Newest fixture message.' },
  ]
  const changedEpochMessages: ChatItem[] = [
    { type: 'assistant', text: 'Reset fixture message.' },
    { type: 'assistant', text: 'Current fixture message.' },
  ]
  let refreshCount = 0
  let refreshKey = 0
  const fetcher = vi.fn<typeof fetch>((input) => {
    const url = requestUrl(input)
    if (url.endsWith('/messages?limit=100')) {
      return Promise.resolve(
        jsonResponse(
          messagesResponse(initialMessages, [], {
            start: 1,
            total: 3,
            epoch: 'fixture-epoch-amber',
            olderCursor: 'fixture-epoch-amber',
            stableStart: 1,
            stableEnd: 2,
            stableDigest: 'fixture-digest-cyan',
          }),
        ),
      )
    }
    if (url.includes('/messages?from=')) {
      refreshCount += 1
      return Promise.resolve(
        jsonResponse(
          refreshCount === 1
            ? messagesResponse(stableEpochMessages, [], {
                start: 2,
                total: 4,
                epoch: 'fixture-epoch-amber',
                olderCursor: 'fixture-epoch-amber',
              })
            : messagesResponse(changedEpochMessages, [], {
                start: 0,
                total: 2,
                epoch: 'fixture-epoch-cobalt',
              }),
        ),
      )
    }
    if (url.endsWith('/actions')) return Promise.resolve(jsonResponse(actions))
    return Promise.resolve(jsonResponse({}))
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
      />,
    )
  }
  act(renderConversation)
  await waitForState(
    () => container.querySelectorAll('.chat-assistant-message').length === 2,
  )
  refreshKey = 1
  act(renderConversation)
  await waitForState(
    () => container.querySelectorAll('.chat-assistant-message').length === 3,
  )
  const stableEpochMessagesOnScreen = Array.from(
    container.querySelectorAll('.chat-assistant-message'),
    (element) => element.textContent,
  )
  refreshKey = 2
  act(renderConversation)
  await waitForState(
    () =>
      container.querySelectorAll('.chat-assistant-message').length === 2 &&
      container.textContent.includes('Current fixture message.'),
  )

  const snapshot = () => ({
    messageRequests: fetcher.mock.calls
      .map(([url]) => requestUrl(url))
      .filter((url) => url.includes('/messages?')),
    stableEpochMessagesOnScreen,
    changedEpochMessagesOnScreen: Array.from(
      container.querySelectorAll('.chat-assistant-message'),
      (element) => element.textContent,
    ),
  })
  expect(snapshot()).toEqual({
    messageRequests: [
      '/api/sessions/sample-session/messages?limit=100',
      '/api/sessions/sample-session/messages?from=1&epoch=fixture-epoch-amber&olderCursor=fixture-epoch-amber&stableStart=1&stableEnd=2&stableDigest=fixture-digest-cyan',
      '/api/sessions/sample-session/messages?from=1&epoch=fixture-epoch-amber&olderCursor=fixture-epoch-amber',
    ],
    stableEpochMessagesOnScreen: [
      'Earlier fixture message.',
      'Newer fixture message.',
      'Newest fixture message.',
    ],
    changedEpochMessagesOnScreen: [
      'Reset fixture message.',
      'Current fixture message.',
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
}, 60_000)

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

it('discards an older page after a refresh changes the conversation epoch', async () => {
  const originalMessages: ChatItem[] = [
    { type: 'assistant', text: 'Original first message.' },
    { type: 'assistant', text: 'Original latest message.' },
  ]
  const refreshedMessages: ChatItem[] = [
    { type: 'assistant', text: 'Replacement first message.' },
    { type: 'assistant', text: 'Replacement latest message.' },
  ]
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
    if (url.includes('from=')) return refreshPage
    if (url.includes('olderCursor=')) return olderPage
    if (url.endsWith('/actions')) return Promise.resolve(jsonResponse(actions))
    return Promise.resolve(
      jsonResponse(
        messagesResponse(originalMessages, [], {
          start: 0,
          total: 2,
          epoch: 'fixture-epoch-original',
          olderCursor: 'fixture-epoch-original',
        }),
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
    renderConversation(1)
  })
  await waitForState(() =>
    fetcher.mock.calls.some(([url]) => requestUrl(url).includes('from=')),
  )
  await act(async () => {
    resolveRefresh?.(
      jsonResponse(
        messagesResponse(refreshedMessages, [], {
          start: 0,
          total: 2,
          epoch: 'fixture-epoch-replacement',
          olderCursor: 'fixture-epoch-replacement',
          epochReset: true,
        }),
      ),
    )
    for (let index = 0; index < 12; index += 1) await Promise.resolve()
  })
  await waitForState(
    () =>
      container.querySelector('.chat-assistant-message')?.textContent ===
      'Replacement first message.',
  )
  await act(async () => {
    resolveOlderPage?.(
      jsonResponse(
        messagesResponse(
          [{ type: 'assistant', text: 'Stale older message.' }],
          [],
          {
            start: 0,
            total: 3,
            epoch: 'fixture-epoch-stale',
            olderCursor: 'fixture-epoch-stale',
          },
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
    olderButton: container.querySelector<HTMLButtonElement>(
      '.conversation-load-older',
    )
      ? {
          text: container.querySelector('.conversation-load-older')
            ?.textContent,
          disabled: container.querySelector<HTMLButtonElement>(
            '.conversation-load-older',
          )?.disabled,
        }
      : null,
  })
  expect(snapshot()).toEqual({
    requests: [
      '/api/sessions/sample-session/messages?limit=100',
      '/api/sessions/sample-session/actions',
      '/api/sessions/sample-session/messages?olderCursor=fixture-epoch-original&limit=100&epoch=fixture-epoch-original',
      '/api/sessions/sample-session/messages?from=0&epoch=fixture-epoch-original&olderCursor=fixture-epoch-original',
      '/api/sessions/sample-session/actions',
    ],
    messages: ['Replacement first message.', 'Replacement latest message.'],
    olderButton: { text: 'Load older messages', disabled: false },
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

it('rerenders a user message when rewind becomes available', () => {
  const message: ChatItem = {
    type: 'user',
    text: 'Revisit this request.',
    rewindId: 'fixture-user-message',
  }
  const sessions: Session[] = []
  const onAction = () => Promise.resolve(true)
  const onOpenInTerminal = () => undefined
  const onSelectDecision = () => undefined
  const onSelectSession = () => undefined
  const onRewind = () => Promise.resolve(true)
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
    onRewind,
  }
  root = createRoot(container)
  act(() => {
    root?.render(<ConversationMessageRow {...rowProps} />)
  })
  act(() => {
    root?.render(
      <ConversationMessageRow
        {...rowProps}
        actions={{ ...actions, rewind: true }}
      />,
    )
  })

  const snapshot = () => ({
    renderCount: renderMessage.mock.calls.length,
    rewindLabel:
      container.querySelector('.chat-rewind-menu summary')?.textContent ?? null,
  })
  expect(snapshot()).toEqual({
    renderCount: 2,
    rewindLabel: 'Rewind',
  })
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
    resolution: null,
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

it.each([
  {
    cardType: 'proposal',
    atBottom: false,
    nearBottom: false,
    resolution: 'completed',
    resolvedSeq: 62,
    cleanupTrigger: 'response',
  },
  {
    cardType: 'proposal',
    atBottom: true,
    nearBottom: false,
    resolution: 'completed',
    resolvedSeq: 62,
    cleanupTrigger: 'response',
  },
  {
    cardType: 'todo',
    atBottom: false,
    nearBottom: false,
    resolution: 'completed',
    resolvedSeq: 62,
    cleanupTrigger: 'response',
  },
  {
    cardType: 'todo',
    atBottom: true,
    nearBottom: false,
    resolution: 'completed',
    resolvedSeq: 62,
    cleanupTrigger: 'response',
  },
  {
    cardType: 'todo',
    atBottom: false,
    nearBottom: false,
    resolution: 'rejected',
    resolvedSeq: 62,
    cleanupTrigger: 'response',
  },
  {
    cardType: 'todo',
    atBottom: true,
    nearBottom: false,
    resolution: 'rejected',
    resolvedSeq: 62,
    cleanupTrigger: 'response',
  },
  {
    cardType: 'proposal',
    atBottom: true,
    nearBottom: false,
    resolution: 'completed',
    resolvedSeq: 63,
    cleanupTrigger: 'response',
  },
  {
    cardType: 'todo',
    atBottom: true,
    nearBottom: false,
    resolution: 'completed',
    resolvedSeq: 63,
    cleanupTrigger: 'response',
  },
  {
    cardType: 'todo',
    atBottom: true,
    nearBottom: false,
    resolution: 'rejected',
    resolvedSeq: 63,
    cleanupTrigger: 'response',
  },
  {
    cardType: 'todo',
    atBottom: false,
    nearBottom: false,
    resolution: 'completed',
    resolvedSeq: 62,
    cleanupTrigger: 'scroll',
  },
  {
    cardType: 'todo',
    atBottom: false,
    nearBottom: false,
    resolution: 'completed',
    resolvedSeq: 62,
    cleanupTrigger: 'response',
    viewportHeight: 560,
    requiresOffscreenCleanup: true,
  },
  {
    cardType: 'todo',
    atBottom: true,
    nearBottom: false,
    resolution: 'completed',
    resolvedSeq: 63,
    cleanupTrigger: 'scroll',
    resolutionScrollDistance: 10,
    requiresBottomPaddingCleanup: true,
  },
  {
    cardType: 'todo',
    atBottom: false,
    nearBottom: true,
    resolution: 'completed',
    resolvedSeq: 62,
    cleanupTrigger: 'response',
  },
] as const)(
  'keeps the remaining $cardType cards in place when resolving card $resolvedSeq as $resolution atBottom=$atBottom nearBottom=$nearBottom cleanup=$cleanupTrigger viewportHeight=$viewportHeight',
  ({
    cardType,
    atBottom,
    nearBottom,
    resolution,
    resolvedSeq,
    cleanupTrigger,
    viewportHeight,
    resolutionScrollDistance: requestedResolutionScrollDistance,
    requiresOffscreenCleanup,
    requiresBottomPaddingCleanup,
  }) => {
    container.style.cssText = 'width: 900px; height: 700px'
    const detail =
      'Review the selected configuration before continuing. '.repeat(10)
    const conversationStart = 5
    const messages: ChatItem[] = [
      { type: 'assistant', text: 'Earlier context. '.repeat(35) },
      ...[1, 2, 3].map((index) =>
        cardType === 'proposal'
          ? {
              ...proposal,
              seq: 60 + index,
              title: `Choose setting ${String(index)}`,
              summary: detail,
            }
          : {
              type: 'todo' as const,
              seq: 60 + index,
              title: `Review setting ${String(index)}`,
              detail,
              completed: false,
              resolution: null,
              comment: null,
            },
      ),
    ]
    renderConversationMessages(messages, conversationStart)

    const scroll = requireElement(
      container.querySelector<HTMLDivElement>('.conversation-scroll'),
    )
    scroll.style.height = `${String(viewportHeight ?? 360)}px`
    scroll.style.flex = 'none'
    renderConversationMessages(messages.slice(), conversationStart)

    const moveScrollWithoutDispatch = (scrollTop: number) => {
      act(() => {
        scroll.scrollTop = scrollTop
      })
    }
    const dispatchScroll = () => {
      void act(() => scroll.dispatchEvent(new Event('scroll')))
    }
    const setScrollPosition = (scrollTop: number) => {
      moveScrollWithoutDispatch(scrollTop)
      dispatchScroll()
    }
    const dispatchUserScrollInput = () => {
      void act(() =>
        scroll.dispatchEvent(
          new WheelEvent('wheel', { bubbles: true, deltaY: -1 }),
        ),
      )
    }
    const bottom = scroll.scrollHeight - scroll.clientHeight
    const scrollPosition = nearBottom
      ? Math.max(0, bottom - 40)
      : Math.min(180, bottom)
    setScrollPosition(atBottom ? bottom : scrollPosition)
    const getPendingCards = () =>
      Array.from(
        scroll.querySelectorAll<HTMLElement>(
          '.chat-proposal-card, .chat-todo-card',
        ),
      )
        .map((element) => {
          const rect = element.getBoundingClientRect()
          return {
            element,
            title: element.querySelector('h3')?.textContent,
            top: Math.round(rect.top),
            height: rect.height,
          }
        })
        .filter(
          ({ title }) =>
            title?.startsWith(
              cardType === 'proposal' ? 'Choose setting' : 'Review setting',
            ) === true,
        )
    const before = getPendingCards().filter(
      ({ title }) =>
        title !==
        `${cardType === 'proposal' ? 'Choose' : 'Review'} setting ${String(resolvedSeq - 60)}`,
    )
    const atBottomNow = () =>
      scroll.scrollHeight - scroll.scrollTop - scroll.clientHeight <= 24
    const wasAtBottom = atBottomNow()
    const scrollBounds = scroll.getBoundingClientRect()
    const intersectsViewport = (top: number, height: number) =>
      top < scrollBounds.bottom && top + height > scrollBounds.top
    const initiallyVisible = before.map(({ top, height }) =>
      intersectsViewport(top, height),
    )
    const getOverlappingChildren = () => {
      const children = Array.from(scroll.children).filter(
        (child): child is HTMLElement => child instanceof HTMLElement,
      )
      const overlappingChildren: string[] = []
      const describeChild = (child: HTMLElement) =>
        child.querySelector('h3')?.textContent ?? child.textContent.slice(0, 40)
      for (const [leftIndex, left] of children.entries()) {
        const leftRect = left.getBoundingClientRect()
        for (const [rightIndex, right] of children.entries()) {
          if (rightIndex <= leftIndex) continue
          const rightRect = right.getBoundingClientRect()
          if (
            leftRect.left < rightRect.right &&
            rightRect.left < leftRect.right &&
            leftRect.top < rightRect.bottom &&
            rightRect.top < leftRect.bottom
          ) {
            overlappingChildren.push(
              `${describeChild(left)}:${String(Math.round(leftRect.top))} overlaps ${describeChild(right)}:${String(Math.round(rightRect.top))}`,
            )
          }
        }
      }
      return overlappingChildren
    }
    const getScrollCompensation = () => ({
      hasChildMarginBlockStart: Array.from(scroll.children).some(
        (child) =>
          child instanceof HTMLElement && child.style.marginBlockStart !== '',
      ),
      hasExtraPaddingBottom:
        scroll.style.getPropertyValue(
          '--conversation-anchor-extra-padding-bottom',
        ) !== '',
    })
    const isResolvedMessage = (
      message: ChatItem,
    ): message is Extract<ChatItem, { type: 'proposal' | 'todo' }> =>
      (message.type === 'proposal' || message.type === 'todo') &&
      message.type === cardType &&
      message.seq === resolvedSeq
    const resolved = messages.find(isResolvedMessage)
    if (resolved === undefined) throw new Error('Expected the card to resolve')
    const settled: ChatItem =
      resolved.type === 'proposal'
        ? { ...resolved, answer: { choice: 0, comment: null } }
        : {
            ...resolved,
            completed: true,
            resolution,
          }
    const isCard = (
      message: ChatItem,
    ): message is Extract<ChatItem, { type: 'proposal' | 'todo' }> =>
      message.type === 'proposal' || message.type === 'todo'
    const remainingCards = messages.filter(
      (message): message is Extract<ChatItem, { type: 'proposal' | 'todo' }> =>
        isCard(message) && !isResolvedMessage(message),
    )
    const surroundingMessages = messages.filter((message) => !isCard(message))
    const originalRemainingTops = before.map(({ top }) => top)
    const stages: number[][] = []
    const stageVisibility: boolean[][] = []
    const bottomStates: boolean[] = []
    const compensationStates: ReturnType<typeof getScrollCompensation>[] = []
    const recordRemainingTops = () => {
      const rects = before.map(({ element }) => element.getBoundingClientRect())
      stages.push(rects.map(({ top }) => Math.round(top)))
      stageVisibility.push(
        rects.map(({ top, height }) => intersectsViewport(top, height)),
      )
      bottomStates.push(atBottomNow())
      compensationStates.push(getScrollCompensation())
    }
    const optimisticMessages = messages.map((message) =>
      isResolvedMessage(message) ? settled : message,
    )
    renderConversationMessages(optimisticMessages, conversationStart)
    recordRemainingTops()
    const afterOptimisticUpdate = getOverlappingChildren()
    renderConversationMessages(
      [...surroundingMessages, settled, ...remainingCards],
      conversationStart,
    )
    recordRemainingTops()
    const afterServerResponse = getOverlappingChildren()
    let userScrollPreservedReadingPosition: boolean | null = null
    let compensationAfterResolutionScroll: ReturnType<
      typeof getScrollCompensation
    > | null = null
    const isScrollCleanup = ['scroll'].includes(cleanupTrigger)
    const extraPaddingAfterServerResponse =
      Number.parseFloat(
        scroll.style.getPropertyValue(
          '--conversation-anchor-extra-padding-bottom',
        ),
      ) || 0
    const requestedScrollDistance =
      requestedResolutionScrollDistance ??
      (atBottom && isScrollCleanup && extraPaddingAfterServerResponse > 0
        ? extraPaddingAfterServerResponse + 32
        : atBottom
          ? 40
          : 20)
    const resolutionScrollDistance = Math.min(
      requestedScrollDistance,
      scroll.scrollTop,
    )
    if (isScrollCleanup) {
      const anchorTitle = before[0]?.title
      const getAnchorTop = () =>
        getPendingCards()
          .find(({ title }) => title === anchorTitle)
          ?.element.getBoundingClientRect().top
      if (atBottom && requestedResolutionScrollDistance !== undefined) {
        moveScrollWithoutDispatch(scroll.scrollHeight - scroll.clientHeight)
      }
      const anchorTopBeforeUserScroll = getAnchorTop()
      const previousScrollTop = scroll.scrollTop
      dispatchUserScrollInput()
      setScrollPosition(previousScrollTop - resolutionScrollDistance)
      compensationAfterResolutionScroll = getScrollCompensation()
      const anchorTopAfterUserScroll = getAnchorTop()
      userScrollPreservedReadingPosition =
        anchorTopBeforeUserScroll !== undefined &&
        anchorTopAfterUserScroll !== undefined &&
        Math.abs(
          anchorTopAfterUserScroll -
            anchorTopBeforeUserScroll -
            resolutionScrollDistance,
        ) <= 0.5
    }
    const notification: ChatItem = {
      type: 'assistant',
      text: 'The selection was recorded.',
    }
    const updatedMessages = [
      ...surroundingMessages,
      settled,
      notification,
      ...remainingCards,
    ]
    renderConversationMessages(updatedMessages, conversationStart)
    recordRemainingTops()
    const afterNotification = getOverlappingChildren()
    const nextResponse: ChatItem = {
      type: 'assistant',
      text: 'The next step is ready.',
    }
    const withNextResponse = [
      ...surroundingMessages,
      settled,
      notification,
      nextResponse,
      ...remainingCards,
    ]
    renderConversationMessages(withNextResponse, conversationStart)
    recordRemainingTops()
    const afterNextResponse = getOverlappingChildren()
    const olderContext: ChatItem = {
      type: 'assistant',
      text: 'Earlier context has loaded.',
    }
    renderConversationMessages(
      [olderContext, ...withNextResponse],
      conversationStart - 1,
    )
    recordRemainingTops()
    const afterDeltaLoad = getOverlappingChildren()
    const remainedAtBottomAfterStages = atBottomNow()
    const compensationBeforeOffscreenScroll = getScrollCompensation()
    const visibleCompensatedCardBeforeOffscreenScroll = before.some(
      ({ element }) => {
        const rect = element.getBoundingClientRect()
        const marginBlockStart =
          Number.parseFloat(element.style.marginBlockStart) || 0
        const compensationTop = rect.top - marginBlockStart
        return (
          element.style.marginBlockStart !== '' &&
          intersectsViewport(compensationTop, marginBlockStart)
        )
      },
    )

    const getVisibleChildren = () =>
      Array.from(scroll.children).flatMap((child, index) => {
        if (!(child instanceof HTMLElement)) return []
        const rect = child.getBoundingClientRect()
        if (!intersectsViewport(rect.top, rect.height)) return []
        return [
          {
            element: child,
            index,
            label:
              child.querySelector('h3')?.textContent ??
              child.textContent.slice(0, 40),
            top: Math.round(rect.top),
          },
        ]
      })
    const offscreenCleanupSteps: {
      visibleChildrenBefore: { index: number; label: string }[]
      visibleChildrenStillVisible: boolean[]
      visibleTopDeltas: (number | null)[]
    }[] = []
    for (let attempt = 0; attempt <= scroll.children.length; attempt += 1) {
      const compensation = getScrollCompensation()
      if (
        !compensation.hasChildMarginBlockStart &&
        !compensation.hasExtraPaddingBottom
      ) {
        break
      }
      const maxScrollTop = scroll.scrollHeight - scroll.clientHeight
      const marginChildren = Array.from(scroll.children).filter(
        (child): child is HTMLElement =>
          child instanceof HTMLElement && child.style.marginBlockStart !== '',
      )
      const extraPaddingBottom =
        Number.parseFloat(
          scroll.style.getPropertyValue(
            '--conversation-anchor-extra-padding-bottom',
          ),
        ) || 0
      const marginScrollTargets = marginChildren.flatMap((child) => {
        const { top, height } = child.getBoundingClientRect()
        const compensation =
          Number.parseFloat(child.style.marginBlockStart) || 0
        return [
          scroll.scrollTop + top - compensation - scrollBounds.bottom - 1,
          scroll.scrollTop + top + height - scrollBounds.top + 1,
        ]
      })
      const maxBeforeBottom = Math.max(0, maxScrollTop - 32)
      const feasibleMarginTarget = marginScrollTargets
        .filter((target) => target >= 0 && target <= maxBeforeBottom)
        .sort(
          (left, right) =>
            Math.abs(left - scroll.scrollTop) -
            Math.abs(right - scroll.scrollTop),
        )[0]
      const targetScrollTop =
        marginChildren.length > 0
          ? (feasibleMarginTarget ?? maxBeforeBottom)
          : Math.max(0, maxScrollTop - extraPaddingBottom - 32)
      dispatchUserScrollInput()
      moveScrollWithoutDispatch(targetScrollTop)
      const visibleBeforeCleanup = getVisibleChildren()
      dispatchScroll()
      const visibleAfterCleanup = getVisibleChildren()
      const visibleAfterByElement = new Map(
        visibleAfterCleanup.map((child) => [child.element, child]),
      )
      offscreenCleanupSteps.push({
        visibleChildrenBefore: visibleBeforeCleanup.map(({ index, label }) => ({
          index,
          label,
        })),
        visibleChildrenStillVisible: visibleBeforeCleanup.map((child) =>
          visibleAfterByElement.has(child.element),
        ),
        visibleTopDeltas: visibleBeforeCleanup.map((child) => {
          const after = visibleAfterByElement.get(child.element)
          return after === undefined ? null : after.top - child.top
        }),
      })
    }
    const afterUserScroll = getOverlappingChildren()
    const compensationAfterOffscreenScroll = getScrollCompensation()
    const expectedBottomAfterResolutionScroll =
      wasAtBottom && (!isScrollCleanup || resolutionScrollDistance <= 24)
    const result = () => ({
      remainingCardCount: before.length,
      remainedAtBottom: remainedAtBottomAfterStages,
      bottomStates,
      userScrollPreservedReadingPosition,
      stageTopDeltas: stages.map((stage) =>
        stage.map((top, cardIndex) => {
          if (initiallyVisible[cardIndex] !== true) return null
          return top - (originalRemainingTops[cardIndex] ?? 0)
        }),
      ),
      stageVisibleStates: stageVisibility.map((stage) =>
        stage.map((visible, cardIndex) =>
          initiallyVisible[cardIndex] === true ? visible : null,
        ),
      ),
      offscreenCleanup: {
        requiredCleanupAssertions:
          requiresOffscreenCleanup === true
            ? {
                compensationPresentBeforeScroll:
                  compensationBeforeOffscreenScroll.hasChildMarginBlockStart ||
                  compensationBeforeOffscreenScroll.hasExtraPaddingBottom,
                compensationRangeVisible:
                  visibleCompensatedCardBeforeOffscreenScroll,
                visibleChildrenObserved: offscreenCleanupSteps.some(
                  ({ visibleChildrenBefore }) =>
                    visibleChildrenBefore.length > 0,
                ),
              }
            : null,
        requiredBottomPaddingCleanup:
          requiresBottomPaddingCleanup === true
            ? {
                paddingPresentAfterServerResponse:
                  compensationStates[1]?.hasExtraPaddingBottom === true,
                paddingRemovedAfterUserScroll:
                  compensationAfterResolutionScroll?.hasExtraPaddingBottom ===
                  resolutionScrollDistance > 24,
                positionPreserved: userScrollPreservedReadingPosition === true,
              }
            : null,
        visibleChildStates: offscreenCleanupSteps.map((step) => ({
          visibleChildrenBefore: step.visibleChildrenBefore,
          visibleChildrenStillVisible: step.visibleChildrenStillVisible,
          visibleTopDeltas: step.visibleTopDeltas,
        })),
        compensation: compensationAfterOffscreenScroll,
      },
      layoutIntegrity: [
        afterOptimisticUpdate,
        afterServerResponse,
        afterNotification,
        afterNextResponse,
        afterDeltaLoad,
        afterUserScroll,
      ],
    })
    const expectedResult = () => ({
      remainingCardCount: 2,
      remainedAtBottom: expectedBottomAfterResolutionScroll,
      bottomStates: [
        wasAtBottom,
        wasAtBottom,
        ...Array.from({ length: 3 }, () => expectedBottomAfterResolutionScroll),
      ],
      userScrollPreservedReadingPosition: isScrollCleanup ? true : null,
      stageTopDeltas: Array.from({ length: 5 }, (_, stageIndex) =>
        originalRemainingTops.map((_, cardIndex) => {
          if (initiallyVisible[cardIndex] !== true) return null
          const expectedScrollDelta =
            isScrollCleanup && stageIndex >= 2
              ? Math.round(resolutionScrollDistance)
              : 0
          return expectedScrollDelta
        }),
      ),
      stageVisibleStates: Array.from({ length: 5 }, (_, stageIndex) =>
        originalRemainingTops.map((_, cardIndex) => {
          if (initiallyVisible[cardIndex] !== true) return null
          const expectedScrollDelta =
            isScrollCleanup && stageIndex >= 2
              ? Math.round(resolutionScrollDistance)
              : 0
          const expectedTop =
            (originalRemainingTops[cardIndex] ?? 0) + expectedScrollDelta
          return intersectsViewport(expectedTop, before[cardIndex]?.height ?? 0)
        }),
      ),
      offscreenCleanup: {
        requiredCleanupAssertions:
          requiresOffscreenCleanup === true
            ? {
                compensationPresentBeforeScroll: true,
                compensationRangeVisible: true,
                visibleChildrenObserved: true,
              }
            : null,
        requiredBottomPaddingCleanup:
          requiresBottomPaddingCleanup === true
            ? {
                paddingPresentAfterServerResponse: true,
                paddingRemovedAfterUserScroll: resolutionScrollDistance > 24,
                positionPreserved: true,
              }
            : null,
        visibleChildStates: offscreenCleanupSteps.map((step) => ({
          visibleChildrenBefore: step.visibleChildrenBefore,
          visibleChildrenStillVisible: step.visibleChildrenBefore.map(
            () => true,
          ),
          visibleTopDeltas: step.visibleChildrenBefore.map(() => 0),
        })),
        compensation: {
          hasChildMarginBlockStart: false,
          hasExtraPaddingBottom: false,
        },
      },
      layoutIntegrity: Array.from({ length: 6 }, () => []),
    })
    expect(result()).toEqual(expectedResult())
  },
)

it.each([
  { formFactor: 'desktop', isMobile: false },
  { formFactor: 'mobile', isMobile: true },
])(
  'keeps reply-waiting cards and todos in the conversation on $formFactor',
  ({ isMobile }) => {
    const pendingQuestionPrompt = {
      header: 'Alert channel',
      question: 'Which alert channel should be enabled?',
      choices: [{ label: 'Email' }, { label: 'Dashboard' }],
      multi_select: false,
      answer: null,
    }
    const messages: ChatItem[] = [
      { type: 'assistant', text: 'The rollout plan is ready.' },
      {
        ...proposal,
        seq: 23,
        title: 'Keep the saved plan',
        answer: { choice: 0, comment: null },
      },
      {
        type: 'question',
        questions: [
          {
            ...pendingQuestionPrompt,
            header: 'Previous retry preference',
            question: 'Which retry preference was selected?',
            answer: 'Email',
          },
        ],
      },
      {
        type: 'todo',
        seq: 24,
        title: 'Confirm the fallback mode',
        detail: null,
        completed: true,
        resolution: 'completed',
        comment: null,
      },
      { type: 'assistant', text: 'The final summary is ready.' },
      {
        ...proposal,
        seq: 21,
        title: 'Choose the rollout interval',
      },
      {
        type: 'question',
        requestId: 'request-rollout-window',
        questions: [pendingQuestionPrompt],
      },
      {
        type: 'permission',
        requestId: 'permission-request-rollout',
        tool: 'Write',
        description: 'Update the rollout configuration.',
      },
      {
        type: 'todo',
        seq: 25,
        title: 'Check the alert destination',
        detail: null,
        completed: false,
        resolution: null,
        comment: null,
      },
    ]
    const answeredMessages = messages.map((message) =>
      message.type === 'proposal' && message.seq === 21
        ? { ...message, answer: { choice: 0, comment: null } }
        : message,
    )
    renderConversationMessages(messages, 0, isMobile)

    const summarizeCards = (root: ParentNode) =>
      Array.from(
        root.querySelectorAll<HTMLElement>(
          '.chat-assistant-message, .chat-question-card, .chat-proposal-card, .chat-permission-card, .chat-todo-card',
        ),
        (element) => {
          if (element.classList.contains('chat-assistant-message')) {
            return `assistant:${element.textContent.trim()}`
          }
          if (element.classList.contains('chat-question-card')) {
            return `question:${element.querySelector('.chat-question-text')?.textContent ?? ''}`
          }
          if (element.classList.contains('chat-proposal-card')) {
            return `proposal:${element.querySelector('h3')?.textContent ?? ''}`
          }
          if (element.classList.contains('chat-permission-card')) {
            return `permission:${element.querySelector('h2')?.textContent ?? ''}`
          }
          return `todo:${element.querySelector('h2')?.textContent ?? ''}:${element.querySelector('h3')?.textContent ?? ''}`
        },
      )
    const pendingActions = container.querySelector(
      '.conversation-pending-actions',
    )
    const composer = container.querySelector('.conversation-composer')
    const timeline = container.querySelector('.conversation-scroll')

    const getPaneSnapshot = () => [
      timeline === null ? [] : summarizeCards(timeline),
      pendingActions,
      composer?.getAttribute('data-mobile') ?? null,
    ]
    const initialSnapshot = getPaneSnapshot()
    renderConversationMessages(answeredMessages, 0, isMobile)
    const resolvedSnapshot = getPaneSnapshot()
    const expectedSnapshots = [
      [
        [
          'assistant:The rollout plan is ready.',
          'proposal:Keep the saved plan',
          'question:Which retry preference was selected?',
          'todo:To-do · done:Confirm the fallback mode',
          'assistant:The final summary is ready.',
          'proposal:Choose the rollout interval',
          'question:Which alert channel should be enabled?',
          'permission:Permission required',
          'todo:To-do · pending:Check the alert destination',
        ],
        null,
        isMobile ? 'true' : null,
      ],
      [
        [
          'assistant:The rollout plan is ready.',
          'proposal:Keep the saved plan',
          'question:Which retry preference was selected?',
          'todo:To-do · done:Confirm the fallback mode',
          'assistant:The final summary is ready.',
          'proposal:Choose the rollout interval',
          'question:Which alert channel should be enabled?',
          'permission:Permission required',
          'todo:To-do · pending:Check the alert destination',
        ],
        null,
        isMobile ? 'true' : null,
      ],
    ]
    const getSnapshots = () => [initialSnapshot, resolvedSnapshot]
    expect(getSnapshots()).toEqual(expectedSnapshots)
  },
)

it.each([
  { formFactor: 'desktop', isMobile: false },
  { formFactor: 'mobile', isMobile: true },
])(
  'keeps incomplete todos in the conversation scroll on $formFactor',
  ({ isMobile }) => {
    const existingMessages: ChatItem[] = [
      {
        type: 'todo',
        seq: 50,
        title: 'Check the first alert channel',
        detail: null,
        completed: false,
        resolution: null,
        comment: null,
      },
    ]
    renderConversationMessages(existingMessages, 0, isMobile)
    renderConversationMessages(
      [
        ...existingMessages,
        {
          type: 'todo',
          seq: 51,
          title: 'Review the selected alert channel',
          detail: null,
          completed: false,
          resolution: null,
          comment: null,
        },
      ],
      0,
      isMobile,
    )

    const snapshot = () => ({
      pendingActions: container.querySelector('.conversation-pending-actions'),
      todos: Array.from(
        container.querySelectorAll('.conversation-scroll .chat-todo-card h3'),
        (heading) => heading.textContent,
      ),
    })
    expect(snapshot()).toEqual({
      pendingActions: null,
      todos: [
        'Check the first alert channel',
        'Review the selected alert channel',
      ],
    })
  },
)

it.each([
  { cardType: 'proposal', formFactor: 'desktop', isMobile: false },
  { cardType: 'proposal', formFactor: 'mobile', isMobile: true },
  { cardType: 'question', formFactor: 'desktop', isMobile: false },
])(
  'keeps an unanswered $cardType in the conversation with a todo on $formFactor',
  ({ cardType, isMobile }) => {
    const answerableMessage: ChatItem =
      cardType === 'proposal'
        ? {
            ...proposal,
            seq: 52,
            title: 'Choose the alert rollout schedule',
          }
        : {
            type: 'question',
            requestId: 'request-alert-channel',
            questions: [
              {
                ...questionPrompt,
                header: 'Alert channel',
                question: 'Which alert channel should be enabled?',
              },
            ],
          }
    renderConversationMessages(
      [
        answerableMessage,
        {
          type: 'todo',
          seq: 53,
          title: 'Confirm the alert rollout schedule',
          detail: null,
          completed: false,
          resolution: null,
          comment: null,
        },
      ],
      0,
      isMobile,
    )

    const snapshot = () => ({
      cards: Array.from(
        container.querySelectorAll(
          '.conversation-scroll .chat-proposal-card, .conversation-scroll .chat-question-card, .conversation-scroll .chat-todo-card',
        ),
        (card) => card.className,
      ),
      timelineTodos: Array.from(
        container.querySelectorAll('.conversation-scroll .chat-todo-card h3'),
        (heading) => heading.textContent,
      ),
    })
    expect(snapshot()).toEqual({
      cards: [
        cardType === 'proposal' ? 'chat-proposal-card' : 'chat-question-card',
        'chat-todo-card',
      ],
      timelineTodos: ['Confirm the alert rollout schedule'],
    })
  },
)

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
