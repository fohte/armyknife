import '#components/conversation-message.css'

import { memo, type ReactNode } from 'react'

import { DecisionCard } from '#components/decision-card'
import { MarkdownMessage } from '#components/markdown-message'
import { PeerMessage } from '#components/peer-message'
import {
  type PendingPermissionAction,
  PermissionCard,
  type PermissionRequestIdentity,
} from '#components/permission-card'
import { ProposalCard } from '#components/proposal-card'
import { QuestionCard } from '#components/question-card'
import { SubagentReport } from '#components/subagent-report'
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
  activeQuestion: boolean
  index: number
  isMobile: boolean
  selected: boolean
  decisionPosition: string | undefined
  onSelectDecision: (seq: number) => void
  message: ChatItem
  pendingAction: PendingPermissionAction
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
  onAction,
  onOpenInTerminal,
  onSelectSession,
  sessionEngine,
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
  return (
    <PermissionCard
      key={`permission-${String(index)}`}
      message={message}
      actions={actions}
      actionsReady={actionsReady}
      pendingAction={pendingAction}
      isCodex={sessionEngine === 'Codex'}
      onAction={(action, permissionIdentity) => {
        void onAction(action, undefined, permissionIdentity)
      }}
      onOpenInTerminal={onOpenInTerminal}
    />
  )
}

function areConversationMessagePropsEqual(
  previous: RenderConversationMessageOptions,
  next: RenderConversationMessageOptions,
): boolean {
  if (
    previous.message !== next.message ||
    previous.index !== next.index ||
    previous.isMobile !== next.isMobile
  ) {
    return false
  }

  switch (previous.message.type) {
    case 'user':
    case 'queued_user':
    case 'user_with_content':
    case 'peer':
    case 'subagent_report':
    case 'assistant':
      return (
        previous.onSelectSession === next.onSelectSession &&
        previous.sessions === next.sessions
      )
    case 'question':
      return (
        previous.activeQuestion === next.activeQuestion &&
        previous.actions === next.actions &&
        previous.actionsReady === next.actionsReady &&
        previous.onAction === next.onAction &&
        previous.onOpenInTerminal === next.onOpenInTerminal &&
        previous.sessionEngine === next.sessionEngine
      )
    case 'decision':
      return (
        previous.selected === next.selected &&
        previous.decisionPosition === next.decisionPosition &&
        previous.onSelectDecision === next.onSelectDecision &&
        previous.onAction === next.onAction
      )
    case 'proposal':
      return previous.onAction === next.onAction
    case 'permission':
      return (
        previous.actions === next.actions &&
        previous.actionsReady === next.actionsReady &&
        previous.pendingAction === next.pendingAction &&
        previous.onAction === next.onAction &&
        previous.onOpenInTerminal === next.onOpenInTerminal &&
        previous.sessionEngine === next.sessionEngine
      )
    case 'event':
    case 'tool_summary':
    case 'tool_in_progress':
      return true
  }
}

export const ConversationMessageRow = memo(function ConversationMessageRow(
  props: RenderConversationMessageOptions,
) {
  return renderConversationMessage(props)
}, areConversationMessagePropsEqual)
