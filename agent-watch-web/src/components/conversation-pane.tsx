import {
  useCallback,
  useEffect,
  useMemo,
  useOptimistic,
  useRef,
  useState,
} from 'react'

import { ConversationPaneView } from '#components/conversation-pane-view'
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
}: ConversationPaneProps) {
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
  const onCommandExecuted = useCallback(() => {
    void refresh()
  }, [refresh])

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
    onSelectDecision,
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
    onAction,
    onCloseStatusPanel,
  })
  const statusPanel = statusPanelOpen ? (
    <SessionStatusPanel
      tabs={panelTabs}
      activeTabId={activeTabId}
      onTabChange={onTabChange}
      onClose={onCloseStatusPanel}
    />
  ) : null

  const conversationView = (
    <ConversationPaneView
      session={session}
      sessions={sessions}
      messages={messages}
      start={conversation.start}
      decisions={displayedConversation.decisions}
      hasUnansweredProposal={displayedConversation.hasUnansweredProposal}
      loadingOlder={loadingOlder}
      onLoadOlder={loadOlder}
      onPruneOldMessages={pruneOldMessages}
      backgroundTasks={displayBackgroundTasks}
      skills={conversation.skills ?? []}
      actions={actions}
      actionsReady={actionsReady}
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
      onRewind={onRewind}
      onOpenInTerminal={onOpenInTerminal}
      onSelectSession={onSelectSession}
      isMobile={isMobile}
      statusPanelOpen={statusPanelOpen}
      selectedDecisionSeq={selectedDecisionSeq}
      onToggleStatusPanel={onToggleStatusPanel}
      onSelectDecision={onSelectDecision}
      mobileStatusPanel={statusPanel}
      onBack={onBack}
      onLater={onLater}
      {...(onClose === undefined ? {} : { onClose })}
      isToday={isToday}
      onToggleToday={onToggleToday}
    />
  )

  if (isMobile) return conversationView

  return (
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
}
