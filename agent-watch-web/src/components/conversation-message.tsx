import type { ReactNode } from 'react'

import { MarkdownMessage } from '#components/markdown-message'
import { PeerMessage } from '#components/peer-message'
import {
  type PendingPermissionAction,
  PermissionCard,
  type PermissionRequestIdentity,
} from '#components/permission-card'
import { ProposalCard } from '#components/proposal-card'
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
  pendingAction: PendingPermissionAction
  onAction: (
    action: string,
    payload?: Record<string, unknown>,
    permissionIdentity?: PermissionRequestIdentity,
  ) => Promise<boolean>
  onOpenInTerminal: () => void
  onSelectSession: (id: string) => void
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
  pendingAction,
  onAction,
  onOpenInTerminal,
  onSelectSession,
  session,
  sessions,
}: RenderConversationMessageOptions): ReactNode {
  if (
    message.type === 'user' ||
    message.type === 'queued_user' ||
    message.type === 'user_with_content'
  ) {
    return (
      <UserWithContent
        key={`user-${String(index)}`}
        message={message}
        onSelectSession={onSelectSession}
        sessions={sessions}
      />
    )
  }
  if (message.type === 'peer') {
    return (
      <PeerMessage
        key={`peer-${String(index)}`}
        message={message}
        onSelectSession={onSelectSession}
        sessions={sessions}
      />
    )
  }
  if (message.type === 'event') {
    return (
      <div
        className="chat-event"
        data-failed={message.failed ? 'true' : undefined}
        data-kind={message.kind}
        key={`event-${String(index)}`}
        title={message.text}
      >
        {message.text}
      </div>
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
  if (message.type === 'proposal') {
    return (
      <ProposalCard
        key={`proposal-${String(message.seq)}`}
        seq={message.seq}
        title={message.title}
        summary={message.summary}
        options={message.options}
        answer={message.answer}
        resolved={message.resolved}
        isMobile={isMobile}
        onAnswer={(answer) =>
          onAction(`proposals/${String(message.seq)}/answer`, {
            choice: answer.choice,
            comment: answer.comment,
          })
        }
      />
    )
  }
  return (
    <PermissionCard
      key={`permission-${String(index)}`}
      message={message}
      actions={actions}
      actionsReady={actionsReady}
      pendingAction={pendingAction}
      isCodex={session.engine === 'Codex'}
      onAction={(action, permissionIdentity) => {
        void onAction(action, undefined, permissionIdentity)
      }}
      onOpenInTerminal={onOpenInTerminal}
    />
  )
}
