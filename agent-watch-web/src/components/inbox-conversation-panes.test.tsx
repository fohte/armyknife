import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'

import { InboxConversationPanes } from '#components/inbox-conversation-panes'
import { SessionConversationCacheProvider } from '#components/session-conversation-cache'
import type { Session, SessionActions, SessionTreeNode } from '#types'

const actions: SessionActions = {
  message: true,
  answerQuestion: true,
  permission: { approve: true, reject: true },
  interrupt: true,
  openInTerminal: true,
  rewind: false,
}

let container: HTMLDivElement
let root: Root | null = null

function jsonResponse(body: unknown) {
  return new Response(JSON.stringify(body), {
    headers: { 'Content-Type': 'application/json' },
  })
}

function requestUrl(input: RequestInfo | URL) {
  if (typeof input === 'string') return input
  return input instanceof URL ? input.href : input.url
}

function treeSession(
  id: string,
  title: string,
  parent: Session | null = null,
): Session {
  return {
    id,
    category: 'in_progress',
    isPaused: false,
    kind: null,
    title,
    preview: null,
    elapsedSeconds: 0,
    unread: false,
    task: null,
    repo: 'sample/repo',
    project: 'sample project',
    parent: parent === null ? null : { id: parent.id, title: parent.title },
    hasSessionTree: true,
    ancestorSessionIds:
      parent === null ? [] : [...parent.ancestorSessionIds, parent.id],
    pullRequest: null,
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
    repo: 'sample/repo',
    vibeMode: { mode: 'disabled' },
    pullRequest: null,
  }
}

async function waitForState(condition: () => boolean) {
  await act(async () => {
    const deadline = Date.now() + 1000
    while (!condition() && Date.now() < deadline) {
      await new Promise((resolve) => window.setTimeout(resolve, 10))
    }
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

function inboxFetch(respondToTreeRequest: (url: string) => Promise<Response>) {
  return vi.fn<typeof fetch>((input) => {
    const url = requestUrl(input)
    if (url.endsWith('/messages?limit=100')) {
      return Promise.resolve(
        jsonResponse({
          messages: [],
          backgroundTasks: [],
          start: 0,
          total: 0,
          hasUnansweredProposal: false,
          activeQuestionIndex: null,
        }),
      )
    }
    if (url.endsWith('/actions')) return Promise.resolve(jsonResponse(actions))
    if (url.endsWith('/tree')) return respondToTreeRequest(url)
    return Promise.resolve(jsonResponse({}))
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

it('opens the selected session when it has a collapsed child', async () => {
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
  const sessions = [rootSession, childSession, grandchildSession]
  const tree = [
    treeNode(rootSession.id, null, rootSession.title),
    treeNode(childSession.id, rootSession.id, childSession.title),
    treeNode(grandchildSession.id, childSession.id, grandchildSession.title),
  ]
  vi.stubGlobal(
    'fetch',
    inboxFetch(() =>
      Promise.resolve(jsonResponse({ revision: 1, sessions: tree })),
    ),
  )

  renderInboxPanes(sessions, rootSession)
  await waitForState(
    () =>
      container.querySelector('#session-tree-row-grandchild-session') !== null,
  )
  const collapseButton = container.querySelector<HTMLButtonElement>(
    '[aria-label="Collapse Child session"]',
  )
  if (collapseButton === null) throw new Error('Expected a collapse button')
  act(() => {
    collapseButton.click()
  })

  const collapsedSnapshot = () => ({
    childExpanded: container
      .querySelector('#session-tree-row-child-session')
      ?.getAttribute('aria-expanded'),
    grandchildRow:
      container.querySelector('#session-tree-row-grandchild-session') !== null,
  })
  expect(collapsedSnapshot()).toEqual({
    childExpanded: 'false',
    grandchildRow: false,
  })

  renderInboxPanes(sessions, childSession)

  const expandedSnapshot = () => ({
    selectedRow: container.querySelector('.session-tree-row.is-current')?.id,
    childExpanded: container
      .querySelector('#session-tree-row-child-session')
      ?.getAttribute('aria-expanded'),
    grandchildRow:
      container.querySelector('#session-tree-row-grandchild-session') !== null,
  })
  expect(expandedSnapshot()).toEqual({
    selectedRow: 'session-tree-row-child-session',
    childExpanded: 'true',
    grandchildRow: true,
  })
})

it('shows a child added after selecting the session', async () => {
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
  const sessions = [rootSession, childSession, grandchildSession]
  const initialTree = [
    treeNode(rootSession.id, null, rootSession.title),
    treeNode(childSession.id, rootSession.id, childSession.title),
  ]
  const updatedTree = [
    ...initialTree,
    treeNode(grandchildSession.id, childSession.id, grandchildSession.title),
  ]
  const fetcher = inboxFetch((url) =>
    Promise.resolve(
      jsonResponse({
        revision: 1,
        sessions: url.endsWith(`/${rootSession.id}/tree`)
          ? initialTree
          : updatedTree,
      }),
    ),
  )
  vi.stubGlobal('fetch', fetcher)

  renderInboxPanes(sessions, rootSession)
  await waitForState(
    () => container.querySelector('#session-tree-row-child-session') !== null,
  )
  renderInboxPanes(sessions, childSession)
  renderInboxPanes(sessions, childSession, new Map([[childSession.id, 1]]))
  await waitForState(
    () =>
      container.querySelector('#session-tree-row-grandchild-session') !== null,
  )

  const snapshot = () => ({
    childExpanded: container
      .querySelector('#session-tree-row-child-session')
      ?.getAttribute('aria-expanded'),
    grandchildRow:
      container.querySelector('#session-tree-row-grandchild-session') !== null,
    treeRequests: fetcher.mock.calls
      .map(([input]) => requestUrl(input))
      .filter((url) => url.endsWith('/tree')),
  })
  expect(snapshot()).toEqual({
    childExpanded: 'true',
    grandchildRow: true,
    treeRequests: [
      '/api/sessions/root-session/tree',
      '/api/sessions/child-session/tree',
    ],
  })
})
