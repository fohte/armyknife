import { createElement, useMemo } from 'react'

import type {
  CommandOutputRefresh,
  CommandOutputStart,
} from '#components/command-result-message'
import { ConversationMessageRow } from '#components/conversation-message-row'
import type {
  PendingPermissionActions,
  PendingPermissionActionState,
} from '#components/permission-card'
import type {
  ChatItem,
  OpenInTerminalStatus,
  RewindMode,
  Session,
  SessionActions,
  SessionLinkTarget,
  UserContentPart,
} from '#types'

type UseConversationMessageElementsOptions = {
  messages: readonly ChatItem[]
  start: number
  decisions?: readonly Extract<ChatItem, { type: 'decision' }>[]
  hasUnansweredProposal?: boolean
  sessionCategory: Session['category']
  sessionId?: string
  isCompacting: boolean
  sessionEngine: Session['engine']
  actions: SessionActions | null
  actionsReady: boolean
  isMobile: boolean
  selectedDecisionSeq: number | null
  onSelectDecision: (seq: number) => void
  onAction: (
    action: string,
    payload?: Record<string, unknown>,
  ) => Promise<boolean>
  onCommandExecuted?: CommandOutputRefresh
  onCommandStarted?: CommandOutputStart
  onOpenInTerminal: () => void
  openInTerminalStatus: OpenInTerminalStatus
  onSelectSession: (id: string) => void
  onRewind?: (
    messageId: string,
    mode: RewindMode,
    draft: string,
  ) => Promise<boolean>
  pendingActions: PendingPermissionActions
  sessions: readonly SessionLinkTarget[]
  now: Date
}

function contentIdentity(content: readonly UserContentPart[]): string[] {
  return content.flatMap((part) => {
    switch (part.type) {
      case 'text':
        return ['text', part.text]
      case 'image':
        return ['image', part.source, String(part.available)]
      case 'file':
        return ['file', part.name]
    }
  })
}

function isPendingProposal(
  message: Extract<ChatItem, { type: 'proposal' }>,
): boolean {
  return (
    message.answer === null && !message.resolved && message.resolution === null
  )
}

function getConversationMessageIdentity(message: ChatItem): string {
  switch (message.type) {
    case 'proposal':
    case 'todo':
    case 'decision':
      return `${message.type}:${String(message.seq)}`
    case 'question':
      return message.requestId === undefined
        ? `${message.type}:${JSON.stringify(
            message.questions.map((question) => [
              question.header ?? '',
              question.question,
              String(question.multi_select),
              ...question.choices.flatMap((choice) => [
                choice.label,
                choice.description ?? '',
              ]),
            ]),
          )}`
        : `${message.type}:${message.requestId}`
    case 'permission':
      return message.requestId === undefined
        ? `${message.type}:${JSON.stringify([
            message.tool,
            message.description,
            message.items ?? [],
          ])}`
        : `${message.type}:${message.requestId}`
    case 'permission_denied':
      return `${message.type}:${String(message.seq)}`
    case 'tool_summary': {
      return `${message.type}:${JSON.stringify([
        message.summary,
        message.operations ?? [],
        message.items ?? [],
      ])}`
    }
    case 'tool_in_progress':
      return `${message.type}:${JSON.stringify([
        message.tool,
        message.target ?? '',
        message.items ?? [],
      ])}`
    case 'user':
      return message.timestamp === undefined
        ? `${message.type}:${JSON.stringify([message.from ?? '', message.text])}`
        : `${message.type}:${JSON.stringify([message.from ?? '', message.timestamp])}`
    case 'queued_user':
    case 'user_with_content':
      return message.timestamp === undefined
        ? `${message.type}:${JSON.stringify([
            message.from ?? '',
            ...contentIdentity(message.content),
          ])}`
        : `${message.type}:${JSON.stringify([message.from ?? '', message.timestamp])}`
    case 'peer':
      return message.timestamp === undefined
        ? `${message.type}:${JSON.stringify([
            message.direction,
            message.session_id,
            ...contentIdentity(message.content),
          ])}`
        : `${message.type}:${JSON.stringify([message.direction, message.session_id, message.timestamp])}`
    case 'subagent_report':
      return `${message.type}:${JSON.stringify([
        message.agent_id,
        message.description ?? '',
      ])}`
    case 'event':
      return message.timestamp === undefined
        ? `${message.type}:${JSON.stringify([
            message.kind,
            message.text,
            String(message.failed),
          ])}`
        : `${message.type}:${JSON.stringify([message.kind, message.timestamp])}`
    case 'assistant':
    case 'narration':
    case 'thinking':
      return `${message.type}:${JSON.stringify([message.text])}`
    case 'file_attachment':
      return `${message.type}:${JSON.stringify([
        message.caption ?? '',
        message.timestamp ?? '',
        ...message.files.flatMap((file) => [file.source, file.name]),
      ])}`
  }
}

export function useConversationMessageElements({
  messages,
  start,
  decisions: allDecisions,
  hasUnansweredProposal: proposalPending,
  sessionCategory,
  sessionId,
  isCompacting,
  sessionEngine,
  actions,
  actionsReady,
  isMobile,
  selectedDecisionSeq,
  onSelectDecision,
  onAction,
  onCommandExecuted,
  onCommandStarted,
  onOpenInTerminal,
  openInTerminalStatus,
  onSelectSession,
  onRewind,
  pendingActions,
  sessions,
  now,
}: UseConversationMessageElementsOptions) {
  return useMemo(() => {
    const activeTool = messages.findLast(
      (message) => message.type === 'tool_in_progress',
    )
    const conversationMessages = messages.flatMap((message, index) =>
      message.type !== 'tool_in_progress' ||
      (sessionCategory !== 'in_progress' && !isCompacting)
        ? [{ message, index }]
        : [],
    )
    const decisions =
      allDecisions ??
      conversationMessages.flatMap(({ message }) =>
        message.type === 'decision' ? [message] : [],
      )
    const decisionPositions = new Map(
      decisions.map((decision, index) => [decision.seq, index + 1]),
    )
    const hasUnansweredProposal =
      proposalPending ??
      messages.some(
        (message) => message.type === 'proposal' && isPendingProposal(message),
      )
    const keyOccurrences = new Map<string, number>()
    const rows = conversationMessages.map(({ message, index }) => {
      const messagePendingAction: PendingPermissionActionState =
        message.type === 'permission' && message.requestId !== undefined
          ? (pendingActions[message.requestId] ?? null)
          : message.type === 'permission_denied'
            ? (pendingActions[`permission-denied-${String(message.seq)}`] ??
              null)
            : null
      const identity = getConversationMessageIdentity(message)
      const occurrence = keyOccurrences.get(identity) ?? 0
      keyOccurrences.set(identity, occurrence + 1)

      return {
        message,
        element: createElement(ConversationMessageRow, {
          key: `${identity}:${String(occurrence)}`,
          actions,
          actionsReady,
          isMobile,
          selected:
            message.type === 'decision' && message.seq === selectedDecisionSeq,
          decisionPosition:
            message.type === 'decision'
              ? `${String(decisionPositions.get(message.seq))} of ${String(decisions.length)}`
              : undefined,
          onSelectDecision,
          message,
          onAction,
          ...(onCommandExecuted === undefined ? {} : { onCommandExecuted }),
          ...(onCommandStarted === undefined ? {} : { onCommandStarted }),
          conversationIndex: start + index,
          onOpenInTerminal,
          onSelectSession,
          ...(onRewind === undefined ? {} : { onRewind }),
          pendingAction: messagePendingAction,
          openInTerminalStatus,
          sessionEngine,
          ...(sessionId === undefined ? {} : { sessionId }),
          sessions,
          now,
        }),
      }
    })

    return {
      activeTool,
      conversationMessageCount: conversationMessages.length,
      elements: rows.map(({ element }) => element),
      hasUnansweredProposal,
    }
  }, [
    actions,
    actionsReady,
    allDecisions,
    isCompacting,
    isMobile,
    messages,
    start,
    now,
    onAction,
    onCommandExecuted,
    onCommandStarted,
    onOpenInTerminal,
    onSelectDecision,
    onSelectSession,
    onRewind,
    openInTerminalStatus,
    pendingActions,
    proposalPending,
    selectedDecisionSeq,
    sessionCategory,
    sessionEngine,
    sessionId,
    sessions,
  ])
}
