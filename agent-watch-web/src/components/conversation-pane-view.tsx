import '#components/conversation-pane.css'
import '#components/conversation-pane-view.css'

import { Button } from '@fohte/ui/button'
import { type ReactNode, useCallback, useMemo, useRef, useState } from 'react'

import { useOptionalAsyncActionFeedback } from '#async-action-feedback'
import { useAgentUsageContext } from '#components/agent-usage-context'
import type {
  CommandOutputRefresh,
  CommandOutputStart,
} from '#components/command-result-message'
import { ConversationPaneHeader } from '#components/conversation-pane-header'
import { ConversationSelectionReply } from '#components/conversation-selection-reply'
import { ConversationStatusBand } from '#components/conversation-status-band'
import { getConversationStatusBandState } from '#components/conversation-status-band-state'
import { EndedSessionBanner } from '#components/ended-session-banner'
import { FileDropIndicator } from '#components/file-drop-indicator'
import { MarkdownCommandContextProvider } from '#components/markdown-command-context'
import { MessageComposer } from '#components/message-composer'
import type {
  PendingPermissionActions,
  PermissionRequestIdentity,
} from '#components/permission-card'
import {
  renderConversationAsyncActionItem,
  renderConversationAsyncActionItems,
} from '#components/render-conversation-async-action-items'
import { scrollToConversationStatusCard } from '#components/scroll-to-conversation-status-card'
import { SessionLinkElapsedSecondsContext } from '#components/session-link-elapsed-seconds'
import { SessionUsageRow } from '#components/session-usage-row'
import { useConversationCommandOutputHighlight } from '#components/use-conversation-command-output-highlight'
import { useConversationMessageElements } from '#components/use-conversation-message-elements'
import { useConversationPaneComposer } from '#components/use-conversation-pane-composer'
import { useConversationPaneInput } from '#components/use-conversation-pane-input'
import { useConversationPaneScrolling } from '#components/use-conversation-pane-scrolling'
import { useConversationSelectionReply } from '#components/use-conversation-selection-reply'
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
  onCommandExecuted?: CommandOutputRefresh
  onCommandStarted?: CommandOutputStart
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
  mobileWebUi?: ReactNode
  mobileUpdateNotice?: ReactNode
  selectedDecisionSeq?: number | null
  decisionJump?: { seq: number } | null
  onDecisionJumpComplete?: () => void
  onDecisionJumpSucceeded?: () => void
  onToggleStatusPanel?: () => void
  onOpenTasks?: () => void
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
  onCommandStarted,
  onRewind,
  onOpenInTerminal,
  onSelectSession,
  onReviewInCrit,
  isMobile = false,
  statusPanelOpen = false,
  mobileStatusPanel = null,
  mobileWebUi = null,
  mobileUpdateNotice,
  selectedDecisionSeq = null,
  decisionJump = null,
  onDecisionJumpComplete = () => undefined,
  onDecisionJumpSucceeded = () => undefined,
  onToggleStatusPanel = () => undefined,
  onOpenTasks = () => undefined,
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
  const [mergeBandTarget, setMergeBandTarget] = useState<HTMLDivElement | null>(
    null,
  )
  const sessionUsage =
    usage?.sessions.find((entry) => entry.sessionId === session.id) ?? null
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
  const {
    draft,
    updateDraft,
    storageAvailable,
    attachments,
    error: attachmentError,
    isSending,
    isProcessingAttachments,
    onPasteFiles,
    removeAttachment,
    sendMessage,
  } = useConversationPaneComposer({
    sessionId: session.id,
    messageEnabled: actions?.message !== false,
    externalSending: sending,
    onAction,
  })
  const {
    focusDraft,
    position: selectionReplyPosition,
    dismiss: dismissSelectionReply,
    reply: replyToSelection,
  } = useConversationSelectionReply({ draft, isMobile, updateDraft })
  const {
    loadOlder,
    onScroll,
    scrollRef: conversationScrollRef,
  } = useConversationPaneScrolling({
    messages,
    loadingOlder,
    onLoadOlder,
    onPruneOldMessages,
    sessionId: session.id,
    start,
    epoch,
    olderCursor,
    decisionJump,
    isMobile,
    onDecisionJumpComplete,
    onDecisionJumpSucceeded,
  })
  useConversationCommandOutputHighlight({
    loading,
    messages,
    scrollRef: conversationScrollRef,
    sessionId: session.id,
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
    conversationMessageIndices,
    elements: conversationMessageElements,
    hasUnansweredProposal,
  } = useConversationMessageElements({
    messages,
    start,
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
    ...(onCommandStarted === undefined ? {} : { onCommandStarted }),
    onOpenInTerminal: openInTerminal,
    openInTerminalStatus,
    onSelectSession: selectLinkedSession,
    onRewind: rewindMessage,
    pendingActions: pendingPermissionActions,
    sessions: sessionLinkTargets,
    now,
  })
  const conversationTimelineElements: ReactNode[] = []
  if (actionFeedback === null) {
    conversationTimelineElements.push(...conversationMessageElements)
  } else {
    const positionedActions = actionFeedback.actions
      .filter(
        (action) =>
          action.sessionId === session.id &&
          action.kind === 'merge' &&
          action.timelineIndex !== undefined &&
          action.timelineIndex >= start,
      )
      .toSorted(
        (left, right) =>
          (left.timelineIndex ?? 0) - (right.timelineIndex ?? 0) ||
          left.createdAt - right.createdAt ||
          left.id - right.id,
      )
    let nextAction = 0
    for (const [
      messageOffset,
      messageElement,
    ] of conversationMessageElements.entries()) {
      const messageIndex = conversationMessageIndices[messageOffset]
      while (
        messageIndex !== undefined &&
        nextAction < positionedActions.length &&
        (positionedActions[nextAction]?.timelineIndex ?? Infinity) <=
          messageIndex
      ) {
        const action = positionedActions[nextAction]
        if (action !== undefined) {
          conversationTimelineElements.push(
            renderConversationAsyncActionItem({
              action,
              feedback: actionFeedback,
            }),
          )
        }
        nextAction += 1
      }
      conversationTimelineElements.push(messageElement)
    }
    while (nextAction < positionedActions.length) {
      const action = positionedActions[nextAction]
      if (action !== undefined) {
        conversationTimelineElements.push(
          renderConversationAsyncActionItem({
            action,
            feedback: actionFeedback,
          }),
        )
      }
      nextAction += 1
    }
  }
  const statusBandState = getConversationStatusBandState({
    session,
    messages,
    backgroundTasks,
  })
  const returnToRequestCard = () => {
    scrollToConversationStatusCard(
      conversationScrollRef.current,
      statusBandState?.kind === 'permission',
    )
  }
  const {
    handlePasteFiles,
    interrupt,
    isFileDragActive,
    isInterrupting,
    onComposerKeyDown,
    onDragEnter,
    onDragOver,
    onDragLeave,
    onDrop,
  } = useConversationPaneInput({
    isMobile,
    canInterrupt: actions?.interrupt === true,
    interruptPending,
    onAction,
    onClearError,
    onPasteFiles,
    updateDraft,
    sendMessage,
  })

  const conversationPane = (
    <section
      className="conversation-pane"
      data-mobile={isMobile ? 'true' : undefined}
      aria-label={`Conversation with ${session.title}`}
      onDragEnter={onDragEnter}
      onDragOver={onDragOver}
      onDragLeave={onDragLeave}
      onDrop={onDrop}
    >
      {fileDropActive || isFileDragActive ? (
        <FileDropIndicator className="conversation-drop-target" />
      ) : null}
      <ConversationPaneHeader
        key={session.id}
        session={session}
        timelineIndex={start + messages.length}
        mergeBandTarget={mergeBandTarget}
        vibeMode={vibeMode}
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
        onOpenTasks={onOpenTasks}
        {...(onToggleToday === undefined ? {} : { onToggleToday })}
        {...(onBack === undefined ? {} : { onBack })}
        {...(onLater === undefined ? {} : { onLater })}
        {...(onClose === undefined ? {} : { onClose })}
      />
      {isMobile ? mobileUpdateNotice : null}

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
          sessionTitle={session.title}
          {...(onCommandExecuted === undefined ? {} : { onCommandExecuted })}
          {...(onCommandStarted === undefined ? {} : { onCommandStarted })}
        >
          {conversationTimelineElements}
        </MarkdownCommandContextProvider>
        {renderConversationAsyncActionItems({
          feedback: actionFeedback,
          sessionId: session.id,
        })}
      </div>

      <div className="conversation-merge-band-slot" ref={setMergeBandTarget} />
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
      {isMobile && mobileWebUi !== null ? (
        <div className="mobile-session-status-overlay">{mobileWebUi}</div>
      ) : isMobile && statusPanelOpen && mobileStatusPanel !== null ? (
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
