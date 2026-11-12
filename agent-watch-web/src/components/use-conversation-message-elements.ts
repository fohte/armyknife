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

function fingerprint(parts: readonly string[]): string {
  let firstHash = 2166136261
  let secondHash = 0x9e3779b9
  for (const part of parts) {
    const sampleLength = Math.min(128, part.length)
    for (let index = 0; index < sampleLength; index += 1) {
      const code = part.charCodeAt(index)
      firstHash = Math.imul(firstHash ^ code, 16777619)
      secondHash = Math.imul(secondHash ^ code, 2246822519)
    }
    const suffixStart = Math.max(sampleLength, part.length - 128)
    for (let index = suffixStart; index < part.length; index += 1) {
      const code = part.charCodeAt(index)
      firstHash = Math.imul(firstHash ^ code, 16777619)
      secondHash = Math.imul(secondHash ^ code, 2246822519)
    }
    firstHash = Math.imul(firstHash ^ part.length, 16777619)
    secondHash = Math.imul(secondHash ^ part.length, 2246822519)
    firstHash = Math.imul(firstHash ^ 255, 16777619)
    secondHash = Math.imul(secondHash ^ 255, 2246822519)
  }
  return `${(firstHash >>> 0).toString(36)}.${(secondHash >>> 0).toString(36)}`
}

function getConversationMessageIdentity(message: ChatItem): string {
  switch (message.type) {
    case 'proposal':
    case 'todo':
    case 'decision':
      return `${message.type}:${String(message.seq)}`
    case 'question':
      return message.requestId === undefined
        ? `${message.type}:${fingerprint(
            message.questions.flatMap((question) => [
              question.header ?? '',
              question.question,
              ...question.choices.map((choice) => choice.label),
            ]),
          )}`
        : `${message.type}:${message.requestId}`
    case 'permission':
      return message.requestId === undefined
        ? `${message.type}:${fingerprint([message.tool, message.description])}`
        : `${message.type}:${message.requestId}`
    case 'tool_summary': {
      const firstOperation = message.operations?.[0]
      return `${message.type}:${fingerprint(
        firstOperation === undefined
          ? [message.summary]
          : [firstOperation.tool, firstOperation.target ?? ''],
      )}`
    }
    case 'tool_in_progress':
      return `${message.type}:${fingerprint([message.tool])}`
    case 'user':
      return message.timestamp === undefined
        ? `${message.type}:${fingerprint([message.from ?? '', message.text])}`
        : `${message.type}:${message.from ?? ''}:${message.timestamp}`
    case 'queued_user':
    case 'user_with_content':
      return message.timestamp === undefined
        ? `${message.type}:${fingerprint([
            message.from ?? '',
            ...contentIdentity(message.content),
          ])}`
        : `${message.type}:${message.from ?? ''}:${message.timestamp}`
    case 'peer':
      return message.timestamp === undefined
        ? `${message.type}:${fingerprint([
            message.direction,
            message.session_id,
            ...contentIdentity(message.content),
          ])}`
        : `${message.type}:${message.direction}:${message.session_id}:${message.timestamp}`
    case 'subagent_report':
      return `${message.type}:${fingerprint([
        message.agent_id,
        message.description ?? '',
      ])}`
    case 'event':
      return message.timestamp === undefined
        ? `${message.type}:${fingerprint([
            message.kind,
            message.text,
            String(message.failed),
          ])}`
        : `${message.type}:${message.kind}:${message.timestamp}`
    case 'assistant':
      return `${message.type}:${fingerprint([message.text])}`
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
