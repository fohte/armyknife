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
import { PermissionDeniedCard } from '#components/permission-denied-card'
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
  now?: Date
}

export function renderConversationMessage({
  actions,
  actionsReady,
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
  now,
}: ConversationMessageRowProps): ReactNode {
  if (
    message.type === 'user' ||
    message.type === 'queued_user' ||
    message.type === 'user_with_content'
  ) {
    return (
      <UserWithContent
        message={message}
        onSelectSession={onSelectSession}
        sessions={sessions}
        {...(now === undefined ? {} : { now })}
      />
    )
  }
  if (message.type === 'peer') {
    return (
      <PeerMessage
        message={message}
        onSelectSession={onSelectSession}
        sessions={sessions}
        {...(now === undefined ? {} : { now })}
      />
    )
  }
  if (message.type === 'subagent_report') {
    return (
      <SubagentReport
        message={message}
        onSelectSession={onSelectSession}
        sessions={sessions}
      />
    )
  }
  if (message.type === 'event') {
    const time = formatMessageTime(message.timestamp, now)
    return (
      <div
        className="chat-event"
        data-failed={message.failed ? 'true' : undefined}
        data-kind={message.kind}
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
      <article className="chat-assistant-message">
        <MarkdownMessage onSelectSession={onSelectSession} sessions={sessions}>
          {message.text}
        </MarkdownMessage>
      </article>
    )
  }
  if (message.type === 'tool_summary') {
    return (
      <ToolSummary
        summary={message.summary}
        operations={message.operations ?? []}
      />
    )
  }
  if (message.type === 'tool_in_progress') {
    return <ToolSummary summary="1 action" operations={[message]} />
  }
  if (message.type === 'question') {
    const unanswered = message.questions.every(
      (question) => question.answer === null,
    )
    const canAnswer =
      message.requestId !== undefined &&
      unanswered &&
      actionsReady &&
      actions?.answerQuestion === true
    return (
      <QuestionCard
        questions={message.questions}
        canAnswer={canAnswer}
        isMobile={isMobile}
        showTerminalFallback={unanswered && actionsReady && !canAnswer}
        canOpenInTerminal={actions?.openInTerminal === true}
        openInTerminalStatus={openInTerminalStatus}
        onAnswer={(answers) =>
          onAction('answer', { requestId: message.requestId, answers })
        }
        onSkip={() =>
          onAction('answer', { requestId: message.requestId, skip: true })
        }
        onOpenInTerminal={onOpenInTerminal}
      />
    )
  }
  if (message.type === 'decision') {
    return (
      <DecisionCard
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
        title={message.title}
        detail={message.detail}
        resolution={message.resolution}
        defaultComment={message.comment ?? ''}
        onResolve={(resolution, comment) =>
          onAction(`todos/${String(message.seq)}/resolve`, {
            resolution,
            comment,
          })
        }
      />
    )
  }
  if (message.type === 'permission_denied') {
    const identity = { requestId: `permission-denied-${String(message.seq)}` }
    return (
      <PermissionDeniedCard
        message={message}
        canRetry={actions?.message === true}
        actionsReady={actionsReady}
        pendingAction={pendingAction}
        onRetry={() => {
          void onAction(
            `permission-denials/${String(message.seq)}/retry`,
            undefined,
            identity,
          )
        }}
      />
    )
  }
  return (
    <PermissionCard
      message={message}
      actions={actions}
      actionsReady={actionsReady}
      pendingAction={pendingAction}
      openInTerminalStatus={openInTerminalStatus}
      isCodex={sessionEngine === 'codex'}
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
