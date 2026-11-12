import { memo } from 'react'

import {
  type ConversationMessageRowProps,
  renderConversationMessage,
} from '#components/conversation-message'

function areConversationMessagePropsEqual(
  previous: ConversationMessageRowProps,
  next: ConversationMessageRowProps,
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
        previous.openInTerminalStatus === next.openInTerminalStatus &&
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
    case 'todo':
      return previous.onAction === next.onAction
    case 'permission':
      return (
        previous.actions === next.actions &&
        previous.actionsReady === next.actionsReady &&
        previous.pendingAction === next.pendingAction &&
        previous.onAction === next.onAction &&
        previous.onOpenInTerminal === next.onOpenInTerminal &&
        previous.openInTerminalStatus === next.openInTerminalStatus &&
        previous.sessionEngine === next.sessionEngine
      )
    case 'event':
    case 'tool_summary':
    case 'tool_in_progress':
      return true
  }
}

export const ConversationMessageRow = memo(function ConversationMessageRow(
  props: ConversationMessageRowProps,
) {
  return renderConversationMessage(props)
}, areConversationMessagePropsEqual)
