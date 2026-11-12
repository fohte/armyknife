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

function getConversationMessageIdentity(message: ChatItem): string {
  switch (message.type) {
    case 'proposal':
    case 'todo':
    case 'decision':
      return `${message.type}:${String(message.seq)}`
    case 'question':
    case 'permission':
      return message.requestId === undefined
        ? `${message.type}:${JSON.stringify(message)}`
        : `${message.type}:${message.requestId}`
    default:
      return `${message.type}:${JSON.stringify(message)}`
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
