import '#components/conversation-message.css'

import type { ReactNode } from 'react'

import { DecisionCard } from '#components/decision-card'
import { MarkdownMessage } from '#components/markdown-message'
import { formatMessageTime } from '#components/message-time'
import { PeerMessage } from '#components/peer-message'
import {
  type PendingPermissionActionState,
  PermissionCard,
  type PermissionRequestIdentity,
} from '#components/permission-card'
import { ProposalCard } from '#components/proposal-card'
import { QuestionCard } from '#components/question-card'
import { SubagentReport } from '#components/subagent-report'
import { TodoCard } from '#components/todo-card'
import { ToolSummary } from '#components/tool-summary'
import { UserWithContent } from '#components/user-with-content'
import type {
  ChatItem,
  OpenInTerminalStatus,
  Session,
  SessionActions,
  SessionLinkTarget,
} from '#types'

export type ConversationMessageRowProps = {
  actions: SessionActions | null
  actionsReady: boolean
  activeQuestion: boolean
  index: number
  isMobile: boolean
  selected: boolean
  decisionPosition: string | undefined
  onSelectDecision: (seq: number) => void
  message: ChatItem
  pendingAction: PendingPermissionActionState
  openInTerminalStatus: OpenInTerminalStatus
  onAction: (
    action: string,
    payload?: Record<string, unknown>,
    permissionIdentity?: PermissionRequestIdentity,
  ) => Promise<boolean>
  onOpenInTerminal: () => void
  onSelectSession: (id: string) => void
  sessionEngine: Session['engine']
  sessions: readonly SessionLinkTarget[]
}

export function renderConversationMessage({
  actions,
  actionsReady,
  activeQuestion,
  index,
  isMobile,
  selected,
  decisionPosition,
  onSelectDecision,
  message,
  pendingAction,
  openInTerminalStatus,
  onAction,
  onOpenInTerminal,
  onSelectSession,
  sessionEngine,
  sessions,
}: ConversationMessageRowProps): ReactNode {
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
  if (message.type === 'subagent_report') {
    return (
      <SubagentReport
        key={`subagent-report-${String(index)}`}
        message={message}
        onSelectSession={onSelectSession}
        sessions={sessions}
      />
    )
  }
  if (message.type === 'event') {
    const time = formatMessageTime(message.timestamp)
    return (
      <div
        className="chat-event"
        data-failed={message.failed ? 'true' : undefined}
        data-kind={message.kind}
        key={`event-${String(index)}`}
        title={message.text}
      >
        <span className="chat-event-text">{message.text}</span>
        {time !== null && (
          <>
            {' '}
            <time dateTime={message.timestamp}>{time}</time>
          </>
        )}
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
      activeQuestion &&
      message.answer === null &&
      actionsReady &&
      actions?.answerQuestion === true
    return (
      <QuestionCard
        key={`question-${String(index)}`}
        {...(message.header === undefined ? {} : { header: message.header })}
        question={message.question}
        choices={message.choices}
        multiSelect={message.multi_select === true}
        answer={message.answer}
        canAnswer={canAnswer}
        isMobile={isMobile}
        showTerminalFallback={
          sessionEngine === 'Codex' &&
          actions !== null &&
          !actions.answerQuestion
        }
        canOpenInTerminal={actions?.openInTerminal === true}
        openInTerminalStatus={openInTerminalStatus}
        onAnswer={(choice) => onAction('answer', { choice })}
        onSkip={() => onAction('answer', { skip: true })}
        onFreeformAnswer={(answer) => {
          const questionContext =
            message.header !== undefined && message.header.length > 0
              ? `${message.header}: ${message.question}`
              : message.question
          return onAction('messages', {
            content: `My answer to "${questionContext}" is:\n${answer}`,
          })
        }}
        onOpenInTerminal={onOpenInTerminal}
      />
    )
  }
  if (message.type === 'decision') {
    return (
      <DecisionCard
        key={`decision-${String(message.seq)}-${String(index)}`}
        decision={message}
        variant="conversation"
        selected={selected}
        {...(decisionPosition === undefined
          ? {}
          : { position: decisionPosition })}
        onSelect={() => {
          onSelectDecision(message.seq)
        }}
        onOk={() => onAction(`decisions/${String(message.seq)}/ok`)}
        onRequestChange={(comment) =>
          onAction(`decisions/${String(message.seq)}/change`, { comment })
        }
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
  if (message.type === 'todo') {
    return (
      <TodoCard
        key={`todo-${String(message.seq)}`}
        title={message.title}
        detail={message.detail}
        completed={message.completed}
        defaultComment={message.comment ?? ''}
        onComplete={(comment) =>
          onAction(`todos/${String(message.seq)}/complete`, { comment })
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
      openInTerminalStatus={openInTerminalStatus}
      isCodex={sessionEngine === 'Codex'}
      onAction={(action, permissionIdentity) => {
        void onAction(
          action,
          { requestId: permissionIdentity.requestId },
          permissionIdentity,
        )
      }}
      onOpenInTerminal={onOpenInTerminal}
    />
  )
}
