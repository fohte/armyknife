import { useCallback, useLayoutEffect, useRef } from 'react'

import type { ChatItem } from '#types'

type UseConversationScrollOptions = {
  messages: readonly ChatItem[]
  loadingOlder: boolean
  onLoadOlder: () => void
  onPruneOldMessages: () => void
  sessionId: string
  start: number
}

export function useConversationScroll({
  messages,
  loadingOlder,
  onLoadOlder,
  onPruneOldMessages,
  sessionId,
  start,
}: UseConversationScrollOptions) {
  const scrollRef = useRef<HTMLDivElement>(null)
  const pendingScrollAnchorRef = useRef<{
    scrollHeight: number
    scrollTop: number
    start: number
  } | null>(null)
  const followLatestRef = useRef(true)

  useLayoutEffect(() => {
    pendingScrollAnchorRef.current = null
    followLatestRef.current = true
  }, [sessionId])

  useLayoutEffect(() => {
    const scrollContainer = scrollRef.current
    if (scrollContainer === null) return
    const anchor = pendingScrollAnchorRef.current
    if (anchor !== null && start < anchor.start) {
      scrollContainer.scrollTop =
        anchor.scrollTop + scrollContainer.scrollHeight - anchor.scrollHeight
      pendingScrollAnchorRef.current = null
    } else if (followLatestRef.current) {
      scrollContainer.scrollTop =
        scrollContainer.scrollHeight - scrollContainer.clientHeight
      onPruneOldMessages()
    }
  }, [messages, onPruneOldMessages, start])

  const loadOlder = useCallback(() => {
    if (start <= 0 || loadingOlder) return
    const scrollContainer = scrollRef.current
    if (scrollContainer !== null) {
      pendingScrollAnchorRef.current = {
        scrollHeight: scrollContainer.scrollHeight,
        scrollTop: scrollContainer.scrollTop,
        start,
      }
    }
    onLoadOlder()
  }, [loadingOlder, onLoadOlder, start])

  const onScroll = useCallback(() => {
    const scrollContainer = scrollRef.current
    if (scrollContainer === null) return
    const pendingAnchor = pendingScrollAnchorRef.current
    if (pendingAnchor !== null) {
      pendingScrollAnchorRef.current = {
        scrollHeight: scrollContainer.scrollHeight,
        scrollTop: scrollContainer.scrollTop,
        start: pendingAnchor.start,
      }
    }
    followLatestRef.current =
      scrollContainer.scrollHeight -
        scrollContainer.scrollTop -
        scrollContainer.clientHeight <=
      24
    if (followLatestRef.current) onPruneOldMessages()
    if (scrollContainer.scrollTop <= 120) loadOlder()
  }, [loadOlder, onPruneOldMessages])

  return { loadOlder, onScroll, scrollRef }
}
