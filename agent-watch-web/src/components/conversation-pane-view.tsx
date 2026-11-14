import '#components/conversation-pane.css'
import '#components/conversation-pane-view.css'

import { Button } from '@fohte/ui/button'
import {
  type KeyboardEvent as ReactKeyboardEvent,
  type ReactNode,
  useCallback,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from 'react'

import { ConversationPaneHeader } from '#components/conversation-pane-header'
import { ConversationSelectionReply } from '#components/conversation-selection-reply'
import { ConversationWorkingStatus } from '#components/conversation-working-status'
import { MessageComposer } from '#components/message-composer'
import type {
  PendingPermissionActions,
  PermissionRequestIdentity,
} from '#components/permission-card'
import { SessionLinkElapsedSecondsContext } from '#components/session-link-elapsed-seconds'
import { useComposerDraft } from '#components/use-composer-draft'
import { useComposerMessage } from '#components/use-composer-message'
import { useConversationFileDrop } from '#components/use-conversation-file-drop'
import { useConversationMessageElements } from '#components/use-conversation-message-elements'
import { useConversationScroll } from '#components/use-conversation-scroll'
import { useConversationSelectionReply } from '#components/use-conversation-selection-reply'
import { useOwnMessageKeyboardNavigation } from '#components/use-own-message-keyboard-navigation'
import { useSessionLinkTargets } from '#session-link-targets'
import type {
  BackgroundTask,
  ChatItem,
  OpenInTerminalStatus,
  RewindMode,
  Session,
  SessionActions,
  SkillSuggestion,
} from '#types'

type ConversationPaneViewProps = {
  session: Session
  sessions?: readonly Session[]
  messages: readonly ChatItem[]
  start?: number
  decisions?: readonly Extract<ChatItem, { type: 'decision' }>[]
  hasUnansweredProposal?: boolean
  loadingOlder?: boolean
  onLoadOlder?: () => void
  onPruneOldMessages?: () => void
  backgroundTasks?: readonly BackgroundTask[]
  skills?: readonly SkillSuggestion[]
  actions: SessionActions | null
  actionsReady?: boolean
  pendingPermissionActions?: PendingPermissionActions | undefined
  openInTerminalStatus?: OpenInTerminalStatus
  loading: boolean
  sending?: boolean
  interruptPending?: boolean
  error: string | null
  onClearError?: () => void
  onAction: (
    action: string,
    payload?: Record<string, unknown>,
    permissionIdentity?: PermissionRequestIdentity,
  ) => Promise<boolean>
  onCommandExecuted?: () => void
  onRewind?: (
    messageId: string,
    mode: RewindMode,
    draft: string,
  ) => Promise<boolean>
  onOpenInTerminal?: () => void
  onSelectSession: (id: string) => void
  isMobile?: boolean
  statusPanelOpen?: boolean
  mobileStatusPanel?: ReactNode
  selectedDecisionSeq?: number | null
  onToggleStatusPanel?: () => void
  onSelectDecision?: (seq: number) => void
  fileDropActive?: boolean
  onBack?: () => void
  onLater?: () => void
  onClose?: () => void
  isToday?: boolean
  onToggleToday?: (isToday: boolean) => void
}

export function ConversationPaneView({
  session,
  sessions = [],
  messages,
  start = 0,
  decisions,
  hasUnansweredProposal: responseHasUnansweredProposal,
  loadingOlder = false,
  onLoadOlder = () => undefined,
  onPruneOldMessages = () => undefined,
  backgroundTasks = [],
  skills = [],
  actions,
  actionsReady = true,
  pendingPermissionActions = {},
  openInTerminalStatus = null,
  loading,
  sending = false,
  interruptPending = false,
  error,
  onClearError = () => undefined,
  onAction,
  onCommandExecuted,
  onRewind,
  onOpenInTerminal,
  onSelectSession,
  isMobile = false,
  statusPanelOpen = false,
  mobileStatusPanel = null,
  selectedDecisionSeq = null,
  onToggleStatusPanel = () => undefined,
  onSelectDecision = () => undefined,
  fileDropActive = false,
  onBack,
  onLater,
  onClose,
  isToday = false,
  onToggleToday,
}: ConversationPaneViewProps) {
  const interruptInFlight = useRef(false)
  const pendingActionsRef = useRef<HTMLElement | null>(null)
  const [isInterrupting, setIsInterrupting] = useState(false)
  const currentDate = new Date()
  const currentYear = currentDate.getFullYear()
  const currentMonth = currentDate.getMonth()
  const currentDay = currentDate.getDate()
  const now = useMemo(
    () => new Date(currentYear, currentMonth, currentDay),
    [currentYear, currentMonth, currentDay],
  )
  const { draft, updateDraft, clearDraftIfUnchanged, storageAvailable } =
    useComposerDraft(session.id)
  const {
    focusDraft,
    position: selectionReplyPosition,
    dismiss: dismissSelectionReply,
    reply: replyToSelection,
  } = useConversationSelectionReply({ draft, isMobile, updateDraft })
  const {
    attachments,
    error: attachmentError,
    isSending,
    isProcessingAttachments,
    onPasteFiles,
    removeAttachment,
    sendMessage,
  } = useComposerMessage({
    sessionId: session.id,
    draft,
    messageEnabled: actions?.message === true,
    externalSending: sending,
    clearDraftIfUnchanged,
    onAction,
  })
  const {
    loadOlder,
    onScroll,
    scrollRef: conversationScrollRef,
  } = useConversationScroll({
    messages,
    loadingOlder,
    onLoadOlder,
    onPruneOldMessages,
    sessionId: session.id,
    start,
  })
  useOwnMessageKeyboardNavigation({
    isMobile,
    scrollRef: conversationScrollRef,
  })
  const sessionLinkTargets = useSessionLinkTargets(sessions)
  const sessionElapsedSecondsById = useMemo(
    () =>
      new Map(
        sessions.map(({ id, elapsedSeconds }) => [id, elapsedSeconds] as const),
      ),
    [sessions],
  )
  // Keep memoized message props stable while the App selection callback changes.
  const onSelectSessionRef = useRef(onSelectSession)
  onSelectSessionRef.current = onSelectSession
  const selectLinkedSession = useCallback((id: string) => {
    onSelectSessionRef.current(id)
  }, [])
  const openInTerminal = useCallback(() => {
    if (onOpenInTerminal === undefined) {
      void onAction('focus')
    } else {
      onOpenInTerminal()
    }
  }, [onAction, onOpenInTerminal])
  const rewindMessage = useCallback(
    (messageId: string, mode: RewindMode, draftText: string) =>
      onRewind?.(messageId, mode, draftText) ?? Promise.resolve(false),
    [onRewind],
  )
  const {
    activeTool,
    conversationMessageCount,
    elements: conversationMessageElements,
    hasUnansweredProposal,
    pendingInteractionElements,
  } = useConversationMessageElements({
    messages,
    ...(decisions === undefined ? {} : { decisions }),
    ...(responseHasUnansweredProposal === undefined
      ? {}
      : { hasUnansweredProposal: responseHasUnansweredProposal }),
    sessionCategory: session.category,
    sessionId: session.id,
    isCompacting: session.isCompacting === true,
    sessionEngine: session.engine,
    actions,
    actionsReady,
    isMobile,
    selectedDecisionSeq,
    onSelectDecision,
    onAction,
    ...(onCommandExecuted === undefined ? {} : { onCommandExecuted }),
    onOpenInTerminal: openInTerminal,
    openInTerminalStatus,
    onSelectSession: selectLinkedSession,
    onRewind: rewindMessage,
    pendingActions: pendingPermissionActions,
    sessions: sessionLinkTargets,
    now,
  })
  const latestPendingInteractionKey =
    pendingInteractionElements.at(-1)?.key ?? null
  useLayoutEffect(() => {
    const pendingActions = pendingActionsRef.current
    if (pendingActions === null) return
    const answerableCards = pendingActions.querySelectorAll<HTMLElement>(
      '[data-answerable="true"], [data-answerable-single-choice="true"]',
    )
    if (answerableCards.length === 0) {
      pendingActions.scrollTop =
        pendingActions.scrollHeight - pendingActions.clientHeight
      return
    }
    const latestAnswerableCard = answerableCards.item(
      answerableCards.length - 1,
    )
    const answerControlGroups =
      latestAnswerableCard.querySelectorAll<HTMLElement>(
        '.chat-proposal-options, .chat-question-choices',
      )
    // The card can exceed the panel height; keep its keyboard choices visible.
    const scrollTarget =
      answerControlGroups.length === 0
        ? latestAnswerableCard
        : answerControlGroups.item(answerControlGroups.length - 1)
    scrollTarget.scrollIntoView({ block: 'nearest' })
  }, [latestPendingInteractionKey])
  const handlePasteFiles = (files: File[]) => {
    onPasteFiles(files)
    onClearError()
  }
  const interrupt = useCallback(() => {
    if (interruptInFlight.current || actions?.interrupt !== true) return
    interruptInFlight.current = true
    setIsInterrupting(true)
    const finish = () => {
      interruptInFlight.current = false
      setIsInterrupting(false)
    }
    void onAction('interrupt').then(finish, finish)
  }, [actions?.interrupt, onAction])
  const {
    isFileDragActive,
    handleDragEnter,
    handleDragOver,
    handleDragLeave,
    handleDrop,
  } = useConversationFileDrop(handlePasteFiles)

  const onComposerKeyDown = (
    event: ReactKeyboardEvent<HTMLTextAreaElement>,
  ) => {
    if (
      event.nativeEvent.isComposing ||
      Reflect.get(event.nativeEvent, 'keyCode') === 229
    ) {
      return
    }
    if (
      event.key === 'Escape' &&
      actions?.interrupt === true &&
      !interruptPending
    ) {
      event.preventDefault()
      interrupt()
      return
    }
    if (event.key === 'Enter' && !isMobile && event.altKey && !event.shiftKey) {
      event.preventDefault()
      const textarea = event.currentTarget
      textarea.setRangeText(
        '\n',
        textarea.selectionStart,
        textarea.selectionEnd,
        'end',
      )
      updateDraft(textarea.value)
      return
    }
    if (
      event.key === 'Enter' &&
      !isMobile &&
      !event.shiftKey &&
      !event.altKey
    ) {
      event.preventDefault()
      void sendMessage()
    }
  }

  const conversationPane = (
    <section
      className="conversation-pane"
      data-mobile={isMobile ? 'true' : undefined}
      aria-label={`Conversation with ${session.title}`}
      onDragEnter={handleDragEnter}
      onDragOver={handleDragOver}
      onDragLeave={handleDragLeave}
      onDrop={handleDrop}
    >
      {fileDropActive || isFileDragActive ? (
        <div className="conversation-drop-target" role="status">
          Drop files to attach
        </div>
      ) : null}
      <ConversationPaneHeader
        key={session.id}
        session={session}
        sessions={sessions}
        actions={actions}
        isMobile={isMobile}
        statusPanelOpen={statusPanelOpen}
        backgroundTasks={backgroundTasks}
        openInTerminalStatus={openInTerminalStatus}
        isToday={isToday}
        onAction={onAction}
        onOpenInTerminal={openInTerminal}
        onSelectSession={selectLinkedSession}
        onToggleStatusPanel={onToggleStatusPanel}
        {...(onToggleToday === undefined ? {} : { onToggleToday })}
        {...(onBack === undefined ? {} : { onBack })}
        {...(onLater === undefined ? {} : { onLater })}
        {...(onClose === undefined ? {} : { onClose })}
      />

      <div
        ref={conversationScrollRef}
        className="conversation-scroll"
        aria-live="polite"
        onScroll={() => {
          dismissSelectionReply()
          onScroll()
        }}
      >
        {start > 0 ? (
          <Button
            className="conversation-load-older"
            type="button"
            variant="plain"
            disabled={loadingOlder}
            onClick={loadOlder}
          >
            {loadingOlder ? 'Loading older messages…' : 'Load older messages'}
          </Button>
        ) : null}
        <div className="conversation-timeline">
          <span>Conversation</span>
        </div>
        {loading && messages.length === 0 ? (
          <p className="conversation-status">Loading conversation…</p>
        ) : null}
        {!loading && conversationMessageCount === 0 && activeTool == null ? (
          <p className="conversation-status">No messages yet.</p>
        ) : null}
        {conversationMessageElements}
      </div>

      {session.isCompacting === true ||
      session.category === 'in_progress' ||
      (isMobile && backgroundTasks.length > 0) ? (
        <ConversationWorkingStatus
          showWorking={
            session.activityStatus === 'working' ||
            (session.activityStatus == null &&
              session.category === 'in_progress' &&
              session.waitingForBackgroundTasks !== true)
          }
          isWaiting={session.activityStatus === 'waiting'}
          waitingReason={session.waitingReason}
          waitingForBackgroundTasks={session.waitingForBackgroundTasks}
          elapsedSeconds={session.elapsedSeconds}
          activeTool={activeTool}
          backgroundTasks={backgroundTasks}
          showBackgroundTasks={isMobile}
          isCompacting={session.isCompacting === true}
        />
      ) : null}

      {(error ?? attachmentError) != null &&
      (error ?? attachmentError) !== '' ? (
        <p className="conversation-error" role="alert">
          {error ?? attachmentError}
        </p>
      ) : null}
      {!storageAvailable ? (
        <p className="conversation-error" role="status">
          Browser storage is unavailable. Drafts may not survive a page reload.
        </p>
      ) : null}
      {pendingInteractionElements.length > 0 ? (
        <section
          className="conversation-pending-actions"
          aria-label="Items needing your response"
          aria-live="polite"
          data-mobile={isMobile ? 'true' : undefined}
          ref={pendingActionsRef}
        >
          {pendingInteractionElements}
        </section>
      ) : null}
      <MessageComposer
        draft={draft}
        focusDraft={focusDraft}
        engine={session.engine}
        skills={skills}
        placeholder={
          hasUnansweredProposal
            ? 'Choose an option above, or message the session'
            : session.pullRequest
              ? `Message PR ${String(session.pullRequest.number)}`
              : 'Message session'
        }
        disabled={actions?.message !== true}
        isSending={isSending}
        isProcessingAttachments={isProcessingAttachments}
        isMobile={isMobile}
        canInterrupt={actions?.interrupt === true}
        isInterrupting={isInterrupting || interruptPending}
        attachments={attachments}
        onDraftChange={updateDraft}
        onKeyDown={onComposerKeyDown}
        onPasteFiles={handlePasteFiles}
        onRemoveAttachment={removeAttachment}
        onSend={() => {
          void sendMessage()
        }}
        onInterrupt={() => {
          interrupt()
        }}
      />
      <ConversationSelectionReply
        position={selectionReplyPosition}
        onReply={replyToSelection}
      />
      {isMobile && statusPanelOpen && mobileStatusPanel !== null ? (
        <div className="mobile-session-status-overlay">{mobileStatusPanel}</div>
      ) : null}
    </section>
  )

  return (
    <SessionLinkElapsedSecondsContext.Provider
      value={sessionElapsedSecondsById}
    >
      {conversationPane}
    </SessionLinkElapsedSecondsContext.Provider>
  )
}
