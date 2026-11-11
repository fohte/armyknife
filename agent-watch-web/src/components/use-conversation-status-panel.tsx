import { useEffect, useState } from 'react'

import { CritPane } from '#components/crit-pane'
import type { Decision } from '#components/decision-card'
import { DecisionsPanel } from '#components/decisions-panel'
import { RunningTasksPanel } from '#components/running-tasks-panel'
import type { SessionStatusPanelTab } from '#components/session-status-panel'
import type { BackgroundTask, ChatItem, Session } from '#types'

type UseConversationStatusPanelOptions = {
  session: Session
  messages: readonly ChatItem[]
  allDecisions?: readonly Decision[]
  backgroundTasks: readonly BackgroundTask[]
  statusPanelOpen: boolean
  onAction: (
    action: string,
    payload?: Record<string, unknown>,
  ) => Promise<boolean>
  onCloseStatusPanel: () => void
  onToggleStatusPanel: () => void
}

export function useConversationStatusPanel({
  session,
  messages,
  allDecisions,
  backgroundTasks,
  statusPanelOpen,
  onAction,
  onCloseStatusPanel,
  onToggleStatusPanel,
}: UseConversationStatusPanelOptions) {
  const [activePanelTabId, setActivePanelTabId] = useState<string | null>(null)
  const [selectedDecisionSeq, setSelectedDecisionSeq] = useState<number | null>(
    null,
  )
  const decisions =
    allDecisions ??
    messages.filter(
      (message): message is Decision => message.type === 'decision',
    )
  const decisionCount = session.decisionCount ?? decisions.length
  const selectedDecision =
    decisions.find((decision) => decision.seq === selectedDecisionSeq) ??
    decisions.find((decision) => decision.review === null) ??
    decisions[0]
  const critUrl =
    session.kind === 'Review' &&
    session.category === 'needs_action' &&
    session.critUrl != null &&
    session.critUrl !== ''
      ? session.critUrl
      : null
  const tabs: readonly SessionStatusPanelTab[] = [
    ...(backgroundTasks.length > 0
      ? [
          {
            id: 'running',
            label: 'Running',
            count: backgroundTasks.length,
            content: (
              <RunningTasksPanel
                sessionId={session.id}
                tasks={backgroundTasks}
              />
            ),
          },
        ]
      : []),
    ...(decisionCount > 0
      ? [
          {
            id: 'decisions',
            label: '◇ Decisions',
            count: decisionCount,
            content: (
              <DecisionsPanel
                decisions={decisions}
                session={session}
                selectedSeq={selectedDecision?.seq ?? null}
                onSelect={setSelectedDecisionSeq}
                onOk={(seq) => onAction(`decisions/${String(seq)}/ok`)}
                onChange={(seq, comment) =>
                  onAction(`decisions/${String(seq)}/change`, { comment })
                }
              />
            ),
          },
        ]
      : []),
    ...(critUrl === null
      ? []
      : [
          {
            id: 'crit',
            label: 'crit',
            count: 1,
            content: <CritPane url={critUrl} onClose={onCloseStatusPanel} />,
          },
        ]),
  ]
  const defaultPanelTabId =
    tabs.find((tab) => tab.id === 'crit')?.id ??
    tabs.find((tab) => tab.id === 'running')?.id ??
    tabs[0]?.id ??
    null
  const activeTabId = tabs.some((tab) => tab.id === activePanelTabId)
    ? activePanelTabId
    : defaultPanelTabId
  useEffect(() => {
    if (activePanelTabId !== activeTabId) setActivePanelTabId(activeTabId)
  }, [activePanelTabId, activeTabId])
  const decisionsPanelOpen = statusPanelOpen && activeTabId === 'decisions'
  const onToggleDecisionsPanel = () => {
    if (decisionsPanelOpen) {
      onToggleStatusPanel()
      return
    }
    setActivePanelTabId('decisions')
    if (!statusPanelOpen) onToggleStatusPanel()
  }

  return {
    activeTabId,
    decisionsPanelOpen,
    onSelectDecision: setSelectedDecisionSeq,
    onTabChange: setActivePanelTabId,
    onToggleDecisionsPanel,
    selectedDecisionSeq: selectedDecision?.seq ?? null,
    tabs,
  }
}
