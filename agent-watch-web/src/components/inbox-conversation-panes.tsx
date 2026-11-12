import type { ComponentProps } from 'react'
import { useCallback, useState } from 'react'

import { ConversationPane } from '#components/conversation-pane'
import { InboxView } from '#components/inbox-view'
import { ResizableColumns } from '#components/resizable-columns'
import type { Session, SessionTreeNode } from '#types'

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
    ReadonlyMap<string, readonly SessionTreeNode[]>
  >(() => new Map())
  const cacheSessionTree = useCallback((tree: readonly SessionTreeNode[]) => {
    setSessionTrees((current) => {
      const next = new Map(current)
      for (const node of tree) next.set(node.id, tree)
      return next
    })
  }, [])
  const inboxView = <InboxView {...inboxProps} isMobile={isMobile} />
  const conversationPane =
    selectedSession != null && (!isMobile || showMobileConversation) ? (
      <ConversationPane
        key={selectedSession.id}
        session={selectedSession}
        sessions={sessions}
        initialSessionTree={sessionTrees.get(selectedSession.id) ?? null}
        onSessionTreeLoaded={cacheSessionTree}
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
