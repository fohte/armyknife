import { createElement, useMemo } from 'react'

import { ConversationMessageRow } from '#components/conversation-message'
import type { PendingPermissionAction } from '#components/permission-card'
import type {
  ChatItem,
  Session,
  SessionActions,
  SessionLinkTarget,
} from '#types'

type UseConversationMessageElementsOptions = {
  messages: readonly ChatItem[]
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
  onSelectSession: (id: string) => void
  pendingAction: PendingPermissionAction
  sessions: readonly SessionLinkTarget[]
}

export function useConversationMessageElements({
  messages,
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
  onSelectSession,
  pendingAction,
  sessions,
}: UseConversationMessageElementsOptions) {
  return useMemo(() => {
    const activeQuestionIndex = messages.findLastIndex(
      (message) => message.type === 'question',
    )
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
    const decisions = conversationMessages
      .map(({ message }) => message)
      .filter(
        (message): message is Extract<ChatItem, { type: 'decision' }> =>
          message.type === 'decision',
      )
    const decisionPositions = new Map(
      decisions.map((decision, index) => [decision.seq, index + 1]),
    )
    const hasUnansweredProposal = messages.some(
      (message) =>
        message.type === 'proposal' &&
        message.answer === null &&
        !message.resolved,
    )
    const elements = conversationMessages.map(({ message, index }) => {
      const messagePendingAction =
        message.type === 'permission' &&
        message.tool === pendingAction?.tool &&
        message.description === pendingAction.description
          ? pendingAction
          : null

      return createElement(ConversationMessageRow, {
        key: String(index),
        actions,
        actionsReady,
        activeQuestion:
          message.type === 'question' && index === activeQuestionIndex,
        index,
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
    isCompacting,
    isMobile,
    messages,
    onAction,
    onOpenInTerminal,
    onSelectDecision,
    onSelectSession,
    pendingAction,
    selectedDecisionSeq,
    sessionCategory,
    sessionEngine,
    sessions,
  ])
}
