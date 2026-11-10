import type { ComponentProps } from 'react'

import { ConversationPane } from '#components/conversation-pane'
import { InboxView } from '#components/inbox-view'
import { ResizableColumns } from '#components/resizable-columns'
import type { Session } from '#types'

type InboxConversationPanesProps = {
  inboxProps: Omit<ComponentProps<typeof InboxView>, 'isMobile'>
  selectedSession: Session | null | undefined
  refreshKey: number
  isMobile: boolean
  showMobileConversation: boolean
  onBack: () => void
  onLater: (id: string) => void
  onSelectSession: (id: string) => void
}

export function InboxConversationPanes({
  inboxProps,
  selectedSession,
  refreshKey,
  isMobile,
  showMobileConversation,
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
        refreshKey={refreshKey}
        isMobile={isMobile}
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
      storageKey="agent-watch:inbox-width"
      firstSize={460}
      firstMinSize={200}
      secondMinSize={300}
      label="Resize session list and conversation"
      fillFirstPaneWhenAlone={false}
    />
  )
}
