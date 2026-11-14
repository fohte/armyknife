import { type Dispatch, type SetStateAction, useEffect, useState } from 'react'

import {
  isReviewDocumentResponse,
  type ReviewDocumentResponse,
} from '#components/conversation-pane-response'
import { requestJson } from '#components/conversation-requests'
import type { Decision } from '#components/decision-card'
import { DecisionsPanel } from '#components/decisions-panel'
import { PullRequestListPanel } from '#components/pull-request-list-panel'
import { ReviewDocumentPanel } from '#components/review-document-panel'
import { RunningTasksPanel } from '#components/running-tasks-panel'
import type { SessionStatusPanelTab } from '#components/session-status-panel'
import { SessionTreePanel } from '#components/session-tree-panel'
import { useSessionPullRequests } from '#components/use-session-pull-requests'
import { WebUiPane } from '#components/web-ui-pane'
import { useSessionOperations } from '#session-operation-context'
import { createSessionOperation } from '#session-operations'
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

type ReviewDocumentState = {
  sessionId: string
  reviewDocumentId: string
  document: ReviewDocumentResponse | null
  error: string | null
  loading: boolean
  saveStatus: 'idle' | 'saving' | 'saved' | 'error' | 'finishing'
  saveError: string | null
  mobileView: 'edit' | 'preview'
}

function requestReviewDocumentAction(
  sessionId: string,
  document: ReviewDocumentResponse,
  action: 'save' | 'finish',
  setState: Dispatch<SetStateAction<ReviewDocumentState | null>>,
  run: ReturnType<typeof useSessionOperations>['run'],
) {
  const matchesDocument = (
    current: ReviewDocumentState | null,
  ): current is ReviewDocumentState =>
    current !== null &&
    current.sessionId === sessionId &&
    current.reviewDocumentId === document.id
  setState((current) => {
    if (current === null || !matchesDocument(current)) return current
    return {
      ...current,
      saveStatus: action === 'save' ? 'saving' : 'finishing',
      saveError: null,
    }
  })

  const onSuccess = () => {
    setState((current) => {
      if (!matchesDocument(current)) return current
      return action === 'save'
        ? { ...current, saveStatus: 'saved', saveError: null }
        : null
    })
  }
  const onError = (error: Error) => {
    setState((current) =>
      matchesDocument(current)
        ? {
            ...current,
            saveStatus: 'error',
            saveError: error.message,
          }
        : current,
    )
  }
  const operation = createSessionOperation(
    sessionId,
    `review-document/${action}`,
    {
      id: document.id,
      frontmatter: document.frontmatter,
      body: document.body,
    },
  )
  operation.match((value) => {
    void run(value).then((result) => {
      result.match(onSuccess, onError)
    })
  }, onError)
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
  const { run } = useSessionOperations()
  const [activePanelTabId, setActivePanelTabId] = useState<string | null>(null)
  const [selectedDecisionSeq, setSelectedDecisionSeq] = useState<number | null>(
    null,
  )
  const [reviewDocumentState, setReviewDocumentState] =
    useState<ReviewDocumentState | null>(null)
  const reviewDocumentId = session.reviewDocumentId
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
  const legacyCritUrl =
    session.kind === 'Review' &&
    session.category === 'needs_action' &&
    session.critUrl != null &&
    session.critUrl !== ''
      ? session.critUrl
      : null
  const webUis =
    session.webUis ??
    (legacyCritUrl === null ? [] : [{ name: 'crit', url: legacyCritUrl }])
  const webUiTabIds = webUis.map((webUi) =>
    webUi.name === 'crit' ? 'crit' : `web-ui-${encodeURIComponent(webUi.url)}`,
  )
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
    ...(reviewDocumentId == null ? [] : ['review-document']),
    ...(decisionCount === 0 ? [] : ['decisions']),
    ...webUiTabIds,
  ]
  const defaultPanelTabId =
    [...[...webUiTabIds].reverse(), 'review-document', 'running'].find((id) =>
      availableTabIds.includes(id),
    ) ??
    availableTabIds[0] ??
    null
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
  useEffect(() => {
    if (reviewDocumentId == null) {
      setReviewDocumentState(null)
      return
    }
    if (!statusPanelOpen || activeTabId !== 'review-document') {
      return
    }
    let current = true
    setReviewDocumentState({
      sessionId: session.id,
      reviewDocumentId,
      document: null,
      error: null,
      loading: true,
      saveStatus: 'idle',
      saveError: null,
      mobileView: 'edit',
    })
    void requestJson(
      `/api/sessions/${encodeURIComponent(session.id)}/review-document`,
      isReviewDocumentResponse,
    ).match(
      (document) => {
        if (current) {
          setReviewDocumentState({
            sessionId: session.id,
            reviewDocumentId: document.id,
            document,
            error: null,
            loading: false,
            saveStatus: 'idle',
            saveError: null,
            mobileView: 'edit',
          })
        }
      },
      (error) => {
        if (current) {
          setReviewDocumentState({
            sessionId: session.id,
            reviewDocumentId,
            document: null,
            error: error.message,
            loading: false,
            saveStatus: 'idle',
            saveError: null,
            mobileView: 'edit',
          })
        }
      },
    )
    return () => {
      current = false
    }
  }, [activeTabId, session.id, reviewDocumentId, statusPanelOpen])
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
    ...(availableTabIds.includes('review-document')
      ? [
          {
            id: 'review-document',
            label: 'Review',
            count: 0,
            visible: true,
            showCount: false,
            content:
              reviewDocumentState === null ||
              reviewDocumentState.sessionId !== session.id ||
              reviewDocumentState.reviewDocumentId !==
                session.reviewDocumentId ||
              reviewDocumentState.loading ? (
                <div className="session-tree-feedback" role="status">
                  Loading review document…
                </div>
              ) : reviewDocumentState.error !== null ? (
                <div className="session-tree-feedback" role="alert">
                  Could not load the review document:{' '}
                  {reviewDocumentState.error}
                </div>
              ) : reviewDocumentState.document === null ? null : (
                <ReviewDocumentPanel
                  frontmatter={reviewDocumentState.document.frontmatter}
                  body={reviewDocumentState.document.body}
                  status={reviewDocumentState.saveStatus}
                  error={reviewDocumentState.saveError}
                  mobileView={reviewDocumentState.mobileView}
                  onFrontmatterChange={(frontmatter) => {
                    setReviewDocumentState((current) =>
                      current?.document === null || current === null
                        ? current
                        : {
                            ...current,
                            document: { ...current.document, frontmatter },
                            saveStatus: 'idle',
                            saveError: null,
                          },
                    )
                  }}
                  onBodyChange={(body) => {
                    setReviewDocumentState((current) =>
                      current?.document === null || current === null
                        ? current
                        : {
                            ...current,
                            document: { ...current.document, body },
                            saveStatus: 'idle',
                            saveError: null,
                          },
                    )
                  }}
                  onMobileViewChange={(mobileView) => {
                    setReviewDocumentState((current) =>
                      current === null ? current : { ...current, mobileView },
                    )
                  }}
                  onSave={() => {
                    const document = reviewDocumentState.document
                    if (document !== null)
                      requestReviewDocumentAction(
                        session.id,
                        document,
                        'save',
                        setReviewDocumentState,
                        run,
                      )
                  }}
                  onFinish={() => {
                    const document = reviewDocumentState.document
                    if (document !== null)
                      requestReviewDocumentAction(
                        session.id,
                        document,
                        'finish',
                        setReviewDocumentState,
                        run,
                      )
                  }}
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
    ...webUis.map((webUi, index) => ({
      id: webUiTabIds[index] ?? `web-ui-${String(index)}`,
      label: webUi.name,
      count: 1,
      content: (
        <WebUiPane
          name={webUi.name}
          url={webUi.url}
          exposedUrl={webUi.exposedUrl ?? null}
          onClose={onCloseStatusPanel}
        />
      ),
    })),
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
