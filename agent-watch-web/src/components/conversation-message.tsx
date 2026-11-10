import { Button } from '@fohte/ui/button'
import type { ReactNode } from 'react'

import { MarkdownMessage } from '#components/markdown-message'
import { QuestionCard } from '#components/question-card'
import { ToolSummary } from '#components/tool-summary'
import { UserWithContent } from '#components/user-with-content'
import type {
  ChatItem,
  Session,
  SessionActions,
  SessionLinkTarget,
} from '#types'

type RenderConversationMessageOptions = {
  actions: SessionActions | null
  actionsReady: boolean
  activeQuestionIndex: number
  index: number
  isMobile: boolean
  message: ChatItem
  onAction: (
    action: string,
    payload?: Record<string, unknown>,
  ) => Promise<boolean>
  onOpenInTerminal: () => void
  onSelectSession: (id: string) => void
  renderTerminalFallback: (message: string) => ReactNode
  session: Session
  sessions: readonly SessionLinkTarget[]
}

export function renderConversationMessage({
  actions,
  actionsReady,
  activeQuestionIndex,
  index,
  isMobile,
  message,
  onAction,
  onOpenInTerminal,
  onSelectSession,
  renderTerminalFallback,
  session,
  sessions,
}: RenderConversationMessageOptions): ReactNode {
  if (message.type === 'user' || message.type === 'user_with_content') {
    return (
      <UserWithContent
        key={`user-${String(index)}`}
        message={message}
        onSelectSession={onSelectSession}
        sessions={sessions}
      />
    )
  }
  if (message.type === 'assistant') {
    return (
      <article
        className="chat-assistant-message"
        key={`assistant-${String(index)}`}
      >
        <MarkdownMessage onSelectSession={onSelectSession} sessions={sessions}>
          {message.text}
        </MarkdownMessage>
      </article>
    )
  }
  if (message.type === 'tool_summary') {
    return (
      <ToolSummary
        key={`tool-${String(index)}`}
        summary={message.summary}
        operations={message.operations ?? []}
      />
    )
  }
  if (message.type === 'tool_in_progress') {
    return (
      <ToolSummary
        key={`tool-${String(index)}`}
        summary="1 action"
        operations={[message]}
      />
    )
  }
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
        onOpenInTerminal={onOpenInTerminal}
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
}
