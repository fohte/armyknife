import { Button } from '@fohte/ui/button'
import type { KeyboardEvent } from 'react'
import { useState } from 'react'

import type { ChatItem, Session, SessionActions } from '#types'

type ConversationPaneViewProps = {
  session: Session
  messages: readonly ChatItem[]
  actions: SessionActions | null
  loading: boolean
  error: string | null
  onAction: (
    action: string,
    payload?: Record<string, unknown>,
  ) => Promise<boolean>
  onSelectSession: (id: string) => void
}
export function ConversationPaneView({
  session,
  messages,
  actions,
  loading,
  error,
  onAction,
  onSelectSession,
}: ConversationPaneViewProps) {
  const [draft, setDraft] = useState('')
  const activeQuestionIndex = messages.findLastIndex(
    (message) => message.type === 'question',
  )
  const waitingQuestion = messages[activeQuestionIndex]
  const parent = session.parent

  const sendMessage = () => {
    if (draft.trim() === '' || actions?.message !== true) return
    const sentDraft = draft
    void onAction('messages', { content: sentDraft }).then((succeeded) => {
      if (succeeded) {
        setDraft((currentDraft) =>
          currentDraft === sentDraft ? '' : currentDraft,
        )
      }
    })
  }

  const onComposerKeyDown = (event: KeyboardEvent<HTMLTextAreaElement>) => {
    if (event.key === 'Escape' && actions?.interrupt === true) {
      event.preventDefault()
      void onAction('interrupt')
      return
    }
    if (event.key === 'Enter' && !event.shiftKey) {
      event.preventDefault()
      sendMessage()
    }
  }

  const onQuestionKeyDown = (event: KeyboardEvent<HTMLElement>) => {
    const target = event.target
    if (
      event.defaultPrevented ||
      event.altKey ||
      event.ctrlKey ||
      event.metaKey ||
      event.shiftKey ||
      (target instanceof HTMLElement &&
        (target.isContentEditable ||
          ['INPUT', 'TEXTAREA', 'SELECT'].includes(target.tagName)))
    )
      return
    if (
      !/^[1-9]$/.test(event.key) ||
      waitingQuestion?.type !== 'question' ||
      activeQuestionIndex < 0 ||
      actions?.answerQuestion !== true
    )
      return
    const choice = Number(event.key)
    if (choice <= waitingQuestion.choices.length) {
      event.preventDefault()
      void onAction('answer', { choice })
    }
  }

  const openInTerminal = () => {
    void onAction('focus')
  }

  return (
    <section
      className="conversation-pane"
      aria-label={`Conversation with ${session.title}`}
      onKeyDown={onQuestionKeyDown}
    >
      <header className="conversation-header">
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
          <h1>{session.title || 'Untitled session'}</h1>
        </div>
        <div className="conversation-header-actions">
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
      </header>

      <div className="conversation-scroll" aria-live="polite">
        <div className="conversation-timeline">
          <span>Conversation</span>
        </div>
        {loading && messages.length === 0 ? (
          <p className="conversation-status">Loading conversation…</p>
        ) : null}
        {!loading && messages.length === 0 ? (
          <p className="conversation-status">No messages yet.</p>
        ) : null}
        {messages.map((message, index) => {
          if (message.type === 'user') {
            return (
              <article
                className="chat-user-group"
                key={`user-${String(index)}`}
              >
                {message.from != null && message.from !== '' ? (
                  <div className="chat-from">From {message.from}</div>
                ) : null}
                <div className="chat-user-message">{message.text}</div>
              </article>
            )
          }
          if (message.type === 'assistant')
            return (
              <article
                className="chat-assistant-message"
                key={`assistant-${String(index)}`}
              >
                {message.text}
              </article>
            )
          if (message.type === 'tool_summary')
            return (
              <details
                className="chat-tool-summary"
                key={`tool-${String(index)}`}
              >
                <summary>▸ {message.summary}</summary>
              </details>
            )
          if (message.type === 'question') {
            const canAnswer =
              index === activeQuestionIndex && actions?.answerQuestion === true
            return (
              <article
                className="chat-question-card"
                key={`question-${String(index)}`}
              >
                <h2>Question</h2>
                <p>{message.question}</p>
                <div className="chat-question-choices">
                  {message.choices.map((choice, choiceIndex) => (
                    <Button
                      type="button"
                      variant="plain"
                      key={`${String(choiceIndex)}-${choice}`}
                      disabled={!canAnswer}
                      onClick={() => {
                        void onAction('answer', { choice: choiceIndex + 1 })
                      }}
                    >
                      <kbd>{choiceIndex + 1}</kbd>
                      <span>{choice}</span>
                    </Button>
                  ))}
                </div>
              </article>
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
              {actions?.permission != null ? (
                <div className="permission-actions">
                  <Button
                    type="button"
                    variant="plain"
                    disabled={!actions.permission.approve}
                    onClick={() => {
                      void onAction('permission/approve')
                    }}
                  >
                    Approve
                  </Button>
                  <Button
                    type="button"
                    variant="plain"
                    disabled={!actions.permission.reject}
                    onClick={() => {
                      void onAction('permission/reject')
                    }}
                  >
                    Reject
                  </Button>
                </div>
              ) : null}
            </article>
          )
        })}
      </div>

      {error != null && error !== '' ? (
        <p className="conversation-error" role="alert">
          {error}
        </p>
      ) : null}
      <form
        className="conversation-composer"
        onSubmit={(event) => {
          event.preventDefault()
          sendMessage()
        }}
      >
        <textarea
          aria-label="Message"
          placeholder={
            session.pullRequest
              ? `Message PR ${String(session.pullRequest.number)}`
              : 'Message session'
          }
          value={draft}
          onChange={(event) => {
            setDraft(event.currentTarget.value)
          }}
          onKeyDown={onComposerKeyDown}
          disabled={actions?.message !== true}
          rows={2}
        />
        <div className="composer-footer">
          <span>↵ Send · ⇧↵ New line · Esc Interrupt</span>
          <Button
            type="submit"
            variant="plain"
            aria-label="Send message"
            disabled={draft.trim() === '' || actions?.message !== true}
          >
            ↑
          </Button>
        </div>
      </form>
    </section>
  )
}
