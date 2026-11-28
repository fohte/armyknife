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
  useOwnMessageKeyboardNavigation({
    isMobile,
    scrollRef: conversationScroll.scrollRef,
  })

  return conversationScroll
}
