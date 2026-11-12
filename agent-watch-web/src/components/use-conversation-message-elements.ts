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
  start?: number
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

export function useConversationMessageElements({
  messages,
  start = 0,
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
    const conversationMessages = messages
      .map((message, index) => ({ message, index }))
      .filter(
        ({ message }) =>
          message.type !== 'tool_in_progress' ||
          (sessionCategory !== 'in_progress' && !isCompacting),
      )
    const decisions =
      allDecisions ??
      conversationMessages
        .map(({ message }) => message)
        .filter(
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
    const elements = conversationMessages.map(({ message, index }) => {
      const messagePendingAction: PendingPermissionActionState =
        message.type === 'permission' && message.requestId !== undefined
          ? (pendingActions[message.requestId] ?? null)
          : null

      return createElement(ConversationMessageRow, {
        key:
          message.type === 'proposal'
            ? `proposal-${String(message.seq)}`
            : String(start + index),
        actions,
        actionsReady,
        index: start + index,
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
    start,
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
