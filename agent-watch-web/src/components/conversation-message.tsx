import '#components/conversation-message.css'

import type { ReactNode } from 'react'

import type {
  CommandOutputRefresh,
  CommandOutputStart,
} from '#components/command-result-message'
import { DecisionCard } from '#components/decision-card'
import { FileAttachmentCard } from '#components/file-attachment-card'
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
import { summarizeToolGroupItems, ToolSummary } from '#components/tool-summary'
import { UserWithContent } from '#components/user-with-content'
import type {
  ChatItem,
  OpenInTerminalStatus,
  RewindMode,
  Session,
  SessionActions,
  SessionLinkTarget,
} from '#types'

const EMPTY_IMAGE_SOURCES: Readonly<Record<string, string | null>> = {}

export type ConversationMessageRowProps = {
  actions: SessionActions | null
  actionsReady: boolean
  isMobile: boolean
  selected: boolean
  decisionPosition: string | undefined
  onSelectDecision: (seq: number) => void
  message: ChatItem
  imageSources?: Readonly<Record<string, string | null>>
  pendingAction: PendingPermissionActionState
  openInTerminalStatus: OpenInTerminalStatus
  onAction: (
    action: string,
    payload?: Record<string, unknown>,
    permissionIdentity?: PermissionRequestIdentity,
  ) => Promise<boolean>
  onCommandExecuted?: CommandOutputRefresh
  onCommandStarted?: CommandOutputStart
  conversationIndex?: number
  onOpenInTerminal: () => void
  onSelectSession: (id: string) => void
  onRewind?: (
    messageId: string,
    mode: RewindMode,
    draft: string,
    timelineIndex?: number,
  ) => Promise<boolean>
  sessionEngine: Session['engine']
  sessionId?: string
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
  imageSources,
  conversationIndex,
  pendingAction,
  openInTerminalStatus,
  onAction,
  onCommandExecuted,
  onCommandStarted,
  onOpenInTerminal,
  onSelectSession,
  onRewind = () => Promise.resolve(false),
  sessionEngine,
  sessionId,
  sessions,
  now,
}: ConversationMessageRowProps): ReactNode {
  if (message.type === 'file_attachment') {
    return (
      <FileAttachmentCard
        message={message}
        {...(now === undefined ? {} : { now })}
      />
    )
  }
  if (
    message.type === 'user' ||
    message.type === 'queued_user' ||
    message.type === 'user_with_content'
  ) {
    return (
      <UserWithContent
        message={message}
        canRewind={actions?.rewind === true}
        sessionEngine={sessionEngine}
        onRewind={onRewind}
        {...(conversationIndex === undefined ? {} : { conversationIndex })}
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
    return <SubagentReport message={message} />
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
            <span aria-hidden="true" className="chat-event-separator">
              ·
            </span>
            <time dateTime={message.timestamp}>{time}</time>
          </>
        )}
      </div>
    )
  }
  if (message.type === 'assistant' || message.type === 'narration') {
    const effectiveImageSources =
      message.type === 'assistant'
        ? (imageSources ?? EMPTY_IMAGE_SOURCES)
        : imageSources
    const className =
      message.type === 'narration'
        ? 'chat-assistant-message chat-narration-message'
        : 'chat-assistant-message'
    return (
      <article className={className}>
        <MarkdownMessage
          {...(effectiveImageSources === undefined
            ? {}
            : { imageSources: effectiveImageSources })}
          {...(sessionId === undefined ? {} : { sessionId })}
          {...(onCommandExecuted === undefined ? {} : { onCommandExecuted })}
          {...(onCommandStarted === undefined ? {} : { onCommandStarted })}
          mobile={isMobile}
        >
          {message.text}
        </MarkdownMessage>
      </article>
    )
  }
  if (message.type === 'thinking') {
    if (isMobile && message.text.trim() === '') return null
    const items = [{ type: 'thinking' as const, text: message.text }]
    return (
      <ToolSummary
        summary={summarizeToolGroupItems(items)}
        operations={[]}
        items={items}
      />
    )
  }
  if (message.type === 'tool_summary') {
    return (
      <ToolSummary
        summary={message.summary}
        operations={message.operations ?? []}
        {...(message.items === undefined ? {} : { items: message.items })}
      />
    )
  }
  if (message.type === 'tool_in_progress') {
    const items = message.items
    return (
      <ToolSummary
        summary={
          items === undefined ? '1 action' : summarizeToolGroupItems(items)
        }
        operations={[message]}
        {...(items === undefined ? {} : { items })}
      />
    )
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
        {...(sessionId === undefined ? {} : { sessionId })}
        {...(message.requestId === undefined
          ? {}
          : { requestId: message.requestId })}
        questions={message.questions}
        cancelled={message.cancelled === true}
        canAnswer={canAnswer}
        isMobile={isMobile}
        showTerminalFallback={
          message.cancelled !== true && unanswered && actionsReady && !canAnswer
        }
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
        {...(sessionId === undefined ? {} : { sessionId })}
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
        {...(sessionId === undefined ? {} : { sessionId })}
        seq={message.seq}
        title={message.title}
        summary={message.summary}
        options={message.options}
        answer={message.answer}
        resolved={message.resolved}
        resolution={message.resolution}
        withdrawalComment={message.comment}
        isMobile={isMobile}
        onAnswer={(answer, attachments) =>
          onAction(`proposals/${String(message.seq)}/answer`, {
            choice: answer.choice,
            comment: answer.comment,
            expectedBody: {
              title: message.title,
              summary: message.summary,
              options: message.options,
            },
            ...(attachments === undefined ? {} : { attachments }),
          })
        }
      />
    )
  }
  if (message.type === 'todo') {
    return (
      <TodoCard
        seq={message.seq}
        {...(sessionId === undefined ? {} : { sessionId })}
        title={message.title}
        detail={message.detail}
        folded={message.folded ?? false}
        addedFolded={message.addedFolded ?? false}
        resolution={message.resolution}
        defaultComment={message.comment ?? ''}
        onResolve={(resolution, comment, attachments) =>
          onAction(`todos/${String(message.seq)}/resolve`, {
            resolution,
            comment,
            expectedTitle: message.title,
            expectedDetail: message.detail,
            ...(attachments === undefined ? {} : { attachments }),
          })
        }
        onFold={(folded) =>
          onAction(`todos/${String(message.seq)}/fold`, { folded })
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
        canDismiss={actionsReady}
        onRetry={() => {
          void onAction(
            `permission-denials/${String(message.seq)}/retry`,
            undefined,
            identity,
          )
        }}
        onDismiss={() => {
          void onAction(
            `permission-denials/${String(message.seq)}/dismiss`,
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
