import { useEffect, useState } from 'react'

import { CritPane } from '#components/crit-pane'
import type { Decision } from '#components/decision-card'
import { DecisionsPanel } from '#components/decisions-panel'
import { PullRequestListPanel } from '#components/pull-request-list-panel'
import { RunningTasksPanel } from '#components/running-tasks-panel'
import type { SessionStatusPanelTab } from '#components/session-status-panel'
import { SessionTreePanel } from '#components/session-tree-panel'
import { useSessionPullRequests } from '#components/use-session-pull-requests'
import {
  needsActionDescendantCount,
  type SessionTreeViewState,
} from '#session-tree-model'
import type { BackgroundTask, ChatItem, Session, SessionTreeNode } from '#types'

type UseConversationStatusPanelOptions = {
  session: Session
  messages: readonly ChatItem[]
  allDecisions?: readonly Decision[]
  backgroundTasks: readonly BackgroundTask[]
  sessionTree?: readonly SessionTreeNode[] | null
  sessionTreeError?: string | null
  sessionTreeViewState?: SessionTreeViewState | null
  onSessionTreeViewStateChange?: (state: SessionTreeViewState) => void
  onSelectSession?: (id: string) => void
  statusPanelOpen?: boolean
  onAction: (
    action: string,
    payload?: Record<string, unknown>,
  ) => Promise<boolean>
  onCloseStatusPanel: () => void
}

export function useConversationStatusPanel({
  session,
  messages,
  allDecisions,
  backgroundTasks,
  sessionTree = null,
  sessionTreeError = null,
  sessionTreeViewState = null,
  onSessionTreeViewStateChange,
  onSelectSession = () => undefined,
  statusPanelOpen = false,
  onAction,
  onCloseStatusPanel,
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
  const pullRequestListAvailable =
    session.task !== null || session.hasSessionTree === true
  const unreviewedDecisionCount =
    session.unreviewedDecisionCount ??
    decisions.filter((decision) => decision.review === null).length
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
  const availableTabIds = [
    ...(backgroundTasks.length === 0 ? [] : ['running']),
    ...(treeAvailable ? ['tree'] : []),
    ...(pullRequestListAvailable ? ['pull-requests'] : []),
    ...(decisionCount === 0 ? [] : ['decisions']),
    ...(critUrl === null ? [] : ['crit']),
  ]
  const defaultPanelTabId = availableTabIds.includes('crit')
    ? 'crit'
    : availableTabIds.includes('running')
      ? 'running'
      : (availableTabIds[0] ?? null)
  const activeTabId =
    activePanelTabId !== null && availableTabIds.includes(activePanelTabId)
      ? activePanelTabId
      : defaultPanelTabId
  const pullRequestList = useSessionPullRequests({
    session,
    enabled:
      statusPanelOpen &&
      pullRequestListAvailable &&
      activeTabId === 'pull-requests',
  })
  const tabs: readonly SessionStatusPanelTab[] = [
    ...(availableTabIds.includes('running')
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
    ...(availableTabIds.includes('tree')
      ? [
          {
            id: 'tree',
            label: 'Tree',
            count: treeCount,
            visible: true,
            showCount: treeCount > 0,
            countTone: 'warning' as const,
            content:
              sessionTree === null ? (
                sessionTreeError !== null ? (
                  <div className="session-tree-feedback" role="alert">
                    Could not load the session tree: {sessionTreeError}
                  </div>
                ) : (
                  <div className="session-tree-feedback" role="status">
                    Loading session tree…
                  </div>
                )
              ) : (
                <>
                  {sessionTreeError === null ? null : (
                    <div className="session-tree-feedback" role="alert">
                      Could not refresh the session tree: {sessionTreeError}
                    </div>
                  )}
                  <SessionTreePanel
                    sessions={sessionTree}
                    selectedSessionId={session.id}
                    onSelectSession={onSelectSession}
                    viewState={sessionTreeViewState}
                    {...(onSessionTreeViewStateChange === undefined
                      ? {}
                      : { onViewStateChange: onSessionTreeViewStateChange })}
                  />
                </>
              ),
          },
        ]
      : []),
    ...(availableTabIds.includes('pull-requests')
      ? [
          {
            id: 'pull-requests',
            label: 'PRs',
            count: pullRequestList.pullRequests.length,
            visible: true,
            showCount: pullRequestList.pullRequests.length > 0,
            content: (
              <PullRequestListPanel
                key={`${session.id}:${session.task?.id ?? ''}`}
                sessionId={session.id}
                pullRequests={pullRequestList.pullRequests}
                loading={pullRequestList.loading}
                error={pullRequestList.error}
                onRefresh={pullRequestList.refresh}
              />
            ),
          },
        ]
      : []),
    ...(availableTabIds.includes('decisions')
      ? [
          {
            id: 'decisions',
            label: 'Decisions',
            count: unreviewedDecisionCount,
            visible: true,
            showCount: true,
            countTone: 'warning' as const,
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
    ...(availableTabIds.includes('crit')
      ? [
          {
            id: 'crit',
            label: 'crit',
            count: 1,
            content:
              critUrl === null ? null : (
                <CritPane url={critUrl} onClose={onCloseStatusPanel} />
              ),
          },
        ]
      : []),
  ]
  useEffect(() => {
    if (activePanelTabId !== activeTabId) setActivePanelTabId(activeTabId)
  }, [activePanelTabId, activeTabId])
  return {
    activeTabId,
    onSelectDecision: setSelectedDecisionSeq,
    onTabChange: setActivePanelTabId,
    selectedDecisionSeq: selectedDecision?.seq ?? null,
    tabs,
  }
}
