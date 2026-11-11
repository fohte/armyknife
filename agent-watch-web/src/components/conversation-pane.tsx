import { err, ok } from 'neverthrow'
import { useCallback, useEffect, useRef, useState } from 'react'

import {
  reuseEqualValue,
  shareUnchangedMessages,
} from '#components/conversation-message-sharing'
import {
  isConversationResponse,
  isSessionActions,
  requestAction,
  requestJson,
} from '#components/conversation-pane-response'
import { ConversationPaneView } from '#components/conversation-pane-view'
import type {
  PendingPermissionAction,
  PermissionRequestIdentity,
} from '#components/permission-card'
import { ResizableColumns } from '#components/resizable-columns'
import { SessionStatusPanel } from '#components/session-status-panel'
import { useConversationStatusPanel } from '#components/use-conversation-status-panel'
import type { BackgroundTask, ChatItem, Session, SessionActions } from '#types'

const EMPTY_SESSIONS: readonly Session[] = []

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
  const [messages, setMessages] = useState<ChatItem[]>([])
  const [backgroundTasks, setBackgroundTasks] = useState<BackgroundTask[]>([])
  const [messagesLoadedAt, setMessagesLoadedAt] = useState<number | null>(null)
  const [actions, setActions] = useState<SessionActions | null>(null)
  const [actionsReady, setActionsReady] = useState(false)
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)
  const [pendingPermissionAction, setPendingPermissionAction] =
    useState<PendingPermissionAction>(null)
  const requestGeneration = useRef(0)

  const refresh = useCallback(() => {
    const generation = requestGeneration.current + 1
    requestGeneration.current = generation
    const sessionPath = `/api/sessions/${encodeURIComponent(session.id)}`
    const messagesRequest = requestJson(
      `${sessionPath}/messages`,
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
          setMessages((previous) =>
            shareUnchangedMessages(previous, messagesResult.value.messages),
          )
          setBackgroundTasks(messagesResult.value.backgroundTasks)
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
  }, [session.id])

  useEffect(() => {
    refresh()
    return () => {
      requestGeneration.current += 1
    }
  }, [refresh, refreshKey])

  useEffect(() => {
    setPendingPermissionAction(null)
  }, [session.id])

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
      backgroundTasks={displayBackgroundTasks}
      actions={actions}
      actionsReady={actionsReady}
      pendingPermissionAction={pendingPermissionAction}
      loading={loading}
      error={error}
      onClearError={() => {
        setError(null)
      }}
      onAction={onAction}
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
