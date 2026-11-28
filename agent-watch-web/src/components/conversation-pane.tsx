import {
  type ReactNode,
  useCallback,
  useEffect,
  useMemo,
  useOptimistic,
  useRef,
  useState,
} from 'react'

import { BackgroundTaskElapsedStartedAtProvider } from '#components/background-task-elapsed-context'
import { commandResultMatchesCommands } from '#components/command-result-message'
import { getConversationTimelineEndIndex } from '#components/conversation-message-timeline'
import { ConversationPaneView } from '#components/conversation-pane-view'
import { MarkdownSessionLinksProvider } from '#components/markdown-message'
import type { PendingPermissionActions } from '#components/permission-card'
import { ResizableColumns } from '#components/resizable-columns'
import { ReviewDocumentFocus } from '#components/review-document-focus'
import { SessionStatusPanel } from '#components/session-status-panel'
import { useConversationPaneActions } from '#components/use-conversation-pane-actions'
import { useConversationPaneData } from '#components/use-conversation-pane-data'
import { useConversationPaneOperation } from '#components/use-conversation-pane-operation'
import { useConversationStatusPanel } from '#components/use-conversation-status-panel'
import { useEditorReviewDocument } from '#components/use-editor-review-document'
import {
  type SessionTreeCallbacks,
  useSessionTree,
} from '#components/use-session-tree'
import { useSessionLinkTargets } from '#session-link-targets'
import { useSessionOperations } from '#session-operation-context'
import {
  applyConversationOperation,
  type SessionOperation,
} from '#session-operations'
import type { SessionTreeViewState } from '#session-tree-model'
import type { OpenInTerminalStatus, Session, SessionTreeNode } from '#types'

const EMPTY_SESSIONS: readonly Session[] = []

type ConversationPaneProps = {
  session: Session
  sessions?: readonly Session[]
  initialSessionTree?: readonly SessionTreeNode[] | null
  initialSessionTreeError?: string | null
  initialSessionTreeViewState?: SessionTreeViewState | null
  shouldSkipSessionTreeFetch?: boolean
  sessionTreeRefreshKey: number
  onSessionTreeViewStateChange?: (
    selectedSessionId: string,
    state: SessionTreeViewState,
  ) => void
  refreshKey: number
  onSelectSession: (id: string) => void
  isMobile?: boolean
  statusPanelOpen?: boolean
  onToggleStatusPanel?: () => void
  onCloseStatusPanel?: () => void
  onBack?: () => void
  onOpenNavigation?: () => void
  mobileNavOpen?: boolean
  onLater?: () => void
  onClose?: () => void
  onOptimisticSessionOperation?: (operation: SessionOperation) => void
  isToday?: boolean
  onToggleToday?: (isToday: boolean) => void
  composerFocusRequestId?: number | null
  onComposerFocusRequestHandled?: (requestId: number) => void
  mobileUpdateNotice?: ReactNode
} & SessionTreeCallbacks

export function ConversationPane({
  session,
  sessions = EMPTY_SESSIONS,
  initialSessionTree = null,
  initialSessionTreeError = null,
  initialSessionTreeViewState = null,
  shouldSkipSessionTreeFetch = false,
  sessionTreeRefreshKey,
  onSessionTreeLoaded,
  onSessionTreeRequestStarted,
  onSessionTreeRequestFailed,
  onSessionTreeViewStateChange,
  refreshKey,
  onSelectSession,
  isMobile = false,
  statusPanelOpen = false,
  onToggleStatusPanel = () => undefined,
  onCloseStatusPanel = () => undefined,
  onBack = () => undefined,
  onOpenNavigation,
  mobileNavOpen = false,
  onLater = () => undefined,
  onClose,
  onOptimisticSessionOperation,
  isToday = false,
  onToggleToday = () => undefined,
  composerFocusRequestId = null,
  onComposerFocusRequestHandled = () => undefined,
  mobileUpdateNotice,
}: ConversationPaneProps) {
  const sessionLinkTargets = useSessionLinkTargets(sessions)
  const onSelectSessionRef = useRef(onSelectSession)
  onSelectSessionRef.current = onSelectSession
  const selectLinkedSession = useCallback((id: string) => {
    onSelectSessionRef.current(id)
  }, [])
  const { pendingOperations, run, confirm } = useSessionOperations()
  const { sessionTree, sessionTreeError } = useSessionTree({
    session,
    initialSessionTree,
    initialSessionTreeError,
    shouldSkipSessionTreeFetch,
    refreshKey: sessionTreeRefreshKey,
    onSessionTreeLoaded,
    onSessionTreeRequestStarted,
    onSessionTreeRequestFailed,
  })
  const {
    actions,
    actionsReady,
    actionsSettled,
    clearErrors,
    conversation,
    conversationRef,
    currentSessionId,
    error,
    loadOlder,
    loading,
    loadingOlder,
    messagesLoadedAt,
    pruneOldMessages,
    refresh,
    refreshForOperation,
    setError,
  } = useConversationPaneData({
    sessionId: session.id,
    refreshKey,
    pendingOperations,
    confirm,
  })
  const [optimisticConversation, dispatchConversationOperation] = useOptimistic(
    conversation,
    applyConversationOperation,
  )
  const pendingConversationOperations = useMemo(
    () =>
      pendingOperations.filter(
        ({ operation }) => operation.sessionId === session.id,
      ),
    [pendingOperations, session.id],
  )
  const nextMessageCountBefore = useRef<number | null>(null)
  useEffect(() => {
    nextMessageCountBefore.current = pendingConversationOperations.reduce(
      (nextCount, pending) =>
        pending.operation.type === 'message' &&
        pending.operation.messageCountBefore !== undefined
          ? Math.max(nextCount, pending.operation.messageCountBefore + 1)
          : nextCount,
      conversation.total,
    )
  }, [conversation.total, pendingConversationOperations])
  const displayedConversation = useMemo(
    () =>
      pendingConversationOperations.reduce(
        (current, { operation }) =>
          applyConversationOperation(current, operation),
        optimisticConversation,
      ),
    [optimisticConversation, pendingConversationOperations],
  )
  const [openInTerminalStatus, setOpenInTerminalStatus] =
    useState<OpenInTerminalStatus>(null)
  const openInTerminalTimer = useRef<number | null>(null)
  const clearOpenInTerminalTimer = useCallback(() => {
    if (openInTerminalTimer.current !== null) {
      window.clearTimeout(openInTerminalTimer.current)
      openInTerminalTimer.current = null
    }
  }, [])
  const setPaneError = useCallback(
    (nextError: string | null) => {
      if (nextError === null) {
        clearErrors()
      } else {
        setError(nextError)
      }
    },
    [clearErrors, setError],
  )
  const onCommandStarted = useCallback(
    () =>
      conversationRef.current.start + conversationRef.current.messages.length,
    [conversationRef],
  )
  const onCommandExecuted = useCallback(
    async (commands: readonly string[], startIndex: number | null) => {
      await refresh()
      const current = conversationRef.current
      const index = current.messages.findLastIndex(
        (message, localIndex) =>
          (startIndex === null || current.start + localIndex >= startIndex) &&
          commandResultMatchesCommands(message, commands),
      )
      return index < 0 ? null : current.start + index
    },
    [conversationRef, refresh],
  )

  useEffect(() => {
    setOpenInTerminalStatus(null)
    clearOpenInTerminalTimer()
    return () => {
      clearOpenInTerminalTimer()
    }
  }, [clearOpenInTerminalTimer, session.id])

  const onOpenInTerminal = useCallback(() => {
    const requestedSessionId = session.id
    clearOpenInTerminalTimer()
    void run({ type: 'focus', sessionId: requestedSessionId }).then(
      (result) => {
        result.match(
          () => {
            if (currentSessionId.current !== requestedSessionId) return
            setOpenInTerminalStatus({ state: 'success' })
            openInTerminalTimer.current = window.setTimeout(() => {
              if (currentSessionId.current === requestedSessionId) {
                setOpenInTerminalStatus(null)
              }
              openInTerminalTimer.current = null
            }, 3_000)
          },
          (requestError) => {
            if (currentSessionId.current !== requestedSessionId) return
            setOpenInTerminalStatus({
              state: 'error',
              message: requestError.message,
            })
          },
        )
      },
    )
  }, [clearOpenInTerminalTimer, run, session.id])

  const { onRewind } = useConversationPaneActions({
    sessionId: session.id,
    sessionTitle: session.title,
    run,
    refresh: () => {
      void refresh()
    },
    setError: setPaneError,
    onSelectSession,
  })
  const { error: operationError, onAction } = useConversationPaneOperation({
    sessionId: session.id,
    conversationRef,
    nextMessageCountBefore,
    dispatchConversationOperation,
    onOptimisticSessionOperation,
    refreshForOperation,
    clearErrors,
    run,
  })

  const pendingPermissionActions =
    pendingOperations.reduce<PendingPermissionActions>(
      (pending, pendingOperation) => {
        const { operation, status } = pendingOperation
        if (operation.sessionId !== session.id) return pending
        if (operation.type === 'permission') {
          return {
            ...pending,
            [operation.requestId]: {
              action: operation.action,
              status,
              requestId: operation.requestId,
            },
          }
        }
        if (operation.type === 'permission_retry') {
          const requestId = `permission-denied-${String(operation.seq)}`
          return {
            ...pending,
            [requestId]: { action: 'retry', status, requestId },
          }
        }
        if (operation.type === 'permission_dismiss') {
          const requestId = `permission-denied-${String(operation.seq)}`
          return {
            ...pending,
            [requestId]: { action: 'dismiss', status, requestId },
          }
        }
        return pending
      },
      {},
    )
  const displayOpenInTerminalStatus = pendingOperations.some(
    ({ operation }) =>
      operation.sessionId === session.id && operation.type === 'focus',
  )
    ? { state: 'sending' as const }
    : openInTerminalStatus
  const interruptPending = pendingOperations.some(
    ({ operation }) =>
      operation.sessionId === session.id && operation.type === 'interrupt',
  )
  const { messages, backgroundTasks } = displayedConversation
  const actionTimelineIndex =
    loading && messages.length === 0
      ? undefined
      : getConversationTimelineEndIndex(
          messages,
          displayedConversation.start,
          session.category,
          session.isCompacting === true,
        )
  const editorReviewDocument = useEditorReviewDocument(session)
  const {
    activeTabId,
    isReviewFocusOpen,
    reviewFocusBody,
    onCloseReviewFocus,
    onOpenCritInFullscreen,
    onOpenWebUiInFullscreen,
    mobileWebUi,
    onTabChange,
    tabs: panelTabs,
    pullRequestListTarget,
  } = useConversationStatusPanel({
    session,
    actionTimelineIndex,
    reviewDocumentSummaries: editorReviewDocument?.summaries ?? null,
    ...(editorReviewDocument?.retrySummaries === undefined
      ? {}
      : {
          onRetryReviewDocumentSummaries: editorReviewDocument.retrySummaries,
        }),
    backgroundTasks,
    sessionTree,
    sessionTreeError,
    sessionTreeViewState: initialSessionTreeViewState,
    onSessionTreeViewStateChange: (state) =>
      onSessionTreeViewStateChange?.(session.id, state),
    onSelectSession,
    statusPanelOpen,
    isMobile,
    onAction,
    onCloseStatusPanel,
  })
  const statusPanel = statusPanelOpen ? (
    <SessionStatusPanel
      tabs={panelTabs}
      activeTabId={activeTabId}
      onTabChange={onTabChange}
      onClose={onCloseStatusPanel}
      sessionTitle={session.title}
      size={isMobile ? 'phone' : 'desktop'}
    />
  ) : null

  const conversationView = (
    <ConversationPaneView
      session={session}
      actionTimelineIndex={actionTimelineIndex}
      vibeMode={conversation.vibeMode ?? session.vibeMode}
      sessions={sessions}
      sessionLinkTargets={sessionLinkTargets}
      messages={messages}
      {...(displayedConversation.messageImageSources === undefined
        ? {}
        : { messageImageSources: displayedConversation.messageImageSources })}
      start={displayedConversation.start}
      epoch={conversation.epoch}
      olderCursor={conversation.olderCursor}
      hasUnansweredProposal={displayedConversation.hasUnansweredProposal}
      loadingOlder={loadingOlder}
      onLoadOlder={loadOlder}
      onPruneOldMessages={pruneOldMessages}
      backgroundTasks={backgroundTasks}
      skills={conversation.skills ?? []}
      actions={actions}
      actionsReady={actionsReady}
      actionsSettled={actionsSettled}
      composerFocusRequestId={composerFocusRequestId}
      onComposerFocusRequestHandled={onComposerFocusRequestHandled}
      mobileUpdateNotice={isMobile ? mobileUpdateNotice : undefined}
      pendingPermissionActions={pendingPermissionActions}
      openInTerminalStatus={displayOpenInTerminalStatus}
      interruptPending={interruptPending}
      loading={loading}
      error={operationError ?? error}
      onClearError={() => {
        clearErrors()
      }}
      onAction={onAction}
      onCommandExecuted={onCommandExecuted}
      onCommandStarted={onCommandStarted}
      onRewind={onRewind}
      onOpenInTerminal={onOpenInTerminal}
      onSelectSession={selectLinkedSession}
      onReviewInCrit={() => {
        onOpenCritInFullscreen()
        if (!statusPanelOpen) onToggleStatusPanel()
      }}
      editorReviewDocument={editorReviewDocument?.state ?? null}
      onOpenDrafts={() => {
        onTabChange('review-document')
        if (!statusPanelOpen) onToggleStatusPanel()
      }}
      onOpenTasks={() => {
        onTabChange('tasks')
        if (!statusPanelOpen) onToggleStatusPanel()
      }}
      isMobile={isMobile}
      statusPanelOpen={statusPanelOpen}
      onToggleStatusPanel={onToggleStatusPanel}
      mobileStatusPanel={statusPanel}
      pullRequestListTarget={pullRequestListTarget}
      mobileNavOpen={mobileNavOpen}
      mobileWebUi={mobileWebUi}
      onBack={onBack}
      {...(onOpenNavigation === undefined ? {} : { onOpenNavigation })}
      onLater={onLater}
      {...(onClose === undefined ? {} : { onClose })}
      isToday={isToday}
      onToggleToday={onToggleToday}
    />
  )

  const conversationLayout = isMobile ? (
    conversationView
  ) : (
    <div className="conversation-status-panel-container">
      <ResizableColumns
        first={conversationView}
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
    <MarkdownSessionLinksProvider
      sessions={sessionLinkTargets}
      onSelectSession={selectLinkedSession}
      webUis={session.webUis ?? []}
      onOpenWebUi={(webUi, url) => {
        onOpenWebUiInFullscreen(webUi, url)
        if (!isMobile && !statusPanelOpen) onToggleStatusPanel()
      }}
    >
      <BackgroundTaskElapsedStartedAtProvider value={messagesLoadedAt}>
        {isReviewFocusOpen && reviewFocusBody !== null ? (
          <ReviewDocumentFocus
            sessionTitle={session.title}
            body={reviewFocusBody}
            onBack={onCloseReviewFocus}
          />
        ) : (
          conversationLayout
        )}
      </BackgroundTaskElapsedStartedAtProvider>
    </MarkdownSessionLinksProvider>
  )
}
