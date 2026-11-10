import type { ComponentProps } from 'react'
import { useState } from 'react'

import { ConversationPane } from '#components/conversation-pane'
import { CritPane } from '#components/crit-pane'
import { InboxView } from '#components/inbox-view'
import { ResizableColumns } from '#components/resizable-columns'
import type { SessionStatusPanelTab } from '#components/session-status-panel'
import { SessionStatusPanel } from '#components/session-status-panel'
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
  const [activePanelTabId, setActivePanelTabId] = useState<string | null>(null)
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

  const critUrl =
    selectedSession?.kind === 'Review' &&
    selectedSession.category === 'needs_action' &&
    selectedSession.critUrl != null &&
    selectedSession.critUrl !== ''
      ? selectedSession.critUrl
      : null
  const panelTabs: readonly SessionStatusPanelTab[] =
    critUrl === null
      ? []
      : [
          {
            id: 'crit',
            label: 'crit',
            count: 1,
            content: <CritPane url={critUrl} onClose={onCloseStatusPanel} />,
          },
        ]
  const statusPanel = statusPanelOpen ? (
    <SessionStatusPanel
      tabs={panelTabs}
      activeTabId={
        panelTabs.some((tab) => tab.id === activePanelTabId)
          ? activePanelTabId
          : (panelTabs[0]?.id ?? null)
      }
      onTabChange={setActivePanelTabId}
      onClose={onCloseStatusPanel}
    />
  ) : null
  const conversationAndPanel =
    conversationPane === null ? null : statusPanel === null ? (
      conversationPane
    ) : (
      <div className="conversation-status-panel-container">
        <ResizableColumns
          first={conversationPane}
          second={statusPanel}
          overlaySecondPane
          storageKey="agent-watch:conversation-width"
          firstSize={560}
          firstMinSize={240}
          secondMinSize={300}
          label="Resize conversation and status panel"
        />
      </div>
    )

  return (
    <ResizableColumns
      first={inboxView}
      second={conversationAndPanel}
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
