import type { ComponentProps } from 'react'
import { useCallback, useRef, useState } from 'react'

import { ConversationPane } from '#components/conversation-pane'
import { InboxView } from '#components/inbox-view'
import { ResizableColumns } from '#components/resizable-columns'
import {
  defaultExpandedSessionIds,
  type SessionTreeViewState,
} from '#session-tree-model'
import type { Session, SessionTreeNode } from '#types'

type SessionTreeCacheEntry = {
  sessions: readonly SessionTreeNode[]
  refreshKey: number
  requestSequence: number
  refreshError: string | null
  viewState: SessionTreeViewState
}

type InboxConversationPanesProps = {
  inboxProps: Omit<ComponentProps<typeof InboxView>, 'isMobile'>
  sessions?: readonly Session[]
  selectedSession: Session | null | undefined
  refreshKey: number
  isMobile: boolean
  showMobileConversation: boolean
  inboxCollapsed?: boolean
  statusPanelOpen?: boolean
  onToggleStatusPanel?: () => void
  onCloseStatusPanel?: () => void
  onBack: () => void
  onLater: (id: string) => void
  onArchiveSession: (id: string) => void
  onSelectSession: (id: string) => void
}

export function InboxConversationPanes({
  inboxProps,
  sessions = [],
  selectedSession,
  refreshKey,
  isMobile,
  showMobileConversation,
  inboxCollapsed = false,
  statusPanelOpen = false,
  onToggleStatusPanel = () => undefined,
  onCloseStatusPanel = () => undefined,
  onBack,
  onLater,
  onArchiveSession,
  onSelectSession,
}: InboxConversationPanesProps) {
  const [sessionTrees, setSessionTrees] = useState<
    ReadonlyMap<string, SessionTreeCacheEntry>
  >(() => new Map())
  const nextTreeRequestSequence = useRef(0)
  const cacheSessionTree = useCallback(
    (
      tree: readonly SessionTreeNode[],
      selectedSessionId: string,
      requestRefreshKey: number,
      requestSequence: number,
    ) => {
      setSessionTrees((current) => {
        const existing = current.get(selectedSessionId)
        if (
          existing !== undefined &&
          (existing.refreshKey > requestRefreshKey ||
            existing.requestSequence > requestSequence)
        ) {
          return current
        }
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
          refreshKey: requestRefreshKey,
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
      setSessionTrees((current) => {
        const existing = current.get(selectedSessionId)
        if (
          existing !== undefined &&
          existing.refreshKey <= requestRefreshKey
        ) {
          const entry = {
            ...existing,
            refreshKey: requestRefreshKey,
            requestSequence,
            refreshError: null,
          }
          const next = new Map(current)
          for (const node of existing.sessions) {
            if (current.get(node.id) === existing) next.set(node.id, entry)
          }
          return next
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
        const next = new Map(current)
        for (const node of existing.sessions) {
          if (current.get(node.id) === existing) next.set(node.id, entry)
        }
        return next
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
        const next = new Map(current)
        for (const node of existing.sessions) {
          if (current.get(node.id) === existing) next.set(node.id, entry)
        }
        return next
      })
    },
    [],
  )
  const inboxView = <InboxView {...inboxProps} isMobile={isMobile} />
  const selectedTreeCache =
    selectedSession == null ? undefined : sessionTrees.get(selectedSession.id)
  const skipSessionTreeFetch =
    selectedTreeCache !== undefined &&
    selectedTreeCache.refreshKey === refreshKey
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
        onSessionTreeLoaded={cacheSessionTree}
        onSessionTreeRequestStarted={markTreeRefreshStarted}
        onSessionTreeRequestFailed={recordTreeRefreshFailure}
        onSessionTreeViewStateChange={updateTreeViewState}
        refreshKey={refreshKey}
        isMobile={isMobile}
        statusPanelOpen={statusPanelOpen}
        onToggleStatusPanel={onToggleStatusPanel}
        onCloseStatusPanel={onCloseStatusPanel}
        onBack={onBack}
        onLater={() => {
          onLater(selectedSession.id)
        }}
        onArchive={() => {
          onArchiveSession(selectedSession.id)
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
