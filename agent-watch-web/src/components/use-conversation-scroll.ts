import { useCallback, useLayoutEffect, useRef } from 'react'

import type { ChatItem } from '#types'

type UseConversationScrollOptions = {
  messages: readonly ChatItem[]
  sessionId: string
  start: number
}

export function useConversationScroll({
  messages,
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
    }
  }, [messages, start])

  const onScroll = useCallback(() => {
    const scrollContainer = scrollRef.current
    if (scrollContainer === null) return
    followLatestRef.current =
      scrollContainer.scrollHeight -
        scrollContainer.scrollTop -
        scrollContainer.clientHeight <=
      24
  }, [])

  const preserveScrollAnchor = useCallback(() => {
    const scrollContainer = scrollRef.current
    if (scrollContainer === null) return
    pendingScrollAnchorRef.current = {
      scrollHeight: scrollContainer.scrollHeight,
      scrollTop: scrollContainer.scrollTop,
      start,
    }
  }, [start])

  return { onScroll, preserveScrollAnchor, scrollRef }
}
