import { useEffect, useState } from 'react'

import { isSessionPullRequestListResponse } from '#components/conversation-pane-response'
import { requestJson } from '#components/conversation-requests'
import { CritPane } from '#components/crit-pane'
import type { Decision } from '#components/decision-card'
import { DecisionsPanel } from '#components/decisions-panel'
import { PullRequestListPanel } from '#components/pull-request-list-panel'
import { RunningTasksPanel } from '#components/running-tasks-panel'
import type { SessionStatusPanelTab } from '#components/session-status-panel'
import { SessionTreePanel } from '#components/session-tree-panel'
import {
  needsActionDescendantCount,
  type SessionTreeViewState,
} from '#session-tree-model'
import type {
  BackgroundTask,
  ChatItem,
  Session,
  SessionPullRequest,
  SessionTreeNode,
} from '#types'

const PULL_REQUEST_REFRESH_INTERVAL = 30_000

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
  sessionTreeViewState = null,
  onSessionTreeViewStateChange,
  onSelectSession = () => undefined,
  statusPanelOpen,
  onAction,
  onCloseStatusPanel,
  onToggleStatusPanel,
}: UseConversationStatusPanelOptions) {
  const [activePanelTabId, setActivePanelTabId] = useState<string | null>(null)
  const [pullRequestListState, setPullRequestListState] = useState<{
    sessionId: string | null
    pullRequests: SessionPullRequest[]
    loading: boolean
    error: string | null
  }>({ sessionId: null, pullRequests: [], loading: false, error: null })
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
  const pullRequestList =
    pullRequestListState.sessionId === session.id
      ? pullRequestListState
      : { pullRequests: [], loading: pullRequestListAvailable, error: null }
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
            content: (
              <RunningTasksPanel
                sessionId={session.id}
                tasks={backgroundTasks}
              />
            ),
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
        ]),
    ...(!pullRequestListAvailable
      ? []
      : [
          {
            id: 'pull-requests',
            label: 'PRs',
            count: pullRequestList.pullRequests.length,
            visible: true,
            showCount: pullRequestList.pullRequests.length > 0,
            content: (
              <PullRequestListPanel
                pullRequests={pullRequestList.pullRequests}
                loading={pullRequestList.loading}
                error={pullRequestList.error}
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
    if (
      !statusPanelOpen ||
      !pullRequestListAvailable ||
      activeTabId !== 'pull-requests'
    ) {
      return
    }

    let current = true
    let inFlight = false
    const refresh = async () => {
      if (inFlight) return
      inFlight = true
      setPullRequestListState((state) => ({
        sessionId: session.id,
        pullRequests: state.sessionId === session.id ? state.pullRequests : [],
        loading: true,
        error: null,
      }))
      await requestJson(
        `/api/sessions/${encodeURIComponent(session.id)}/pull-requests`,
        isSessionPullRequestListResponse,
      ).match(
        (response) => {
          if (current) {
            setPullRequestListState({
              sessionId: session.id,
              pullRequests: response.pullRequests,
              loading: false,
              error: response.error,
            })
          }
          inFlight = false
        },
        (error) => {
          if (current) {
            setPullRequestListState((state) => ({
              sessionId: session.id,
              pullRequests:
                state.sessionId === session.id ? state.pullRequests : [],
              loading: false,
              error: error.message,
            }))
          }
          inFlight = false
        },
      )
    }
    void refresh()
    const interval = window.setInterval(
      () => void refresh(),
      PULL_REQUEST_REFRESH_INTERVAL,
    )
    return () => {
      current = false
      window.clearInterval(interval)
    }
  }, [
    activeTabId,
    pullRequestListAvailable,
    session.id,
    session.task?.id,
    statusPanelOpen,
  ])
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
