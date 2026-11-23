import {
  type Dispatch,
  type SetStateAction,
  useCallback,
  useEffect,
  useRef,
  useState,
} from 'react'

import { getPendingCritReviewUrl } from '#components/conversation-crit-url'
import {
  isReviewDocumentResponse,
  type ReviewDocumentResponse,
} from '#components/conversation-pane-response'
import { requestJson } from '#components/conversation-requests'
import type { Decision } from '#components/decision-card'
import { DecisionsPanel } from '#components/decisions-panel'
import { ReviewDocumentPanel } from '#components/review-document-panel'
import type { ReviewDocumentSummariesState } from '#components/review-document-summary'
import { RunningTasksPanel } from '#components/running-tasks-panel'
import type { SessionStatusPanelTab } from '#components/session-status-panel'
import { SessionTasksPanel } from '#components/session-tasks-panel'
import { SessionTreePanel } from '#components/session-tree-panel'
import { useWebUiFullscreen } from '#components/use-web-ui-fullscreen'
import { WebUiPane } from '#components/web-ui-pane'
import { webUiTabId } from '#components/web-ui-tab-id'
import { webUiTargetUrl } from '#components/web-ui-target-url'
import { useSessionOperations } from '#session-operation-context'
import { createSessionOperation } from '#session-operations'
import { getSessionTasks } from '#session-task-utils'
import {
  needsActionDescendantCount,
  type SessionTreeViewState,
} from '#session-tree-model'
import type { BackgroundTask, ChatItem, Session, SessionTreeNode } from '#types'

type UseConversationStatusPanelOptions = {
  session: Session
  reviewDocumentSummaries?: ReviewDocumentSummariesState | null
  onRetryReviewDocumentSummaries?: () => void
  messages: readonly ChatItem[]
  allDecisions?: readonly Decision[]
  backgroundTasks: readonly BackgroundTask[]
  sessionTree?: readonly SessionTreeNode[] | null
  sessionTreeError?: string | null
  sessionTreeViewState?: SessionTreeViewState | null
  onSessionTreeViewStateChange?: (state: SessionTreeViewState) => void
  onSelectSession?: (id: string) => void
  statusPanelOpen?: boolean
  isMobile?: boolean
  hostname?: string
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
  unsavedChanges: boolean
  view: 'edit' | 'preview'
}

type ReviewDocumentStates = Record<string, ReviewDocumentState>

function requestReviewDocumentAction(
  sessionId: string,
  document: ReviewDocumentResponse,
  action: 'save' | 'finish',
  setState: Dispatch<SetStateAction<ReviewDocumentStates>>,
  run: ReturnType<typeof useSessionOperations>['run'],
) {
  const savedRevision = (value: unknown) =>
    typeof value === 'object' &&
    value !== null &&
    'revision' in value &&
    typeof value.revision === 'string'
      ? value.revision
      : null
  setState((current) => {
    const state = current[document.id]
    if (state === undefined || state.sessionId !== sessionId) return current
    return {
      ...current,
      [document.id]: {
        ...state,
        saveStatus: action === 'save' ? 'saving' : 'finishing',
        saveError: null,
      },
    }
  })

  const onSuccess = (result: unknown) => {
    const revision = savedRevision(result)
    setState((current) => {
      const state = current[document.id]
      if (state === undefined || state.sessionId !== sessionId) return current
      return {
        ...current,
        [document.id]:
          action === 'save'
            ? {
                ...state,
                ...(state.document === null || revision === null
                  ? {}
                  : {
                      document: { ...state.document, revision },
                    }),
                saveStatus: 'saved',
                saveError: null,
                unsavedChanges: false,
              }
            : {
                ...state,
                saveStatus: 'saved',
                saveError: null,
                unsavedChanges: false,
              },
      }
    })
  }
  const onError = (error: Error) => {
    setState((current) => {
      const state = current[document.id]
      return state === undefined || state.sessionId !== sessionId
        ? current
        : {
            ...current,
            [document.id]: {
              ...state,
              saveStatus: 'error',
              saveError: error.message,
            },
          }
    })
  }
  const operation = createSessionOperation(
    sessionId,
    `review-document/${action}`,
    {
      id: document.id,
      revision: document.revision,
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
  isMobile = false,
  hostname,
  onAction,
  onCloseStatusPanel,
  reviewDocumentSummaries = null,
  onRetryReviewDocumentSummaries,
}: UseConversationStatusPanelOptions) {
  const { run } = useSessionOperations()
  const [activePanelTabId, setActivePanelTabId] = useState<string | null>(null)
  const [selectedDecisionSeq, setSelectedDecisionSeq] = useState<number | null>(
    null,
  )
  const [decisionJump, setDecisionJump] = useState<{ seq: number } | null>(null)
  const onDecisionJumpComplete = useCallback(() => {
    setDecisionJump(null)
  }, [])
  const onDecisionJumpSucceeded = useCallback(() => {
    onDecisionJumpComplete()
    if (isMobile) onCloseStatusPanel()
  }, [isMobile, onCloseStatusPanel, onDecisionJumpComplete])
  const [reviewDocumentStates, setReviewDocumentStates] =
    useState<ReviewDocumentStates>({})
  const [reviewDocumentRetryCounts, setReviewDocumentRetryCounts] = useState<
    Record<string, number>
  >({})
  const requestedReviewDocuments = useRef(new Set<string>())
  const [selectedReviewDocumentId, setSelectedReviewDocumentId] = useState<
    string | null
  >(null)
  const [isReviewExpanded, setReviewExpanded] = useState(false)
  const reviewDocumentId = session.reviewDocumentId
  const reviewDocuments =
    reviewDocumentSummaries?.status === 'ready'
      ? reviewDocumentSummaries.summaries
      : []
  const selectedSummary =
    reviewDocuments.find(
      (summary) => summary.id === selectedReviewDocumentId,
    ) ??
    reviewDocuments.find((summary) => summary.id === reviewDocumentId) ??
    reviewDocuments[0] ??
    null
  const selectedDocumentId = selectedSummary?.id ?? null
  const reviewTabCount = Math.max(
    session.reviewDocumentCount ?? Number(reviewDocumentId != null),
    reviewDocuments.length,
  )
  const reviewDocumentState =
    selectedDocumentId === null
      ? null
      : (reviewDocumentStates[selectedDocumentId] ?? null)
  const hasReviewDocumentPanel =
    selectedSummary !== null &&
    reviewDocumentState !== null &&
    reviewDocumentState.sessionId === session.id &&
    !reviewDocumentState.loading &&
    reviewDocumentState.error === null &&
    reviewDocumentState.document !== null
  const reviewDocumentRetryCount =
    selectedDocumentId === null
      ? 0
      : (reviewDocumentRetryCounts[selectedDocumentId] ?? 0)
  const decisions =
    allDecisions ??
    messages.filter(
      (message): message is Decision => message.type === 'decision',
    )
  const decisionCount = session.decisionCount ?? decisions.length
  const unreviewedDecisionCount =
    session.unreviewedDecisionCount ??
    decisions.filter((decision) => decision.review === null).length
  const selectedDecision =
    decisions.find((decision) => decision.seq === selectedDecisionSeq) ??
    decisions.find((decision) => decision.review === null) ??
    decisions[0]
  const pendingCritReviewUrl = getPendingCritReviewUrl(session)
  const registeredWebUis = (session.webUis ?? []).filter(
    (webUi) => webUi.name !== 'crit' || webUi.url.trim() !== '',
  )
  const webUis =
    pendingCritReviewUrl !== null &&
    !registeredWebUis.some((webUi) => webUi.name === 'crit')
      ? [...registeredWebUis, { name: 'crit', url: pendingCritReviewUrl }]
      : registeredWebUis
  const webUiTabIds = webUis.map(webUiTabId)
  const {
    request: fullscreenRequest,
    webUiRequest: fullscreenWebUi,
    mobileWebUi,
    openWebUi: onOpenWebUiInFullscreen,
    closeWebUi: closeFullscreenWebUi,
  } = useWebUiFullscreen({
    isMobile,
    statusPanelOpen,
    ...(hostname === undefined ? {} : { hostname }),
    onSelectTab: setActivePanelTabId,
    onClose: onCloseStatusPanel,
  })
  const treeCount =
    sessionTree === null
      ? 0
      : needsActionDescendantCount(sessionTree, session.id)
  const treeAvailable =
    session.hasSessionTree === true &&
    (sessionTreeError !== null ||
      sessionTree === null ||
      sessionTree.length > 1)
  const sessionTasks = getSessionTasks(session)
  const availableTabIds = [
    ...(backgroundTasks.length === 0 ? [] : ['running']),
    ...(sessionTasks.length === 0 ? [] : ['tasks']),
    ...(treeAvailable ? ['tree'] : []),
    ...(reviewTabCount === 0 ? [] : ['review-document']),
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
  useEffect(() => {
    if (!statusPanelOpen || isMobile) setReviewExpanded(false)
  }, [isMobile, statusPanelOpen])
  useEffect(() => {
    if (selectedDocumentId === null) {
      return
    }
    if (!statusPanelOpen || activeTabId !== 'review-document') {
      return
    }
    const requestKey = `${session.id}:${selectedDocumentId}:${String(reviewDocumentRetryCount)}`
    if (requestedReviewDocuments.current.has(requestKey)) {
      return
    }
    requestedReviewDocuments.current.add(requestKey)
    setReviewDocumentStates((states) => ({
      ...states,
      [selectedDocumentId]: {
        sessionId: session.id,
        reviewDocumentId: selectedDocumentId,
        document: null,
        error: null,
        loading: true,
        saveStatus: 'idle',
        saveError: null,
        unsavedChanges: false,
        view: 'preview',
      },
    }))
    void requestJson(
      `/api/sessions/${encodeURIComponent(session.id)}/review-documents/${encodeURIComponent(selectedDocumentId)}`,
      isReviewDocumentResponse,
    ).match(
      (document) => {
        const nextState: ReviewDocumentState = {
          sessionId: session.id,
          reviewDocumentId: document.id,
          document,
          error: null,
          loading: false,
          saveStatus: 'idle',
          saveError: null,
          unsavedChanges: false,
          view: 'preview',
        }
        setReviewDocumentStates((states) => ({
          ...states,
          [document.id]: nextState,
        }))
      },
      (error) => {
        setReviewDocumentStates((states) => ({
          ...states,
          [selectedDocumentId]: {
            sessionId: session.id,
            reviewDocumentId: selectedDocumentId,
            document: null,
            error: error.message,
            loading: false,
            saveStatus: 'idle',
            saveError: null,
            unsavedChanges: false,
            view: 'preview',
          },
        }))
      },
    )
  }, [
    activeTabId,
    session.id,
    selectedDocumentId,
    statusPanelOpen,
    reviewDocumentRetryCount,
  ])
  const retrySelectedReviewDocument = () => {
    if (selectedDocumentId === null) return
    setReviewDocumentStates((states) => {
      const state = states[selectedDocumentId]
      return state === undefined
        ? states
        : {
            ...states,
            [selectedDocumentId]: { ...state, error: null, loading: true },
          }
    })
    setReviewDocumentRetryCounts((counts) => ({
      ...counts,
      [selectedDocumentId]: (counts[selectedDocumentId] ?? 0) + 1,
    }))
  }
  const updateSelectedReviewDocument = (
    update: (state: ReviewDocumentState) => ReviewDocumentState,
  ) => {
    if (selectedDocumentId === null) return
    setReviewDocumentStates((states) => {
      const state = states[selectedDocumentId]
      return state === undefined
        ? states
        : { ...states, [selectedDocumentId]: update(state) }
    })
  }
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
    ...(availableTabIds.includes('tasks')
      ? [
          {
            id: 'tasks',
            label: 'Tasks',
            count: sessionTasks.length,
            showCount: true,
            content: (
              <SessionTasksPanel
                tasks={sessionTasks}
                repo={session.repo}
                primaryTaskId={session.task?.id ?? null}
                onMakePrimary={(taskId) => onAction('primary-task', { taskId })}
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
                    size={isMobile ? 'phone' : 'desktop'}
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
    ...(availableTabIds.includes('review-document')
      ? [
          {
            id: 'review-document',
            label: 'Review',
            count: reviewTabCount,
            visible: true,
            showCount: true,
            content:
              selectedSummary === null ? (
                reviewDocumentSummaries?.status === 'unavailable' ? (
                  <div className="session-tree-feedback" role="alert">
                    Could not load open drafts: {reviewDocumentSummaries.error}
                    {onRetryReviewDocumentSummaries === undefined ? null : (
                      <button
                        type="button"
                        onClick={onRetryReviewDocumentSummaries}
                      >
                        Retry drafts
                      </button>
                    )}
                  </div>
                ) : (
                  <div className="session-tree-feedback" role="status">
                    {reviewDocumentSummaries === null
                      ? 'Loading drafts…'
                      : 'No open drafts.'}
                  </div>
                )
              ) : reviewDocumentState === null ||
                reviewDocumentState.sessionId !== session.id ||
                reviewDocumentState.loading ? (
                <div className="session-tree-feedback" role="status">
                  Loading draft…
                </div>
              ) : reviewDocumentState.error !== null ? (
                <div className="session-tree-feedback" role="alert">
                  Could not load the draft: {reviewDocumentState.error}
                  <button type="button" onClick={retrySelectedReviewDocument}>
                    Retry draft
                  </button>
                </div>
              ) : reviewDocumentState.document === null ? null : (
                <ReviewDocumentPanel
                  summary={selectedSummary}
                  summaries={reviewDocuments}
                  isMobile={isMobile}
                  isExpanded={isReviewExpanded}
                  canSave={selectedSummary.canSave}
                  canFinish={selectedSummary.canFinish}
                  frontmatter={reviewDocumentState.document.frontmatter}
                  body={reviewDocumentState.document.body}
                  status={reviewDocumentState.saveStatus}
                  unsavedChanges={reviewDocumentState.unsavedChanges}
                  error={reviewDocumentState.saveError}
                  view={reviewDocumentState.view}
                  onSelectDocument={setSelectedReviewDocumentId}
                  onExpand={() => {
                    if (!isMobile) setReviewExpanded((expanded) => !expanded)
                  }}
                  onFrontmatterChange={(frontmatter) => {
                    updateSelectedReviewDocument((current) => ({
                      ...current,
                      document:
                        current.document === null
                          ? null
                          : { ...current.document, frontmatter },
                      saveStatus: 'idle',
                      saveError: null,
                      unsavedChanges: true,
                    }))
                  }}
                  onBodyChange={(body) => {
                    updateSelectedReviewDocument((current) => ({
                      ...current,
                      document:
                        current.document === null
                          ? null
                          : { ...current.document, body },
                      saveStatus: 'idle',
                      saveError: null,
                      unsavedChanges: true,
                    }))
                  }}
                  onViewChange={(view) => {
                    updateSelectedReviewDocument((current) => ({
                      ...current,
                      view,
                    }))
                  }}
                  onSave={() => {
                    const document = reviewDocumentState.document
                    if (document !== null)
                      requestReviewDocumentAction(
                        session.id,
                        document,
                        'save',
                        setReviewDocumentStates,
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
                        setReviewDocumentStates,
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
                showReviewed
                onSelect={(seq) => {
                  setSelectedDecisionSeq(seq)
                  setDecisionJump({ seq })
                }}
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
          url={
            fullscreenWebUi?.webUi.name === webUi.name
              ? fullscreenWebUi.url
              : webUi.url
          }
          {...(fullscreenWebUi?.webUi.name === webUi.name
            ? { targetUrlOverride: fullscreenWebUi.targetUrl }
            : {})}
          exposedUrl={webUi.exposedUrl ?? null}
          size={isMobile ? 'phone' : 'desktop'}
          fullscreenRequest={
            fullscreenWebUi?.webUi.name === webUi.name ? fullscreenRequest : 0
          }
          onClose={closeFullscreenWebUi}
        />
      ),
    })),
  ]
  useEffect(() => {
    if (activePanelTabId !== activeTabId) setActivePanelTabId(activeTabId)
  }, [activePanelTabId, activeTabId])
  return {
    activeTabId,
    onOpenCritInFullscreen: () => {
      const critWebUi = webUis.find((webUi) => webUi.name === 'crit')
      if (critWebUi !== undefined) {
        onOpenWebUiInFullscreen(
          critWebUi,
          critWebUi.url,
          webUiTargetUrl(critWebUi, hostname),
        )
      }
    },
    onOpenWebUiInFullscreen,
    fullscreenWebUi,
    mobileWebUi,
    isReviewExpanded,
    hasReviewDocumentPanel,
    onCollapseReview: () => {
      setReviewExpanded(false)
    },
    onDecisionJumpComplete,
    onDecisionJumpSucceeded,
    onSelectDecision: setSelectedDecisionSeq,
    decisionJump,
    onTabChange: setActivePanelTabId,
    selectedDecisionSeq: selectedDecision?.seq ?? null,
    tabs,
  }
}
