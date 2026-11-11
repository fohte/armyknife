import { err, ok } from 'neverthrow'
import { useCallback, useEffect, useRef, useState } from 'react'

import {
  reuseEqualValue,
  shareUnchangedMessages,
} from '#components/conversation-message-sharing'
import {
  isConversationResponse,
  isSessionActions,
} from '#components/conversation-pane-response'
import { ConversationPaneView } from '#components/conversation-pane-view'
import { requestAction, requestJson } from '#components/conversation-requests'
import type {
  PendingPermissionAction,
  PermissionRequestIdentity,
} from '#components/permission-card'
import { ResizableColumns } from '#components/resizable-columns'
import { SessionStatusPanel } from '#components/session-status-panel'
import { useConversationStatusPanel } from '#components/use-conversation-status-panel'
import type {
  ConversationResponse,
  OpenInTerminalStatus,
  Session,
  SessionActions,
} from '#types'

const EMPTY_SESSIONS: readonly Session[] = []
const EMPTY_CONVERSATION: ConversationResponse = {
  messages: [],
  backgroundTasks: [],
  start: 0,
  total: 0,
  decisions: [],
  hasUnansweredProposal: false,
  activeQuestionIndex: null,
}
const INITIAL_MESSAGE_LIMIT = 100

type ConversationPaneProps = {
  session: Session
  sessions?: readonly Session[]
  refreshKey: number
  onSelectSession: (id: string) => void
  isMobile?: boolean
  statusPanelOpen?: boolean
  onToggleStatusPanel?: () => void
  onCloseStatusPanel?: () => void
  onBack?: () => void
  onLater?: () => void
  isToday?: boolean
  onToggleToday?: (isToday: boolean) => void
}

export function ConversationPane({
  session,
  sessions = EMPTY_SESSIONS,
  refreshKey,
  onSelectSession,
  isMobile = false,
  statusPanelOpen = false,
  onToggleStatusPanel = () => undefined,
  onCloseStatusPanel = () => undefined,
  onBack = () => undefined,
  onLater = () => undefined,
  isToday = false,
  onToggleToday = () => undefined,
}: ConversationPaneProps) {
  const [conversation, setConversation] =
    useState<ConversationResponse>(EMPTY_CONVERSATION)
  const conversationRef = useRef(conversation)
  const conversationSessionId = useRef<string | null>(null)
  const [loadingOlder, setLoadingOlder] = useState(false)
  const [messagesLoadedAt, setMessagesLoadedAt] = useState<number | null>(null)
  const [actions, setActions] = useState<SessionActions | null>(null)
  const [actionsReady, setActionsReady] = useState(false)
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)
  const [pendingPermissionAction, setPendingPermissionAction] =
    useState<PendingPermissionAction>(null)
  const [openInTerminalStatus, setOpenInTerminalStatus] =
    useState<OpenInTerminalStatus>(null)
  const currentSessionId = useRef(session.id)
  currentSessionId.current = session.id
  const openInTerminalTimer = useRef<number | null>(null)
  const requestGeneration = useRef(0)
  const updateConversation = useCallback((next: ConversationResponse) => {
    conversationRef.current = next
    setConversation(next)
  }, [])
  const clearOpenInTerminalTimer = useCallback(() => {
    if (openInTerminalTimer.current !== null) {
      window.clearTimeout(openInTerminalTimer.current)
      openInTerminalTimer.current = null
    }
  }, [])

  const refresh = useCallback(() => {
    const generation = requestGeneration.current + 1
    requestGeneration.current = generation
    setLoadingOlder(false)
    const sessionPath = `/api/sessions/${encodeURIComponent(session.id)}`
    const loadedConversation =
      conversationSessionId.current === session.id
        ? conversationRef.current
        : null
    const messagesRequest = requestJson(
      loadedConversation === null
        ? `${sessionPath}/messages?limit=${String(INITIAL_MESSAGE_LIMIT)}`
        : `${sessionPath}/messages?from=${String(loadedConversation.start)}`,
      isConversationResponse,
    ).match(
      (value) => ok(value),
      (requestError) => err(requestError),
    )
    const actionsRequest = requestJson(
      `${sessionPath}/actions`,
      isSessionActions,
    ).match(
      (value) => ok(value),
      (requestError) => err(requestError),
    )
    void Promise.all([messagesRequest, actionsRequest]).then(
      ([messagesResult, actionsResult]) => {
        if (requestGeneration.current !== generation) return
        if (messagesResult.isOk()) {
          const previous =
            conversationSessionId.current === session.id
              ? conversationRef.current
              : EMPTY_CONVERSATION
          updateConversation({
            ...messagesResult.value,
            messages: shareUnchangedMessages(
              previous.messages,
              messagesResult.value.messages,
              previous.start,
              messagesResult.value.start,
            ),
          })
          conversationSessionId.current = session.id
          setMessagesLoadedAt(Date.now())
        }
        if (actionsResult.isOk()) {
          setActions((previous) =>
            reuseEqualValue(previous, actionsResult.value),
          )
        }
        setPendingPermissionAction((pending) => {
          if (pending === null) return null
          const permissionDisappeared =
            messagesResult.isOk() &&
            !messagesResult.value.messages.some(
              (message) =>
                message.type === 'permission' &&
                message.tool === pending.tool &&
                message.description === pending.description,
            )
          const permissionUnavailable =
            actionsResult.isOk() &&
            (actionsResult.value.permission == null ||
              (!actionsResult.value.permission.approve &&
                !actionsResult.value.permission.reject))
          return permissionDisappeared || permissionUnavailable ? null : pending
        })
        const firstError = messagesResult.isErr()
          ? messagesResult.error
          : actionsResult.isErr()
            ? actionsResult.error
            : null
        if (messagesResult.isOk() && actionsResult.isOk()) {
          setActionsReady(true)
        }
        setError(firstError?.message ?? null)
        setLoading(false)
      },
    )
  }, [session.id, updateConversation])

  useEffect(() => {
    refresh()
    return () => {
      requestGeneration.current += 1
    }
  }, [refresh, refreshKey])

  useEffect(() => {
    setPendingPermissionAction(null)
    setOpenInTerminalStatus(null)
    if (conversationSessionId.current !== session.id) {
      updateConversation(EMPTY_CONVERSATION)
      setMessagesLoadedAt(null)
      setLoadingOlder(false)
    }
    clearOpenInTerminalTimer()
    return clearOpenInTerminalTimer
  }, [clearOpenInTerminalTimer, session.id, updateConversation])

  const loadOlder = useCallback(() => {
    const current = conversationRef.current
    if (
      conversationSessionId.current !== session.id ||
      current.start <= 0 ||
      loadingOlder
    ) {
      return
    }
    const generation = requestGeneration.current + 1
    requestGeneration.current = generation
    setLoadingOlder(true)
    void requestJson(
      `/api/sessions/${encodeURIComponent(session.id)}/messages?before=${String(current.start)}&limit=${String(INITIAL_MESSAGE_LIMIT)}`,
      isConversationResponse,
    ).match(
      (response) => {
        if (requestGeneration.current !== generation) return
        if (conversationSessionId.current !== session.id) return
        const latest = conversationRef.current
        const messages = [...response.messages, ...latest.messages]
        updateConversation({
          ...response,
          messages: shareUnchangedMessages(
            latest.messages,
            messages,
            latest.start,
            response.start,
          ),
        })
        setMessagesLoadedAt(Date.now())
        setError(null)
        setLoadingOlder(false)
      },
      (requestError) => {
        if (requestGeneration.current !== generation) return
        setError(requestError.message)
        setLoadingOlder(false)
      },
    )
  }, [loadingOlder, session.id, updateConversation])

  const onOpenInTerminal = useCallback(() => {
    const requestedSessionId = session.id
    clearOpenInTerminalTimer()
    setOpenInTerminalStatus({ state: 'sending' })
    void requestAction(requestedSessionId, 'focus').match(
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
  }, [clearOpenInTerminalTimer, session.id])

  const onAction = useCallback(
    (
      action: string,
      payload?: Record<string, unknown>,
      permissionIdentity?: PermissionRequestIdentity,
    ) => {
      const permissionAction =
        action === 'permission/approve'
          ? 'approve'
          : action === 'permission/reject'
            ? 'reject'
            : null
      if (permissionAction !== null && permissionIdentity !== undefined) {
        setPendingPermissionAction({
          action: permissionAction,
          status: 'sending',
          ...permissionIdentity,
        })
      }
      return requestAction(session.id, action, payload).match(
        () => {
          if (permissionAction !== null && permissionIdentity !== undefined) {
            setPendingPermissionAction({
              action: permissionAction,
              status: 'sent',
              ...permissionIdentity,
            })
          }
          setError(null)
          refresh()
          return true
        },
        (requestError) => {
          if (permissionAction !== null && permissionIdentity !== undefined) {
            setPendingPermissionAction(null)
          }
          setError(requestError.message)
          return false
        },
      )
    },
    [refresh, session.id],
  )

  const { messages, backgroundTasks } = conversation
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
    decisionsPanelOpen,
    onSelectDecision,
    onTabChange,
    onToggleDecisionsPanel,
    selectedDecisionSeq,
    tabs: panelTabs,
  } = useConversationStatusPanel({
    session,
    messages,
    allDecisions: conversation.decisions,
    backgroundTasks: displayBackgroundTasks,
    statusPanelOpen,
    onAction,
    onCloseStatusPanel,
    onToggleStatusPanel,
  })

  const conversationView = (
    <ConversationPaneView
      session={session}
      sessions={sessions}
      messages={messages}
      start={conversation.start}
      decisions={conversation.decisions}
      hasUnansweredProposal={conversation.hasUnansweredProposal}
      activeQuestionIndex={conversation.activeQuestionIndex}
      loadingOlder={loadingOlder}
      onLoadOlder={loadOlder}
      backgroundTasks={displayBackgroundTasks}
      actions={actions}
      actionsReady={actionsReady}
      pendingPermissionAction={pendingPermissionAction}
      openInTerminalStatus={openInTerminalStatus}
      loading={loading}
      error={error}
      onClearError={() => {
        setError(null)
      }}
      onAction={onAction}
      onOpenInTerminal={onOpenInTerminal}
      onSelectSession={onSelectSession}
      isMobile={isMobile}
      statusPanelOpen={statusPanelOpen}
      decisionsPanelOpen={decisionsPanelOpen}
      selectedDecisionSeq={selectedDecisionSeq}
      onToggleStatusPanel={onToggleStatusPanel}
      onToggleDecisionsPanel={onToggleDecisionsPanel}
      onSelectDecision={onSelectDecision}
      onBack={onBack}
      onLater={onLater}
      isToday={isToday}
      onToggleToday={onToggleToday}
    />
  )

  if (isMobile) return conversationView

  return (
    <div className="conversation-status-panel-container">
      <ResizableColumns
        first={conversationView}
        second={
          statusPanelOpen ? (
            <SessionStatusPanel
              tabs={panelTabs}
              activeTabId={activeTabId}
              onTabChange={onTabChange}
              onClose={onCloseStatusPanel}
            />
          ) : null
        }
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
