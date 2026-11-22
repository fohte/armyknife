import {
  type ReactNode,
  useCallback,
  useEffect,
  useMemo,
  useOptimistic,
  useRef,
  useState,
} from 'react'

import { commandResultMatchesCommands } from '#components/command-result-message'
import { ConversationPaneView } from '#components/conversation-pane-view'
import { MarkdownSessionLinksProvider } from '#components/markdown-message'
import type { PendingPermissionActions } from '#components/permission-card'
import { ResizableColumns } from '#components/resizable-columns'
import { SessionStatusPanel } from '#components/session-status-panel'
import { useConversationPaneActions } from '#components/use-conversation-pane-actions'
import { useConversationPaneData } from '#components/use-conversation-pane-data'
import { useConversationStatusPanel } from '#components/use-conversation-status-panel'
import {
  type SessionTreeCallbacks,
  useSessionTree,
} from '#components/use-session-tree'
import { createQueuedPromptId } from '#queued-prompt-id'
import { useSessionLinkTargets } from '#session-link-targets'
import { useSessionOperations } from '#session-operation-context'
import {
  applyConversationOperation,
  createSessionOperation,
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
    run,
    refresh: () => {
      void refresh()
    },
    setError: setPaneError,
    onSelectSession,
  })
  const onAction = useCallback(
    (action: string, payload?: Record<string, unknown>) => {
      const messageCountBefore =
        action === 'messages'
          ? Math.max(
              conversationRef.current.total,
              nextMessageCountBefore.current ?? 0,
            )
          : undefined
      const operationResult = createSessionOperation(
        session.id,
        action,
        action === 'messages'
          ? {
              ...payload,
              _queuedPromptId: createQueuedPromptId(),
              _messageCountBefore: messageCountBefore,
            }
          : payload,
      )
      if (operationResult.isErr()) {
        setError(operationResult.error.message)
        return Promise.resolve(false)
      }
      const operation = operationResult.value
      if (operation.type === 'message' && messageCountBefore !== undefined) {
        nextMessageCountBefore.current = messageCountBefore + 1
      }
      const dispatchers = [dispatchConversationOperation]
      if (onOptimisticSessionOperation !== undefined) {
        dispatchers.push(onOptimisticSessionOperation)
      }
      return run(operation, {
        optimisticDispatchers: dispatchers,
        refresh: (_result, confirmedOperation) =>
          refreshForOperation(confirmedOperation),
      }).then((result) =>
        result.match(
          () => {
            clearErrors()
            return true
          },
          (requestError) => {
            setError(requestError.message)
            return false
          },
        ),
      )
    },
    [
      clearErrors,
      dispatchConversationOperation,
      onOptimisticSessionOperation,
      refreshForOperation,
      run,
      session.id,
    ],
  )

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
  const backgroundTaskElapsedOffset =
    messagesLoadedAt === null
      ? 0
      : Math.max(0, Math.floor((Date.now() - messagesLoadedAt) / 1_000))
  const displayBackgroundTasks = backgroundTasks.map((task) => ({
    ...task,
    elapsedSeconds: task.elapsedSeconds + backgroundTaskElapsedOffset,
  }))
  const {
    activeTabId,
    onOpenCritInFullscreen,
    onOpenWebUiInFullscreen,
    mobileWebUi,
    onDecisionJumpComplete,
    onDecisionJumpSucceeded,
    onSelectDecision,
    decisionJump,
    onTabChange,
    selectedDecisionSeq,
    tabs: panelTabs,
  } = useConversationStatusPanel({
    session,
    messages,
    allDecisions: displayedConversation.decisions,
    backgroundTasks: displayBackgroundTasks,
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
      vibeMode={conversation.vibeMode ?? session.vibeMode}
      sessions={sessions}
      sessionLinkTargets={sessionLinkTargets}
      messages={messages}
      start={conversation.start}
      epoch={conversation.epoch}
      olderCursor={conversation.olderCursor}
      decisions={displayedConversation.decisions}
      hasUnansweredProposal={displayedConversation.hasUnansweredProposal}
      loadingOlder={loadingOlder}
      onLoadOlder={loadOlder}
      onPruneOldMessages={pruneOldMessages}
      backgroundTasks={displayBackgroundTasks}
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
      error={error}
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
      onOpenTasks={() => {
        onTabChange('tasks')
        if (!statusPanelOpen) onToggleStatusPanel()
      }}
      isMobile={isMobile}
      statusPanelOpen={statusPanelOpen}
      selectedDecisionSeq={selectedDecisionSeq}
      decisionJump={decisionJump}
      onDecisionJumpComplete={onDecisionJumpComplete}
      onDecisionJumpSucceeded={onDecisionJumpSucceeded}
      onToggleStatusPanel={onToggleStatusPanel}
      onSelectDecision={onSelectDecision}
      mobileStatusPanel={statusPanel}
      mobileWebUi={mobileWebUi}
      onBack={onBack}
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
      {conversationLayout}
    </MarkdownSessionLinksProvider>
  )
}
