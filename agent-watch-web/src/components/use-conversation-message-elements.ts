import { createElement, useMemo } from 'react'

import { ConversationMessageRow } from '#components/conversation-message-row'
import type {
  PendingPermissionActions,
  PendingPermissionActionState,
} from '#components/permission-card'
import type {
  ChatItem,
  OpenInTerminalStatus,
  Session,
  SessionActions,
  SessionLinkTarget,
  UserContentPart,
} from '#types'

type UseConversationMessageElementsOptions = {
  messages: readonly ChatItem[]
  decisions?: readonly Extract<ChatItem, { type: 'decision' }>[]
  hasUnansweredProposal?: boolean
  sessionCategory: Session['category']
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
  onOpenInTerminal: () => void
  openInTerminalStatus: OpenInTerminalStatus
  onSelectSession: (id: string) => void
  pendingActions: PendingPermissionActions
  sessions: readonly SessionLinkTarget[]
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
        ? `${message.type}:${JSON.stringify([message.tool, message.description])}`
        : `${message.type}:${message.requestId}`
    case 'tool_summary': {
      return `${message.type}:${JSON.stringify([
        message.summary,
        message.operations ?? [],
      ])}`
    }
    case 'tool_in_progress':
      return `${message.type}:${JSON.stringify([message.tool, message.target ?? ''])}`
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
      return `${message.type}:${JSON.stringify([message.text])}`
  }
}

export function useConversationMessageElements({
  messages,
  decisions: allDecisions,
  hasUnansweredProposal: proposalPending,
  sessionCategory,
  isCompacting,
  sessionEngine,
  actions,
  actionsReady,
  isMobile,
  selectedDecisionSeq,
  onSelectDecision,
  onAction,
  onOpenInTerminal,
  openInTerminalStatus,
  onSelectSession,
  pendingActions,
  sessions,
}: UseConversationMessageElementsOptions) {
  return useMemo(() => {
    const activeTool = messages.findLast(
      (message) => message.type === 'tool_in_progress',
    )
    const conversationMessages = messages.filter(
      (message) =>
        message.type !== 'tool_in_progress' ||
        (sessionCategory !== 'in_progress' && !isCompacting),
    )
    const decisions =
      allDecisions ??
      conversationMessages.filter(
        (message): message is Extract<ChatItem, { type: 'decision' }> =>
          message.type === 'decision',
      )
    const decisionPositions = new Map(
      decisions.map((decision, index) => [decision.seq, index + 1]),
    )
    const hasUnansweredProposal =
      proposalPending ??
      messages.some(
        (message) =>
          message.type === 'proposal' &&
          message.answer === null &&
          !message.resolved,
      )
    const keyOccurrences = new Map<string, number>()
    const elements = conversationMessages.map((message) => {
      const messagePendingAction: PendingPermissionActionState =
        message.type === 'permission' && message.requestId !== undefined
          ? (pendingActions[message.requestId] ?? null)
          : null
      const identity = getConversationMessageIdentity(message)
      const occurrence = keyOccurrences.get(identity) ?? 0
      keyOccurrences.set(identity, occurrence + 1)

      return createElement(ConversationMessageRow, {
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
        onOpenInTerminal,
        onSelectSession,
        pendingAction: messagePendingAction,
        openInTerminalStatus,
        sessionEngine,
        sessions,
      })
    })

    return {
      activeTool,
      conversationMessageCount: conversationMessages.length,
      elements,
      hasUnansweredProposal,
    }
  }, [
    actions,
    actionsReady,
    allDecisions,
    isCompacting,
    isMobile,
    messages,
    onAction,
    onOpenInTerminal,
    onSelectDecision,
    onSelectSession,
    openInTerminalStatus,
    pendingActions,
    proposalPending,
    selectedDecisionSeq,
    sessionCategory,
    sessionEngine,
    sessions,
  ])
}
