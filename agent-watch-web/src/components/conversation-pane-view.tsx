import { Button } from '@fohte/ui/button'
import type { KeyboardEvent as ReactKeyboardEvent } from 'react'
import { useCallback, useEffect, useLayoutEffect, useRef } from 'react'

import { renderConversationMessage } from '#components/conversation-message'
import { ConversationWorkingStatus } from '#components/conversation-working-status'
import { CopySessionIdControl } from '#components/copy-session-id-control'
import { MessageComposer } from '#components/message-composer'
import type {
  PendingPermissionAction,
  PermissionRequestIdentity,
} from '#components/permission-card'
import { SessionHeading } from '#components/session-heading'
import { SessionTaskLinks } from '#components/session-task-links'
import { useComposerDraft } from '#components/use-composer-draft'
import { useComposerMessage } from '#components/use-composer-message'
import { useConversationFileDrop } from '#components/use-conversation-file-drop'
import type {
  BackgroundTask,
  ChatItem,
  Session,
  SessionActions,
  SessionLinkTarget,
} from '#types'

type ConversationPaneViewProps = {
  session: Session
  sessions?: readonly Session[]
  messages: readonly ChatItem[]
  backgroundTasks?: readonly BackgroundTask[]
  actions: SessionActions | null
  actionsReady?: boolean
  pendingPermissionAction?: PendingPermissionAction | undefined
  loading: boolean
  sending?: boolean
  error: string | null
  onClearError?: () => void
  onAction: (
    action: string,
    payload?: Record<string, unknown>,
    permissionIdentity?: PermissionRequestIdentity,
  ) => Promise<boolean>
  onSelectSession: (id: string) => void
  isMobile?: boolean
  statusPanelOpen?: boolean
  onToggleStatusPanel?: () => void
  runningCount?: number
  fileDropActive?: boolean
  onBack?: () => void
  onLater?: () => void
}

function useSessionLinkTargets(
  sessions: readonly Session[],
): readonly SessionLinkTarget[] {
  // Elapsed-time updates replace session objects even when link details are unchanged.
  const targetsRef = useRef<readonly SessionLinkTarget[]>([])
  const nextTargets = sessions.map(
    ({ id, category, isPaused, task, repo }) => ({
      id,
      category,
      isPaused,
      task,
      repo,
    }),
  )
  const targetsAreUnchanged =
    targetsRef.current.length === nextTargets.length &&
    targetsRef.current.every((target, index) => {
      const nextTarget = nextTargets[index]
      return (
        nextTarget !== undefined &&
        target.id === nextTarget.id &&
        target.category === nextTarget.category &&
        target.isPaused === nextTarget.isPaused &&
        target.task?.id === nextTarget.task?.id &&
        target.task?.number === nextTarget.task?.number &&
        target.task?.title === nextTarget.task?.title &&
        target.repo === nextTarget.repo
      )
    })

  if (!targetsAreUnchanged) targetsRef.current = nextTargets
  return targetsRef.current
}

export function ConversationPaneView({
  session,
  sessions = [],
  messages,
  backgroundTasks = [],
  actions,
  actionsReady = true,
  pendingPermissionAction = null,
  loading,
  sending = false,
  error,
  onClearError = () => undefined,
  onAction,
  onSelectSession,
  isMobile = false,
  statusPanelOpen = false,
  onToggleStatusPanel = () => undefined,
  runningCount = 0,
  fileDropActive = false,
  onBack,
  onLater,
}: ConversationPaneViewProps) {
  const { draft, updateDraft, clearDraftIfUnchanged, storageAvailable } =
    useComposerDraft(session.id)
  const {
    attachments,
    error: attachmentError,
    isSending,
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
  const scrollRef = useRef<HTMLDivElement>(null)
  const followLatestRef = useRef(true)
  const sessionLinkTargets = useSessionLinkTargets(sessions)
  // Keep memoized message props stable while the App selection callback changes.
  const onSelectSessionRef = useRef(onSelectSession)
  onSelectSessionRef.current = onSelectSession
  const selectLinkedSession = useCallback((id: string) => {
    onSelectSessionRef.current(id)
  }, [])
  const activeQuestionIndex = messages.findLastIndex(
    (message) => message.type === 'question',
  )
  const activeTool = messages.findLast(
    (message) => message.type === 'tool_in_progress',
  )
  const conversationMessages = messages.filter(
    (message) =>
      message.type !== 'tool_in_progress' ||
      (session.category !== 'in_progress' && session.isCompacting !== true),
  )
  const waitingQuestion = messages[activeQuestionIndex]
  const parent = session.parent
  const isReview = session.kind === 'Review'
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
    if (event.key === 'Enter' && !isMobile && !event.shiftKey) {
      event.preventDefault()
      sendMessage()
    }
  }

  const openInTerminal = () => {
    void onAction('focus')
  }

  useEffect(() => {
    const onQuestionKeyDown = (event: WindowEventMap['keydown']) => {
      const target = event.target
      if (
        isMobile ||
        event.defaultPrevented ||
        event.altKey ||
        event.ctrlKey ||
        event.metaKey ||
        event.shiftKey ||
        (target instanceof HTMLElement &&
          (target.isContentEditable ||
            ['INPUT', 'TEXTAREA', 'SELECT'].includes(target.tagName)))
      ) {
        return
      }
      if (
        !/^[1-9]$/.test(event.key) ||
        waitingQuestion?.type !== 'question' ||
        waitingQuestion.answer !== null ||
        activeQuestionIndex < 0 ||
        !actionsReady ||
        actions?.answerQuestion !== true
      ) {
        return
      }
      const choice = Number(event.key)
      if (choice <= waitingQuestion.choices.length) {
        event.preventDefault()
        void onAction('answer', { choice })
      }
    }

    window.addEventListener('keydown', onQuestionKeyDown)
    return () => {
      window.removeEventListener('keydown', onQuestionKeyDown)
    }
  }, [
    actions?.answerQuestion,
    actionsReady,
    activeQuestionIndex,
    isMobile,
    onAction,
    waitingQuestion,
  ])

  useLayoutEffect(() => {
    const scrollContainer = scrollRef.current
    if (scrollContainer !== null && followLatestRef.current) {
      scrollContainer.scrollTop =
        scrollContainer.scrollHeight - scrollContainer.clientHeight
    }
  }, [messages])

  const updateFollowLatest = () => {
    const scrollContainer = scrollRef.current
    if (scrollContainer !== null) {
      followLatestRef.current =
        scrollContainer.scrollHeight -
          scrollContainer.scrollTop -
          scrollContainer.clientHeight <=
        24
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
          Drop PNG, JPEG, GIF, or WebP images to attach
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
              ‹
            </Button>
            <div className="mobile-conversation-heading">
              <div className="mobile-conversation-context">
                <SessionTaskLinks task={session.task} tasks={linkedTasks} />
                {session.task ? <span aria-hidden="true">·</span> : null}
                <span>{session.repo.split('/').at(-1)}</span>
              </div>
              <SessionHeading
                title={session.title}
                isPaused={session.isPaused}
              />
            </div>
            <CopySessionIdControl sessionId={session.id} compact />
            <Button
              className="mobile-conversation-later cursor-pointer"
              type="button"
              variant="plain"
              onClick={onLater}
            >
              Later
            </Button>
          </>
        ) : (
          <>
            <div className="conversation-header-copy">
              <div className="conversation-context-line">
                <SessionTaskLinks task={session.task} tasks={linkedTasks} />
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
              </div>
              <SessionHeading
                title={session.title}
                isPaused={session.isPaused}
              />
            </div>
            <div className="conversation-header-actions">
              <Button
                className="conversation-status-panel-toggle"
                type="button"
                variant="plain"
                aria-label="Toggle session status panel with Command Period"
                aria-expanded={statusPanelOpen}
                aria-controls="session-status-panel"
                onClick={() => {
                  onToggleStatusPanel()
                }}
              >
                <span aria-hidden="true">◐</span>
                {runningCount > 0 ? <span>{runningCount}</span> : null}
              </Button>
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
              <CopySessionIdControl sessionId={session.id} />
              {actions?.openInTerminal === true ? (
                <Button type="button" variant="plain" onClick={openInTerminal}>
                  Open in terminal
                </Button>
              ) : null}
            </div>
          </>
        )}
      </header>

      {isMobile && isReview ? (
        <p className="mobile-review-note">
          Open this session on your Mac to review it in crit.
        </p>
      ) : null}

      <div
        ref={scrollRef}
        className="conversation-scroll"
        aria-live="polite"
        onScroll={updateFollowLatest}
      >
        <div className="conversation-timeline">
          <span>Conversation</span>
        </div>
        {loading && messages.length === 0 ? (
          <p className="conversation-status">Loading conversation…</p>
        ) : null}
        {!loading && conversationMessages.length === 0 && activeTool == null ? (
          <p className="conversation-status">No messages yet.</p>
        ) : null}
        {conversationMessages.map((message, index) =>
          renderConversationMessage({
            actions,
            actionsReady,
            activeQuestionIndex,
            index,
            isMobile,
            message,
            onAction,
            onOpenInTerminal: openInTerminal,
            onSelectSession: selectLinkedSession,
            pendingAction: pendingPermissionAction,
            session,
            sessions: sessionLinkTargets,
          }),
        )}
      </div>

      {session.isCompacting === true ||
      session.category === 'in_progress' ||
      backgroundTasks.length > 0 ? (
        <ConversationWorkingStatus
          showWorking={session.category === 'in_progress'}
          elapsedSeconds={session.elapsedSeconds}
          activeTool={activeTool}
          backgroundTasks={backgroundTasks}
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
        placeholder={
          session.pullRequest
            ? `Message PR ${String(session.pullRequest.number)}`
            : 'Message session'
        }
        disabled={actions?.message !== true}
        isSending={isSending}
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
    </section>
  )

  if (isMobile) return conversationPane

  return conversationPane
}
