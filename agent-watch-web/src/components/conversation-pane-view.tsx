import '#components/conversation-pane.css'
import '#components/conversation-pane-view.css'

import { Button } from '@fohte/ui/button'
import { ChevronLeft, PanelRightClose, PanelRightOpen } from 'lucide-react'
import {
  type KeyboardEvent as ReactKeyboardEvent,
  type ReactNode,
  useCallback,
  useMemo,
  useRef,
} from 'react'

import { ConversationWorkingStatus } from '#components/conversation-working-status'
import { MessageComposer } from '#components/message-composer'
import { OpenInTerminalAction } from '#components/open-in-terminal-action'
import type {
  PendingPermissionActions,
  PermissionRequestIdentity,
} from '#components/permission-card'
import { SessionHeading } from '#components/session-heading'
import { SessionLinkElapsedSecondsContext } from '#components/session-link-elapsed-seconds'
import { SessionTaskLinks } from '#components/session-task-links'
import { StarredSessionToggle } from '#components/starred-session-toggle'
import { useComposerDraft } from '#components/use-composer-draft'
import { useComposerMessage } from '#components/use-composer-message'
import { useConversationFileDrop } from '#components/use-conversation-file-drop'
import { useConversationMessageElements } from '#components/use-conversation-message-elements'
import { useConversationScroll } from '#components/use-conversation-scroll'
import { useSessionLinkTargets } from '#session-link-targets'
import type {
  BackgroundTask,
  ChatItem,
  OpenInTerminalStatus,
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
  error: string | null
  onClearError?: () => void
  onAction: (
    action: string,
    payload?: Record<string, unknown>,
    permissionIdentity?: PermissionRequestIdentity,
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
  error,
  onClearError = () => undefined,
  onAction,
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
  const { draft, updateDraft, clearDraftIfUnchanged, storageAvailable } =
    useComposerDraft(session.id)
  const {
    attachments,
    error: attachmentError,
    isSending,
    isProcessingAttachments,
    onPasteFiles,
    removeAttachment,
    sendMessage,
  } = useComposerMessage({
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
  const {
    activeTool,
    conversationMessageCount,
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
    isCompacting: session.isCompacting === true,
    sessionEngine: session.engine,
    actions,
    actionsReady,
    isMobile,
    selectedDecisionSeq,
    onSelectDecision,
    onAction,
    onOpenInTerminal: openInTerminal,
    openInTerminalStatus,
    onSelectSession: selectLinkedSession,
    pendingActions: pendingPermissionActions,
    sessions: sessionLinkTargets,
  })
  const parent = session.parent
  const linkedTasks =
    session.tasks ?? (session.task === null ? [] : [session.task])

  const handlePasteFiles = (files: File[]) => {
    onPasteFiles(files)
    onClearError()
  }
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
    if (event.key === 'Escape' && actions?.interrupt === true) {
      event.preventDefault()
      void onAction('interrupt')
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
      sendMessage()
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
      <header className="conversation-header">
        {isMobile ? (
          <>
            <Button
              className="mobile-conversation-back cursor-pointer"
              type="button"
              variant="plain"
              aria-label="Back to Inbox"
              onClick={onBack}
            >
              <ChevronLeft aria-hidden="true" size={30} strokeWidth={2.5} />
            </Button>
            <div className="mobile-conversation-heading">
              <SessionHeading
                title={session.title}
                isPaused={session.isPaused}
                sessionId={session.id}
              />
              <div className="mobile-conversation-context">
                <SessionTaskLinks
                  task={session.task}
                  tasks={linkedTasks}
                  sessions={sessions}
                  onMakePrimary={(taskId) =>
                    onAction('primary-task', { taskId })
                  }
                />
                {session.task ? <span aria-hidden="true">·</span> : null}
                <span>{session.repo.split('/').at(-1)}</span>
              </div>
            </div>
            <Button
              className="mobile-conversation-status-toggle"
              type="button"
              variant="plain"
              aria-label={
                statusPanelOpen
                  ? 'Close session status panel'
                  : 'Open session status panel'
              }
              aria-expanded={statusPanelOpen}
              aria-controls="session-status-panel"
              title={
                statusPanelOpen
                  ? 'Close session status panel'
                  : 'Open session status panel'
              }
              onClick={() => {
                onToggleStatusPanel()
              }}
            >
              {statusPanelOpen ? (
                <PanelRightClose aria-hidden="true" size={18} />
              ) : (
                <PanelRightOpen aria-hidden="true" size={18} />
              )}
              {backgroundTasks.length > 0 ? (
                <span className="conversation-status-panel-count">
                  {backgroundTasks.length}
                </span>
              ) : null}
            </Button>
            <Button
              className="mobile-conversation-later cursor-pointer"
              type="button"
              variant="plain"
              onClick={onLater}
            >
              Later
            </Button>
            {session.category === 'done' && onClose !== undefined ? (
              <Button
                className="mobile-conversation-close cursor-pointer"
                type="button"
                variant="plain"
                onClick={onClose}
              >
                Close
              </Button>
            ) : null}
            <StarredSessionToggle
              className="mobile-conversation-today cursor-pointer"
              size="mobile"
              isStarred={isToday}
              onToggleToday={onToggleToday}
            />
          </>
        ) : (
          <>
            <div className="conversation-header-copy">
              <div className="conversation-context-line">
                <SessionTaskLinks
                  task={session.task}
                  tasks={linkedTasks}
                  sessions={sessions}
                  onMakePrimary={(taskId) =>
                    onAction('primary-task', { taskId })
                  }
                />
                <a
                  className="conversation-repo"
                  href={`https://github.com/${session.repo}`}
                  target="_blank"
                  rel="noreferrer"
                >
                  {session.repo}
                </a>
                {parent != null ? (
                  <a
                    className="conversation-parent"
                    href={`#${encodeURIComponent(parent.id)}`}
                    onClick={(event) => {
                      event.preventDefault()
                      onSelectSession(parent.id)
                    }}
                  >
                    ↑ Parent: {parent.title ?? 'session'}
                  </a>
                ) : null}
                <div className="conversation-header-actions">
                  <Button
                    className="conversation-status-panel-toggle"
                    type="button"
                    variant="plain"
                    aria-label={
                      statusPanelOpen
                        ? 'Close session status panel'
                        : 'Open session status panel'
                    }
                    aria-expanded={statusPanelOpen}
                    aria-controls="session-status-panel"
                    title={
                      statusPanelOpen
                        ? 'Close session status panel'
                        : 'Open session status panel'
                    }
                    onClick={() => {
                      onToggleStatusPanel()
                    }}
                  >
                    {statusPanelOpen ? (
                      <PanelRightClose aria-hidden="true" size={15} />
                    ) : (
                      <PanelRightOpen aria-hidden="true" size={15} />
                    )}
                    {backgroundTasks.length > 0 ? (
                      <span className="conversation-status-panel-count">
                        {backgroundTasks.length}
                      </span>
                    ) : null}
                  </Button>
                  <StarredSessionToggle
                    className="conversation-today-toggle"
                    size="desktop"
                    isStarred={isToday}
                    onToggleToday={onToggleToday}
                  />
                  {session.pullRequest ? (
                    <a
                      className="conversation-pr-link"
                      href={`https://github.com/${session.repo}/pull/${String(session.pullRequest.number)}`}
                      target="_blank"
                      rel="noreferrer"
                    >
                      #{session.pullRequest.number} PR
                    </a>
                  ) : null}
                  {session.category === 'done' && onClose !== undefined ? (
                    <Button
                      className="conversation-close cursor-pointer"
                      type="button"
                      variant="plain"
                      onClick={onClose}
                    >
                      Close
                    </Button>
                  ) : null}
                  {actions?.openInTerminal === true ? (
                    <OpenInTerminalAction
                      status={openInTerminalStatus}
                      onClick={openInTerminal}
                    />
                  ) : null}
                </div>
              </div>
              <SessionHeading
                title={session.title}
                isPaused={session.isPaused}
                sessionId={session.id}
              />
            </div>
          </>
        )}
      </header>

      <div
        ref={conversationScrollRef}
        className="conversation-scroll"
        aria-live="polite"
        onScroll={onScroll}
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
      <MessageComposer
        draft={draft}
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
        attachments={attachments}
        onDraftChange={updateDraft}
        onKeyDown={onComposerKeyDown}
        onPasteFiles={handlePasteFiles}
        onRemoveAttachment={removeAttachment}
        onSend={sendMessage}
        onInterrupt={() => {
          void onAction('interrupt')
        }}
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
