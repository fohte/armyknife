import type { ComponentProps } from 'react'

import { ConversationPane } from '#components/conversation-pane'
import { InboxView } from '#components/inbox-view'
import { ResizableColumns } from '#components/resizable-columns'
import type { Session } from '#types'

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
  onSelectSession,
}: InboxConversationPanesProps) {
  const inboxView = <InboxView {...inboxProps} isMobile={isMobile} />
  const conversationPane =
    selectedSession != null && (!isMobile || showMobileConversation) ? (
      <ConversationPane
        key={selectedSession.id}
        session={selectedSession}
        sessions={sessions}
        refreshKey={refreshKey}
        isMobile={isMobile}
        statusPanelOpen={statusPanelOpen}
        onToggleStatusPanel={onToggleStatusPanel}
        onCloseStatusPanel={onCloseStatusPanel}
        onBack={onBack}
        onLater={() => {
          onLater(selectedSession.id)
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
