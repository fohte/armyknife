import { useConversationDecisionJump } from '#components/use-conversation-decision-jump'
import { useConversationScroll } from '#components/use-conversation-scroll'
import { useOwnMessageKeyboardNavigation } from '#components/use-own-message-keyboard-navigation'
import type { ChatItem } from '#types'

type UseConversationPaneScrollingOptions = {
  messages: readonly ChatItem[]
  loadingOlder: boolean
  onLoadOlder: () => void
  onPruneOldMessages: () => void
  sessionId: string
  start: number
  epoch: string | undefined
  olderCursor: string | undefined
  decisionJump: { seq: number } | null
  onDecisionJumpComplete: () => void
  onDecisionJumpSucceeded: () => void
  isMobile: boolean
}

export function useConversationPaneScrolling({
  messages,
  loadingOlder,
  onLoadOlder,
  onPruneOldMessages,
  sessionId,
  start,
  epoch,
  olderCursor,
  decisionJump,
  onDecisionJumpComplete,
  onDecisionJumpSucceeded,
  isMobile,
}: UseConversationPaneScrollingOptions) {
  const conversationScroll = useConversationScroll({
    messages,
    loadingOlder,
    onLoadOlder,
    onPruneOldMessages,
    sessionId,
    start,
    epoch,
    olderCursor,
  })
  useConversationDecisionJump({
    decisionJump,
    epoch,
    loadingOlder,
    loadOlder: conversationScroll.loadOlder,
    messages,
    olderCursor,
    onComplete: onDecisionJumpComplete,
    onSucceeded: onDecisionJumpSucceeded,
    start,
  })
  useOwnMessageKeyboardNavigation({
    isMobile,
    scrollRef: conversationScroll.scrollRef,
  })

  return conversationScroll
}
