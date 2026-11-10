import { Button } from '@fohte/ui/button'
import type { KeyboardEvent as ReactKeyboardEvent } from 'react'
import { useEffect, useLayoutEffect, useRef, useState } from 'react'

import { ConversationWorkingStatus } from '#components/conversation-working-status'
import { CritPane } from '#components/crit-pane'
import { MarkdownMessage } from '#components/markdown-message'
import { MessageComposer } from '#components/message-composer'
import { QuestionCard } from '#components/question-card'
import { renderConversationCritColumns } from '#components/resizable-columns'
import { SessionHeading } from '#components/session-heading'
import { ToolSummary } from '#components/tool-summary'
import { useComposerDraft } from '#components/use-composer-draft'
import { useComposerMessage } from '#components/use-composer-message'
import type { ChatItem, Session, SessionActions } from '#types'

type ConversationPaneViewProps = {
  session: Session
  sessions?: readonly Session[]
  messages: readonly ChatItem[]
  actions: SessionActions | null
  actionsReady?: boolean
  loading: boolean
  sending?: boolean
  error: string | null
  onClearError?: () => void
  onAction: (
    action: string,
    payload?: Record<string, unknown>,
  ) => Promise<boolean>
  onSelectSession: (id: string) => void
  isMobile?: boolean
  onBack?: () => void
  onLater?: () => void
}

export function ConversationPaneView({
  session,
  sessions = [],
  messages,
  actions,
  actionsReady = true,
  loading,
  sending = false,
  error,
  onClearError = () => undefined,
  onAction,
  onSelectSession,
  isMobile = false,
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
  const [openCritUrl, setOpenCritUrl] = useState<string | null>(null)
  const scrollRef = useRef<HTMLDivElement>(null)
  const followLatestRef = useRef(true)
  const activeQuestionIndex = messages.findLastIndex(
    (message) => message.type === 'question',
  )
  const activeTool = messages.findLast(
    (message) => message.type === 'tool_in_progress',
  )
  const conversationMessages = messages.filter(
    (message) =>
      message.type !== 'tool_in_progress' || session.category !== 'in_progress',
  )
  const waitingQuestion = messages[activeQuestionIndex]
  const parent = session.parent
  const isReview = session.kind === 'Review'
  const critUrl =
    isReview && session.category === 'needs_action' ? session.critUrl : null
  const hasCritUrl = typeof critUrl === 'string' && critUrl !== ''
  const critOpen = hasCritUrl && openCritUrl === critUrl

  const handlePasteFiles = (files: File[]) => {
    onPasteFiles(files)
    onClearError()
  }

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

  const renderTerminalFallback = (message: string) => (
    <div className="conversation-terminal-fallback">
      <span>{message}</span>
      {actions?.openInTerminal === true ? (
        <Button type="button" variant="plain" onClick={openInTerminal}>
          Open in terminal
        </Button>
      ) : null}
    </div>
  )

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
    >
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
                {session.task ? (
                  <span>
                    {session.task.priority != null
                      ? `P${String(session.task.priority)} `
                      : ''}
                    #{session.task.number} {session.task.title}
                  </span>
                ) : null}
                {session.task ? <span aria-hidden="true"> · </span> : null}
                <span>{session.repo.split('/').at(-1)}</span>
              </div>
              <SessionHeading
                title={session.title}
                isPaused={session.isPaused}
              />
            </div>
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
                {session.task ? (
                  <a
                    className="conversation-task-link"
                    href={`https://tq.fohte.net/tasks/${encodeURIComponent(session.task.id)}`}
                    target="_blank"
                    rel="noreferrer"
                  >
                    {session.task.priority != null ? (
                      <span>P{session.task.priority}</span>
                    ) : null}{' '}
                    #{session.task.number} {session.task.title}
                  </a>
                ) : null}
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
              {hasCritUrl ? (
                <Button
                  className="conversation-crit-toggle"
                  type="button"
                  variant="plain"
                  aria-expanded={critOpen}
                  onClick={() => {
                    setOpenCritUrl(critOpen ? null : critUrl)
                  }}
                >
                  {critOpen ? 'Hide crit' : 'Open crit'}
                </Button>
              ) : null}
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
        {conversationMessages.map((message, index) => {
          if (message.type === 'user') {
            return (
              <article
                className="chat-user-group"
                key={`user-${String(index)}`}
              >
                {message.from != null && message.from !== '' ? (
                  <div className="chat-from">From {message.from}</div>
                ) : null}
                <MarkdownMessage
                  className="chat-user-message"
                  onSelectSession={onSelectSession}
                  sessions={sessions}
                >
                  {message.text}
                </MarkdownMessage>
              </article>
            )
          }
          if (message.type === 'assistant')
            return (
              <article
                className="chat-assistant-message"
                key={`assistant-${String(index)}`}
              >
                <MarkdownMessage
                  onSelectSession={onSelectSession}
                  sessions={sessions}
                >
                  {message.text}
                </MarkdownMessage>
              </article>
            )
          if (message.type === 'tool_summary')
            return (
              <ToolSummary
                key={`tool-${String(index)}`}
                summary={message.summary}
                operations={message.operations ?? []}
              />
            )
          if (message.type === 'tool_in_progress')
            return (
              <ToolSummary
                key={`tool-${String(index)}`}
                summary="1 action"
                operations={[message]}
              />
            )
          if (message.type === 'question') {
            const canAnswer =
              index === activeQuestionIndex &&
              message.answer === null &&
              actionsReady &&
              actions?.answerQuestion === true
            return (
              <QuestionCard
                key={`question-${String(index)}`}
                question={message.question}
                choices={message.choices}
                answer={message.answer}
                canAnswer={canAnswer}
                isMobile={isMobile}
                showTerminalFallback={
                  session.engine === 'Codex' &&
                  actions !== null &&
                  !actions.answerQuestion
                }
                canOpenInTerminal={actions?.openInTerminal === true}
                onAnswer={(choice) => {
                  void onAction('answer', { choice })
                }}
                onOpenInTerminal={openInTerminal}
              />
            )
          }
          return (
            <article
              className="chat-permission-card"
              key={`permission-${String(index)}`}
            >
              <h2>Permission required</h2>
              <p className="permission-tool">{message.tool}</p>
              <p>{message.description}</p>
              {actions?.permission != null &&
              (actions.permission.approve || actions.permission.reject) ? (
                <div className="permission-actions">
                  <Button
                    type="button"
                    variant="plain"
                    disabled={!actionsReady || !actions.permission.approve}
                    onClick={() => {
                      void onAction('permission/approve')
                    }}
                  >
                    Approve
                  </Button>
                  <Button
                    type="button"
                    variant="plain"
                    disabled={!actionsReady || !actions.permission.reject}
                    onClick={() => {
                      void onAction('permission/reject')
                    }}
                  >
                    Reject
                  </Button>
                </div>
              ) : session.engine === 'Codex' && actions !== null ? (
                renderTerminalFallback('Review this permission in terminal.')
              ) : null}
            </article>
          )
        })}
      </div>

      {session.category === 'in_progress' ? (
        <ConversationWorkingStatus
          elapsedSeconds={session.elapsedSeconds}
          activeTool={activeTool}
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

  const closeCrit = () => {
    setOpenCritUrl(null)
  }
  if (isMobile) return conversationPane

  return renderConversationCritColumns(
    conversationPane,
    critOpen ? <CritPane url={critUrl} onClose={closeCrit} /> : null,
  )
}
