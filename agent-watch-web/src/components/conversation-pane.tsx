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
import type { PendingPermissionActions } from '#components/permission-card'
import { ResizableColumns } from '#components/resizable-columns'
import { useSessionConversationCache } from '#components/session-conversation-cache'
import { SessionStatusPanel } from '#components/session-status-panel'
import { useConversationPaneActions } from '#components/use-conversation-pane-actions'
import { useConversationStatusPanel } from '#components/use-conversation-status-panel'
import {
  type SessionTreeCallbacks,
  useSessionTree,
} from '#components/use-session-tree'
import type { SessionTreeViewState } from '#session-tree-model'
import type {
  ConversationResponse,
  OpenInTerminalStatus,
  Session,
  SessionActions,
  SessionTreeNode,
} from '#types'

const EMPTY_SESSIONS: readonly Session[] = []
const EMPTY_CONVERSATION: ConversationResponse = {
  messages: [],
  backgroundTasks: [],
  skills: [],
  start: 0,
  total: 0,
  decisions: [],
  hasUnansweredProposal: false,
  activeQuestionIndex: null,
}
const INITIAL_MESSAGE_LIMIT = 100
const MAX_LOADED_MESSAGES = 300
const RETAINED_MESSAGE_COUNT = 200

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
  isToday = false,
  onToggleToday = () => undefined,
}: ConversationPaneProps) {
  const sessionConversationCache = useSessionConversationCache()
  const setCachedConversation = sessionConversationCache?.setCachedConversation
  const [initialCachedConversation] = useState(() =>
    sessionConversationCache?.getCachedConversation(session.id),
  )
  const [conversation, setConversation] = useState<ConversationResponse>(
    initialCachedConversation?.conversation ?? EMPTY_CONVERSATION,
  )
  const conversationRef = useRef(conversation)
  const conversationSessionId = useRef<string | null>(
    initialCachedConversation === undefined ? null : session.id,
  )
  const [loadingOlder, setLoadingOlder] = useState(false)
  const loadingOlderRef = useRef(false)
  const [messagesLoadedAt, setMessagesLoadedAt] = useState<number | null>(
    initialCachedConversation?.messagesLoadedAt ?? null,
  )
  const messagesLoadedAtRef = useRef(messagesLoadedAt)
  const [actions, setActions] = useState<SessionActions | null>(null)
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
  const [actionsReady, setActionsReady] = useState(false)
  const [loading, setLoading] = useState(
    initialCachedConversation === undefined,
  )
  const [error, setError] = useState<string | null>(null)
  const [pendingPermissionActions, setPendingPermissionActions] =
    useState<PendingPermissionActions>({})
  const [openInTerminalStatus, setOpenInTerminalStatus] =
    useState<OpenInTerminalStatus>(null)
  const currentSessionId = useRef(session.id)
  currentSessionId.current = session.id
  const openInTerminalTimer = useRef<number | null>(null)
  const requestGeneration = useRef(0)
  const olderRequestGeneration = useRef(0)
  const updateConversation = useCallback(
    (next: ConversationResponse) => {
      conversationRef.current = next
      setConversation(next)
      if (
        setCachedConversation &&
        conversationSessionId.current === session.id
      ) {
        const loadedAt = messagesLoadedAtRef.current
        if (loadedAt !== null) {
          setCachedConversation(session.id, {
            conversation: next,
            messagesLoadedAt: loadedAt,
          })
        }
      }
    },
    [session.id, setCachedConversation],
  )
  const markMessagesLoaded = useCallback(() => {
    const loadedAt = Date.now()
    messagesLoadedAtRef.current = loadedAt
    setMessagesLoadedAt(loadedAt)
  }, [])
  const pruneOldMessages = useCallback(() => {
    const current = conversationRef.current
    if (current.messages.length <= MAX_LOADED_MESSAGES) return
    const dropCount = current.messages.length - RETAINED_MESSAGE_COUNT
    updateConversation({
      ...current,
      start: current.start + dropCount,
      stableStart: undefined,
      stableEnd: undefined,
      stableDigest: undefined,
      messages: current.messages.slice(dropCount),
    })
  }, [updateConversation])
  const clearOpenInTerminalTimer = useCallback(() => {
    if (openInTerminalTimer.current !== null) {
      window.clearTimeout(openInTerminalTimer.current)
      openInTerminalTimer.current = null
    }
  }, [])

  const refresh = useCallback(() => {
    const generation = requestGeneration.current + 1
    requestGeneration.current = generation
    const sessionPath = `/api/sessions/${encodeURIComponent(session.id)}`
    const loadedConversation =
      conversationSessionId.current === session.id
        ? conversationRef.current
        : null
    const stableDigestQuery =
      loadedConversation !== null &&
      loadedConversation.stableStart === loadedConversation.start &&
      loadedConversation.stableEnd !== undefined &&
      loadedConversation.stableDigest !== undefined
        ? `&stableStart=${String(loadedConversation.stableStart)}&stableEnd=${String(loadedConversation.stableEnd)}&stableDigest=${loadedConversation.stableDigest}`
        : ''
    const messagesRequest = requestJson(
      loadedConversation === null
        ? `${sessionPath}/messages?limit=${String(INITIAL_MESSAGE_LIMIT)}`
        : `${sessionPath}/messages?from=${String(loadedConversation.start)}${stableDigestQuery}`,
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
        let refreshedConversation: ConversationResponse | null = null
        if (messagesResult.isOk()) {
          const alreadyLoaded = conversationSessionId.current === session.id
          const previous = alreadyLoaded
            ? conversationRef.current
            : EMPTY_CONVERSATION
          const preserveLoadedPrefix =
            alreadyLoaded && previous.start < messagesResult.value.start
          const start = preserveLoadedPrefix
            ? previous.start
            : messagesResult.value.start
          const messages = preserveLoadedPrefix
            ? [
                ...previous.messages.slice(
                  0,
                  messagesResult.value.start - previous.start,
                ),
                ...messagesResult.value.messages,
              ]
            : messagesResult.value.messages
          refreshedConversation = {
            ...messagesResult.value,
            start,
            messages: shareUnchangedMessages(
              previous.messages,
              messages,
              previous.start,
              start,
            ),
          }
          conversationSessionId.current = session.id
          markMessagesLoaded()
          updateConversation(refreshedConversation)
        }
        if (actionsResult.isOk()) {
          setActions((previous) =>
            reuseEqualValue(previous, actionsResult.value),
          )
        }
        const permissionUnavailable =
          actionsResult.isOk() &&
          (actionsResult.value.permission == null ||
            (!actionsResult.value.permission.approve &&
              !actionsResult.value.permission.reject))
        setPendingPermissionActions((pending) => {
          if (permissionUnavailable) return {}
          if (refreshedConversation === null) return pending
          const activeRequestIds = new Set(
            refreshedConversation.messages.flatMap((message) =>
              message.type === 'permission' && message.requestId !== undefined
                ? [message.requestId]
                : [],
            ),
          )
          return Object.fromEntries(
            Object.entries(pending).filter(([requestId]) =>
              activeRequestIds.has(requestId),
            ),
          )
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
  }, [markMessagesLoaded, session.id, updateConversation])

  useEffect(() => {
    refresh()
    return () => {
      requestGeneration.current += 1
    }
  }, [refresh, refreshKey])

  useEffect(() => {
    setPendingPermissionActions({})
    setOpenInTerminalStatus(null)
    if (conversationSessionId.current !== session.id) {
      updateConversation(EMPTY_CONVERSATION)
      messagesLoadedAtRef.current = null
      setMessagesLoadedAt(null)
      setLoadingOlder(false)
      loadingOlderRef.current = false
    }
    clearOpenInTerminalTimer()
    return () => {
      olderRequestGeneration.current += 1
      clearOpenInTerminalTimer()
    }
  }, [clearOpenInTerminalTimer, session.id, updateConversation])

  const loadOlder = useCallback(() => {
    const current = conversationRef.current
    if (
      conversationSessionId.current !== session.id ||
      current.start <= 0 ||
      loadingOlderRef.current
    ) {
      return
    }
    loadingOlderRef.current = true
    const generation = olderRequestGeneration.current + 1
    olderRequestGeneration.current = generation
    setLoadingOlder(true)
    void requestJson(
      `/api/sessions/${encodeURIComponent(session.id)}/messages?before=${String(current.start)}&limit=${String(INITIAL_MESSAGE_LIMIT)}`,
      isConversationResponse,
    ).match(
      (response) => {
        if (olderRequestGeneration.current !== generation) return
        if (conversationSessionId.current !== session.id) return
        const latest = conversationRef.current
        const messages = [...response.messages, ...latest.messages]
        markMessagesLoaded()
        updateConversation({
          ...response,
          stableStart: undefined,
          stableEnd: undefined,
          stableDigest: undefined,
          messages: shareUnchangedMessages(
            latest.messages,
            messages,
            latest.start,
            response.start,
          ),
        })
        setError(null)
        loadingOlderRef.current = false
        setLoadingOlder(false)
      },
      (requestError) => {
        if (olderRequestGeneration.current !== generation) return
        setError(requestError.message)
        loadingOlderRef.current = false
        setLoadingOlder(false)
      },
    )
  }, [loadingOlder, markMessagesLoaded, session.id, updateConversation])

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

  const { onAction, onRewind } = useConversationPaneActions({
    sessionId: session.id,
    refresh,
    setError,
    setPendingPermissionActions,
    onSelectSession,
  })

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
    onSelectDecision,
    onTabChange,
    selectedDecisionSeq,
    tabs: panelTabs,
  } = useConversationStatusPanel({
    session,
    messages,
    allDecisions: conversation.decisions,
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
      decisions={conversation.decisions}
      hasUnansweredProposal={conversation.hasUnansweredProposal}
      loadingOlder={loadingOlder}
      onLoadOlder={loadOlder}
      onPruneOldMessages={pruneOldMessages}
      backgroundTasks={displayBackgroundTasks}
      skills={conversation.skills ?? []}
      actions={actions}
      actionsReady={actionsReady}
      pendingPermissionActions={pendingPermissionActions}
      openInTerminalStatus={openInTerminalStatus}
      loading={loading}
      error={error}
      onClearError={() => {
        setError(null)
      }}
      onAction={onAction}
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
