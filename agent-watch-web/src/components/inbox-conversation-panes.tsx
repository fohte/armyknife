import type { ComponentProps } from 'react'
import { useCallback, useLayoutEffect, useRef, useState } from 'react'

import { ConversationPane } from '#components/conversation-pane'
import { InboxView } from '#components/inbox-view'
import { ResizableColumns } from '#components/resizable-columns'
import {
  defaultExpandedSessionIds,
  expandSessionAncestors,
  type SessionTreeViewState,
} from '#session-tree-model'
import type { Session, SessionTreeNode } from '#types'

const EMPTY_SESSION_REFRESH_REVISIONS: ReadonlyMap<string, number> = new Map()

type SessionTreeCacheEntry = {
  sessions: readonly SessionTreeNode[]
  refreshKey: number
  requestSequence: number
  refreshError: string | null
  viewState: SessionTreeViewState
}

function treeRefreshKey(
  tree: readonly SessionTreeNode[],
  sessionRefreshRevisions: ReadonlyMap<string, number>,
  sessions: readonly Session[],
) {
  const treeSessionIds = new Set(tree.map((node) => node.id))
  const treeRevision = tree.reduce(
    (latestRevision, node) =>
      Math.max(latestRevision, sessionRefreshRevisions.get(node.id) ?? 0),
    0,
  )
  return sessions.reduce(
    (latestRevision, session) =>
      session.ancestorSessionIds.some((ancestorId) =>
        treeSessionIds.has(ancestorId),
      )
        ? Math.max(latestRevision, sessionRefreshRevisions.get(session.id) ?? 0)
        : latestRevision,
    treeRevision,
  )
}

function loadedTreeRefreshKey(
  tree: readonly SessionTreeNode[],
  requestRefreshKey: number,
  sessionRefreshRevisions: ReadonlyMap<string, number>,
  sessions: readonly Session[],
) {
  const currentTreeRefreshKey = treeRefreshKey(
    tree,
    sessionRefreshRevisions,
    sessions,
  )
  if (currentTreeRefreshKey > requestRefreshKey) {
    return -1
  }
  return currentTreeRefreshKey
}

function replaceTreeCacheEntry(
  current: ReadonlyMap<string, SessionTreeCacheEntry>,
  existing: SessionTreeCacheEntry,
  replacement: SessionTreeCacheEntry,
) {
  const next = new Map(current)
  for (const node of existing.sessions) {
    if (current.get(node.id) === existing) next.set(node.id, replacement)
  }
  return next
}

type InboxConversationPanesProps = {
  inboxProps: Omit<ComponentProps<typeof InboxView>, 'isMobile'>
  sessions?: readonly Session[]
  selectedSession: Session | null | undefined
  refreshKey: number
  sessionRefreshRevisions?: ReadonlyMap<string, number>
  isMobile: boolean
  showMobileConversation: boolean
  inboxCollapsed?: boolean
  statusPanelOpen?: boolean
  onToggleStatusPanel?: () => void
  onCloseStatusPanel?: () => void
  onBack: () => void
  onLater: (id: string) => void
  onCloseSession: (id: string, keepWorktree?: boolean) => void
  onSelectSession: (id: string) => void
}

export function InboxConversationPanes({
  inboxProps,
  sessions = [],
  selectedSession,
  refreshKey,
  sessionRefreshRevisions: providedSessionRefreshRevisions,
  isMobile,
  showMobileConversation,
  inboxCollapsed = false,
  statusPanelOpen = false,
  onToggleStatusPanel = () => undefined,
  onCloseStatusPanel = () => undefined,
  onBack,
  onLater,
  onCloseSession,
  onSelectSession,
}: InboxConversationPanesProps) {
  const hasScopedSessionRefreshRevisions =
    providedSessionRefreshRevisions !== undefined
  const sessionRefreshRevisions =
    providedSessionRefreshRevisions ?? EMPTY_SESSION_REFRESH_REVISIONS
  const [sessionTrees, setSessionTrees] = useState<
    ReadonlyMap<string, SessionTreeCacheEntry>
  >(() => new Map())
  const nextTreeRequestSequence = useRef(0)
  const pendingTreeRefreshes = useRef(
    new Map<string, { refreshKey: number; requestSequence: number }>(),
  )
  const sessionRefreshRevisionsRef = useRef(sessionRefreshRevisions)
  sessionRefreshRevisionsRef.current = sessionRefreshRevisions
  const sessionsRef = useRef(sessions)
  sessionsRef.current = sessions
  const scopedRefreshEnabledRef = useRef(hasScopedSessionRefreshRevisions)
  scopedRefreshEnabledRef.current = hasScopedSessionRefreshRevisions
  const previousSelectedSessionId = useRef(selectedSession?.id)
  const cacheSessionTree = useCallback(
    (
      tree: readonly SessionTreeNode[],
      selectedSessionId: string,
      requestRefreshKey: number,
      requestSequence: number,
    ) => {
      if (
        pendingTreeRefreshes.current.get(selectedSessionId)?.requestSequence ===
        requestSequence
      ) {
        pendingTreeRefreshes.current.delete(selectedSessionId)
      }
      const cachedRefreshKey = scopedRefreshEnabledRef.current
        ? loadedTreeRefreshKey(
            tree,
            requestRefreshKey,
            sessionRefreshRevisionsRef.current,
            sessionsRef.current,
          )
        : requestRefreshKey
      setSessionTrees((current) => {
        if (
          tree.some((node) => {
            const cached = current.get(node.id)
            return (
              cached !== undefined &&
              (cached.refreshKey > requestRefreshKey ||
                cached.requestSequence > requestSequence)
            )
          })
        ) {
          return current
        }
        const existing = current.get(selectedSessionId)
        const nodeIds = new Set(tree.map((node) => node.id))
        const viewState =
          existing === undefined
            ? {
                expandedSessionIds: [
                  ...defaultExpandedSessionIds(tree, selectedSessionId),
                ],
                focusedSessionId: selectedSessionId,
                treeHasFocus: false,
              }
            : {
                expandedSessionIds:
                  existing.viewState.expandedSessionIds.filter((id) =>
                    nodeIds.has(id),
                  ),
                focusedSessionId: nodeIds.has(
                  existing.viewState.focusedSessionId,
                )
                  ? existing.viewState.focusedSessionId
                  : selectedSessionId,
                treeHasFocus: existing.viewState.treeHasFocus,
              }
        const entry: SessionTreeCacheEntry = {
          sessions: tree,
          refreshKey: cachedRefreshKey,
          requestSequence,
          refreshError: null,
          viewState,
        }
        const next = new Map(current)
        if (existing !== undefined) {
          for (const node of existing.sessions) {
            if (current.get(node.id) === existing) next.delete(node.id)
          }
        }
        for (const node of tree) next.set(node.id, entry)
        return next
      })
    },
    [],
  )
  const markTreeRefreshStarted = useCallback(
    (selectedSessionId: string, requestRefreshKey: number) => {
      const requestSequence = nextTreeRequestSequence.current + 1
      nextTreeRequestSequence.current = requestSequence
      pendingTreeRefreshes.current.set(selectedSessionId, {
        refreshKey: requestRefreshKey,
        requestSequence,
      })
      setSessionTrees((current) => {
        const existing = current.get(selectedSessionId)
        if (
          existing !== undefined &&
          existing.refreshKey <= requestRefreshKey
        ) {
          const replacement = {
            ...existing,
            refreshKey: requestRefreshKey,
            requestSequence,
            refreshError: null,
          }
          return replaceTreeCacheEntry(current, existing, replacement)
        }
        return current
      })
      return requestSequence
    },
    [],
  )
  const recordTreeRefreshFailure = useCallback(
    (
      selectedSessionId: string,
      requestRefreshKey: number,
      requestSequence: number,
      message: string,
    ) => {
      if (
        pendingTreeRefreshes.current.get(selectedSessionId)?.requestSequence ===
        requestSequence
      ) {
        pendingTreeRefreshes.current.delete(selectedSessionId)
      }
      setSessionTrees((current) => {
        const existing = current.get(selectedSessionId)
        if (
          existing === undefined ||
          existing.refreshKey > requestRefreshKey ||
          existing.requestSequence > requestSequence
        ) {
          return current
        }
        const entry = {
          ...existing,
          refreshKey: requestRefreshKey,
          requestSequence,
          refreshError: message,
        }
        return replaceTreeCacheEntry(current, existing, entry)
      })
    },
    [],
  )
  const updateTreeViewState = useCallback(
    (selectedSessionId: string, viewState: SessionTreeViewState) => {
      setSessionTrees((current) => {
        const existing = current.get(selectedSessionId)
        if (existing === undefined) return current
        const entry = { ...existing, viewState }
        return replaceTreeCacheEntry(current, existing, entry)
      })
    },
    [],
  )
  const inboxView = <InboxView {...inboxProps} isMobile={isMobile} />
  const selectedTreeCache =
    selectedSession == null ? undefined : sessionTrees.get(selectedSession.id)
  const selectedTreeRefreshKey =
    selectedTreeCache === undefined
      ? (pendingTreeRefreshes.current.get(selectedSession?.id ?? '')
          ?.refreshKey ?? refreshKey)
      : hasScopedSessionRefreshRevisions
        ? treeRefreshKey(
            selectedTreeCache.sessions,
            sessionRefreshRevisions,
            sessions,
          )
        : refreshKey
  const skipSessionTreeFetch =
    selectedTreeCache !== undefined &&
    selectedTreeCache.refreshKey === selectedTreeRefreshKey
  useLayoutEffect(() => {
    const selectedSessionId = selectedSession?.id
    if (previousSelectedSessionId.current === selectedSessionId) return
    previousSelectedSessionId.current = selectedSessionId
    if (selectedSessionId === undefined) return
    setSessionTrees((current) => {
      const existing = current.get(selectedSessionId)
      if (existing === undefined) return current
      const viewState = expandSessionAncestors(
        existing.sessions,
        selectedSessionId,
        existing.viewState,
      )
      if (viewState === existing.viewState) return current
      return replaceTreeCacheEntry(current, existing, {
        ...existing,
        viewState,
      })
    })
  }, [selectedSession?.id])
  const conversationPane =
    selectedSession != null && (!isMobile || showMobileConversation) ? (
      <ConversationPane
        key={selectedSession.id}
        session={selectedSession}
        sessions={sessions}
        initialSessionTree={selectedTreeCache?.sessions ?? null}
        initialSessionTreeError={selectedTreeCache?.refreshError ?? null}
        initialSessionTreeViewState={selectedTreeCache?.viewState ?? null}
        shouldSkipSessionTreeFetch={skipSessionTreeFetch}
        sessionTreeRefreshKey={selectedTreeRefreshKey}
        onSessionTreeLoaded={cacheSessionTree}
        onSessionTreeRequestStarted={markTreeRefreshStarted}
        onSessionTreeRequestFailed={recordTreeRefreshFailure}
        onSessionTreeViewStateChange={updateTreeViewState}
        refreshKey={
          hasScopedSessionRefreshRevisions
            ? (sessionRefreshRevisions.get(selectedSession.id) ?? 0)
            : refreshKey
        }
        isMobile={isMobile}
        statusPanelOpen={statusPanelOpen}
        onToggleStatusPanel={onToggleStatusPanel}
        onCloseStatusPanel={onCloseStatusPanel}
        onBack={onBack}
        onLater={() => {
          onLater(selectedSession.id)
        }}
        onClose={() => {
          onCloseSession(selectedSession.id)
        }}
        isToday={selectedSession.isToday === true}
        onToggleToday={(isToday) => {
          inboxProps.onToggleToday(selectedSession.id, isToday)
        }}
        onSelectSession={onSelectSession}
      />
    ) : null

  if (isMobile) {
    return (
      <>
        {inboxView}
        {conversationPane}
      </>
    )
  }

  return (
    <ResizableColumns
      first={inboxView}
      second={conversationPane}
      firstCollapsed={inboxCollapsed}
      storageKey="agent-watch:inbox-width"
      firstSize={460}
      firstMinSize={200}
      secondMinSize={300}
      label="Resize session list and conversation"
      fillFirstPaneWhenAlone={false}
    />
  )
}
