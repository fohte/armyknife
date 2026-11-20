import '#components/conversation-pane.css'
import '#components/conversation-pane-view.css'

import { Button } from '@fohte/ui/button'
import { type ReactNode, useCallback, useMemo, useRef, useState } from 'react'

import { useOptionalAsyncActionFeedback } from '#async-action-feedback'
import { useAgentUsageContext } from '#components/agent-usage-context'
import { AsyncActionItem } from '#components/async-action-item'
import { ConversationPaneHeader } from '#components/conversation-pane-header'
import { ConversationSelectionReply } from '#components/conversation-selection-reply'
import { ConversationStatusBand } from '#components/conversation-status-band'
import { getConversationStatusBandState } from '#components/conversation-status-band-state'
import { EndedSessionBanner } from '#components/ended-session-banner'
import { MarkdownCommandContextProvider } from '#components/markdown-command-context'
import { MessageComposer } from '#components/message-composer'
import type {
  PendingPermissionActions,
  PermissionRequestIdentity,
} from '#components/permission-card'
import { SessionLinkElapsedSecondsContext } from '#components/session-link-elapsed-seconds'
import { SessionUsageRow } from '#components/session-usage-row'
import { useComposerDraft } from '#components/use-composer-draft'
import { useComposerMessage } from '#components/use-composer-message'
import { useConversationFileDrop } from '#components/use-conversation-file-drop'
import { useConversationMessageElements } from '#components/use-conversation-message-elements'
import { useConversationPaneKeyboard } from '#components/use-conversation-pane-keyboard'
import { useConversationScroll } from '#components/use-conversation-scroll'
import { useConversationSelectionReply } from '#components/use-conversation-selection-reply'
import { useOwnMessageKeyboardNavigation } from '#components/use-own-message-keyboard-navigation'
import { useSessionResume } from '#components/use-session-resume'
import { useSessionLinkTargets } from '#session-link-targets'
import type {
  BackgroundTask,
  ChatItem,
  OpenInTerminalStatus,
  RewindMode,
  Session,
  SessionActions,
  SessionLinkTarget,
  SkillSuggestion,
  VibeMode,
} from '#types'

type ConversationPaneViewProps = {
  session: Session
  vibeMode?: VibeMode | undefined
  sessions?: readonly Session[]
  sessionLinkTargets?: readonly SessionLinkTarget[]
  messages: readonly ChatItem[]
  start?: number
  epoch?: string | undefined
  olderCursor?: string | undefined
  decisions?: readonly Extract<ChatItem, { type: 'decision' }>[]
  hasUnansweredProposal?: boolean
  loadingOlder?: boolean
  onLoadOlder?: () => void
  onPruneOldMessages?: () => void
  backgroundTasks?: readonly BackgroundTask[]
  skills?: readonly SkillSuggestion[]
  actions: SessionActions | null
  actionsReady?: boolean
  actionsSettled?: boolean
  isResuming?: boolean
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
  onReviewInCrit?: () => void
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
  composerFocusRequestId?: number | null
  onComposerFocusRequestHandled?: (requestId: number) => void
}

export function ConversationPaneView({
  session,
  vibeMode = session.vibeMode,
  sessions = [],
  sessionLinkTargets: providedSessionLinkTargets,
  messages,
  start = 0,
  epoch,
  olderCursor,
  decisions,
  hasUnansweredProposal: responseHasUnansweredProposal,
  loadingOlder = false,
  onLoadOlder = () => undefined,
  onPruneOldMessages = () => undefined,
  backgroundTasks = [],
  skills = [],
  actions,
  actionsReady = true,
  actionsSettled = true,
  isResuming = false,
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
  onReviewInCrit,
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
  composerFocusRequestId = null,
  onComposerFocusRequestHandled = () => undefined,
}: ConversationPaneViewProps) {
  const actionFeedback = useOptionalAsyncActionFeedback()
  const { usage } = useAgentUsageContext()
  const sessionUsage =
    usage?.sessions.find((entry) => entry.sessionId === session.id) ?? null
  const interruptInFlight = useRef(false)
  const [isInterrupting, setIsInterrupting] = useState(false)
  const isEnded = actions?.message === false
  const { isPending: isResumePending, resume } = useSessionResume(
    session.id,
    isEnded,
    isResuming,
    onAction,
  )
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
    messageEnabled: actions?.message !== false,
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
    epoch,
    olderCursor,
  })
  useOwnMessageKeyboardNavigation({
    isMobile,
    scrollRef: conversationScrollRef,
  })
  const sessionLinkTargets = useSessionLinkTargets(
    sessions,
    providedSessionLinkTargets,
  )
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
  const statusBandState = getConversationStatusBandState({
    session,
    messages,
    backgroundTasks,
  })
  const returnToRequestCard = () => {
    const cardSelector =
      statusBandState?.kind === 'permission'
        ? '.chat-permission-card:not(.chat-permission-denied-card)'
        : '.chat-question-card[data-has-unanswered-question="true"]'
    const cards =
      conversationScrollRef.current?.querySelectorAll<HTMLElement>(cardSelector)
    cards?.[cards.length - 1]?.scrollIntoView({
      behavior: 'smooth',
      block: 'center',
    })
  }
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

  const onComposerKeyDown = useConversationPaneKeyboard({
    isMobile,
    canInterrupt: actions?.interrupt === true,
    interruptPending,
    interrupt,
    updateDraft,
    sendMessage,
  })

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
        vibeMode={vibeMode}
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
        {start > 0 || (epoch !== undefined && olderCursor != null) ? (
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
        {isMobile ? null : (
          <div className="conversation-timeline">
            <span>Conversation</span>
          </div>
        )}
        {loading && messages.length === 0 ? (
          <p className="conversation-status">Loading conversation…</p>
        ) : null}
        {!loading && conversationMessageCount === 0 && activeTool == null ? (
          <p className="conversation-status">No messages yet.</p>
        ) : null}
        <MarkdownCommandContextProvider
          sessionId={session.id}
          {...(onCommandExecuted === undefined ? {} : { onCommandExecuted })}
        >
          {conversationMessageElements}
        </MarkdownCommandContextProvider>
        {actionFeedback?.actions
          .filter((action) => action.sessionId === session.id)
          .map((action) => (
            <AsyncActionItem
              action={action}
              form="line"
              key={action.id}
              onRetry={() => {
                actionFeedback.retry(action.id)
              }}
            />
          ))}
      </div>

      {isEnded ? (
        <EndedSessionBanner
          endedAt={session.updatedAt}
          isMobile={isMobile}
          isResuming={isResumePending}
          canResume={actions.resume !== false}
          onResume={resume}
        />
      ) : (
        <>
          <ConversationStatusBand
            state={statusBandState}
            elapsedSeconds={session.elapsedSeconds}
            activeTool={activeTool}
            backgroundTasks={backgroundTasks}
            isMobile={isMobile}
            {...(onReviewInCrit === undefined ? {} : { onReviewInCrit })}
            {...(statusBandState?.kind === 'question' ||
            statusBandState?.kind === 'permission'
              ? { onReturnToCard: returnToRequestCard }
              : {})}
          />

          {!storageAvailable ? (
            <p className="conversation-error" role="status">
              Browser storage is unavailable. Drafts may not survive a page
              reload.
            </p>
          ) : null}
          <MessageComposer
            draft={draft}
            focusDraft={focusDraft}
            focusRequestId={composerFocusRequestId}
            onFocusRequestHandled={onComposerFocusRequestHandled}
            actionsSettled={actionsSettled}
            engine={session.engine}
            skills={skills}
            placeholder={
              hasUnansweredProposal
                ? 'Choose an option above, or message the session'
                : session.pullRequest
                  ? `Message PR ${String(session.pullRequest.number)}`
                  : 'Message session'
            }
            disabled={actions?.message === false}
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
        </>
      )}
      {(error ?? attachmentError) != null &&
      (error ?? attachmentError) !== '' ? (
        <p className="conversation-error" role="alert">
          {error ?? attachmentError}
        </p>
      ) : null}
      <SessionUsageRow isMobile={isMobile} usage={sessionUsage} />
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
