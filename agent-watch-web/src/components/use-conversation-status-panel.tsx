import { useEffect, useState } from 'react'

import { CritPane } from '#components/crit-pane'
import type { Decision } from '#components/decision-card'
import { DecisionsPanel } from '#components/decisions-panel'
import { RunningTasksPanel } from '#components/running-tasks-panel'
import type { SessionStatusPanelTab } from '#components/session-status-panel'
import { SessionTreePanel } from '#components/session-tree-panel'
import { needsActionDescendantCount } from '#session-tree-model'
import type { BackgroundTask, ChatItem, Session, SessionTreeNode } from '#types'

type UseConversationStatusPanelOptions = {
  session: Session
  messages: readonly ChatItem[]
  allDecisions?: readonly Decision[]
  backgroundTasks: readonly BackgroundTask[]
  sessionTree?: readonly SessionTreeNode[] | null
  sessionTreeError?: string | null
  onSelectSession?: (id: string) => void
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
  sessionTree = null,
  sessionTreeError = null,
  onSelectSession = () => undefined,
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
  const treeCount =
    sessionTree === null
      ? 0
      : needsActionDescendantCount(sessionTree, session.id)
  const treeAvailable =
    session.hasSessionTree === true &&
    (sessionTreeError !== null ||
      sessionTree === null ||
      sessionTree.length > 1)
  const tabs: readonly SessionStatusPanelTab[] = [
    ...(backgroundTasks.length > 0
      ? [
          {
            id: 'running',
            label: 'Running',
            count: backgroundTasks.length,
            content: <RunningTasksPanel tasks={backgroundTasks} />,
          },
        ]
      : []),
    ...(!treeAvailable
      ? []
      : [
          {
            id: 'tree',
            label: 'Tree',
            count: treeCount,
            visible: true,
            showCount: treeCount > 0,
            countTone: 'warning' as const,
            content:
              sessionTreeError !== null ? (
                <div className="session-tree-feedback" role="alert">
                  Could not load the session tree: {sessionTreeError}
                </div>
              ) : sessionTree === null ? (
                <div className="session-tree-feedback" role="status">
                  Loading session tree…
                </div>
              ) : (
                <SessionTreePanel
                  sessions={sessionTree}
                  selectedSessionId={session.id}
                  onSelectSession={onSelectSession}
                />
              ),
          },
        ]),
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
