import { useCallback, useLayoutEffect, useRef } from 'react'

import type { ChatItem } from '#types'

type UseConversationScrollOptions = {
  messages: readonly ChatItem[]
  loadingOlder: boolean
  onLoadOlder: () => void
  onPruneOldMessages: () => void
  sessionId: string
  start: number
  epoch?: string | undefined
  olderCursor?: string | undefined
}

function isAtBottom(scrollContainer: HTMLDivElement) {
  return (
    scrollContainer.scrollHeight -
      scrollContainer.scrollTop -
      scrollContainer.clientHeight <=
    24
  )
}

export function useConversationScroll({
  messages,
  loadingOlder,
  onLoadOlder,
  onPruneOldMessages,
  sessionId,
  start,
  epoch,
  olderCursor,
}: UseConversationScrollOptions) {
  const scrollRef = useRef<HTMLDivElement>(null)
  const pendingScrollAnchorRef = useRef<{
    scrollHeight: number
    scrollTop: number
    start: number
    epoch: string | undefined
  } | null>(null)
  const followLatestRef = useRef(true)
  const programmaticScrollTopRef = useRef<number | null>(null)
  const suppressResizeRef = useRef(false)
  const previousMessagesRef = useRef<{
    messages: readonly ChatItem[]
    start: number
    end: number
    epoch: string | undefined
  } | null>(null)
  const userLayoutChangeRef = useRef(false)
  const userLayoutChangeTimeoutRef = useRef<number | null>(null)
  const loadingOlderRef = useRef(loadingOlder)
  const onPruneOldMessagesRef = useRef(onPruneOldMessages)
  loadingOlderRef.current = loadingOlder
  onPruneOldMessagesRef.current = onPruneOldMessages

  const setAnchoredScrollTop = useCallback(
    (scrollContainer: HTMLDivElement, scrollTop: number) => {
      programmaticScrollTopRef.current = scrollTop
      scrollContainer.scrollTop = scrollTop
    },
    [],
  )

  useLayoutEffect(() => {
    pendingScrollAnchorRef.current = null
    followLatestRef.current = true
    previousMessagesRef.current = null
    userLayoutChangeRef.current = false
    if (userLayoutChangeTimeoutRef.current !== null) {
      window.clearTimeout(userLayoutChangeTimeoutRef.current)
      userLayoutChangeTimeoutRef.current = null
    }
    programmaticScrollTopRef.current = null
    suppressResizeRef.current = false
  }, [sessionId])

  useLayoutEffect(() => {
    const scrollContainer = scrollRef.current
    if (scrollContainer === null) return
    const previousMessages = previousMessagesRef.current
    const end = start + messages.length
    const appendedMessages =
      previousMessages !== null &&
      end > previousMessages.end &&
      start >= previousMessages.start &&
      epoch === previousMessages.epoch
    const messageItemsChanged =
      previousMessages === null ||
      messages.length !== previousMessages.messages.length ||
      messages.some(
        (message, index) => message !== previousMessages.messages[index],
      )
    previousMessagesRef.current = { messages, start, end, epoch }

    const anchor = pendingScrollAnchorRef.current
    if (anchor !== null) {
      if (start < anchor.start || epoch !== anchor.epoch) {
        const scrollTop =
          anchor.scrollTop + scrollContainer.scrollHeight - anchor.scrollHeight
        setAnchoredScrollTop(scrollContainer, scrollTop)
        suppressResizeRef.current = true
        pendingScrollAnchorRef.current = null
      } else if (!loadingOlder) {
        pendingScrollAnchorRef.current = null
      }
      if (pendingScrollAnchorRef.current === null) {
        userLayoutChangeRef.current = false
        if (userLayoutChangeTimeoutRef.current !== null) {
          window.clearTimeout(userLayoutChangeTimeoutRef.current)
          userLayoutChangeTimeoutRef.current = null
        }
      }
      return
    }

    if (userLayoutChangeRef.current) {
      followLatestRef.current = isAtBottom(scrollContainer)
    } else if (previousMessages === null || appendedMessages) {
      if (!followLatestRef.current) return
      if (!loadingOlder) {
        const scrollTop =
          scrollContainer.scrollHeight - scrollContainer.clientHeight
        scrollContainer.scrollTop = scrollTop
        onPruneOldMessages()
      }
      return
    } else if (
      messageItemsChanged ||
      start !== previousMessages.start ||
      epoch !== previousMessages.epoch
    ) {
      followLatestRef.current = isAtBottom(scrollContainer)
      if (followLatestRef.current && !loadingOlder) {
        onPruneOldMessages()
      }
    }
  }, [
    epoch,
    loadingOlder,
    messages,
    onPruneOldMessages,
    setAnchoredScrollTop,
    start,
  ])

  useLayoutEffect(() => {
    const scrollContainer = scrollRef.current
    if (
      scrollContainer === null ||
      typeof ResizeObserver === 'undefined' ||
      typeof MutationObserver === 'undefined'
    ) {
      return
    }

    const initialResizeTargets = new WeakSet<Element>()
    const resizeObserver = new ResizeObserver((entries) => {
      let resizedExistingChild = false
      for (const entry of entries) {
        if (initialResizeTargets.has(entry.target)) {
          initialResizeTargets.delete(entry.target)
        } else {
          resizedExistingChild = true
        }
      }
      if (suppressResizeRef.current) {
        suppressResizeRef.current = false
        return
      }
      if (loadingOlderRef.current || !resizedExistingChild) return
      const container = scrollRef.current
      if (container === null) return
      if (userLayoutChangeRef.current) {
        userLayoutChangeRef.current = false
        if (userLayoutChangeTimeoutRef.current !== null) {
          window.clearTimeout(userLayoutChangeTimeoutRef.current)
          userLayoutChangeTimeoutRef.current = null
        }
        followLatestRef.current = isAtBottom(container)
        return
      }
      if (!followLatestRef.current) return
      const scrollTop = container.scrollHeight - container.clientHeight
      container.scrollTop = scrollTop
      onPruneOldMessagesRef.current()
    })
    const observeChildren = (nodes: Iterable<Node>) => {
      for (const node of nodes) {
        if (!(node instanceof Element)) continue
        initialResizeTargets.add(node)
        resizeObserver.observe(node)
      }
    }
    observeChildren(scrollContainer.children)

    const mutationObserver = new MutationObserver((records) => {
      for (const record of records) observeChildren(record.addedNodes)
    })
    mutationObserver.observe(scrollContainer, { childList: true })

    const markUserLayoutChange = () => {
      userLayoutChangeRef.current = true
      if (userLayoutChangeTimeoutRef.current !== null) {
        window.clearTimeout(userLayoutChangeTimeoutRef.current)
      }
      userLayoutChangeTimeoutRef.current = window.setTimeout(() => {
        userLayoutChangeRef.current = false
        userLayoutChangeTimeoutRef.current = null
      }, 1000)
    }
    scrollContainer.addEventListener('click', markUserLayoutChange, true)
    scrollContainer.addEventListener('input', markUserLayoutChange, true)
    scrollContainer.addEventListener('change', markUserLayoutChange, true)
    scrollContainer.addEventListener('paste', markUserLayoutChange, true)
    scrollContainer.addEventListener('drop', markUserLayoutChange, true)

    return () => {
      resizeObserver.disconnect()
      mutationObserver.disconnect()
      scrollContainer.removeEventListener('click', markUserLayoutChange, true)
      scrollContainer.removeEventListener('input', markUserLayoutChange, true)
      scrollContainer.removeEventListener('change', markUserLayoutChange, true)
      scrollContainer.removeEventListener('paste', markUserLayoutChange, true)
      scrollContainer.removeEventListener('drop', markUserLayoutChange, true)
      if (userLayoutChangeTimeoutRef.current !== null) {
        window.clearTimeout(userLayoutChangeTimeoutRef.current)
        userLayoutChangeTimeoutRef.current = null
      }
    }
  }, [])

  const loadOlder = useCallback(() => {
    const hasEpochCursor = epoch !== undefined && olderCursor != null
    if ((start <= 0 && !hasEpochCursor) || loadingOlder) return
    const scrollContainer = scrollRef.current
    if (scrollContainer !== null) {
      pendingScrollAnchorRef.current = {
        scrollHeight: scrollContainer.scrollHeight,
        scrollTop: scrollContainer.scrollTop,
        start,
        epoch,
      }
    }
    onLoadOlder()
  }, [epoch, loadingOlder, onLoadOlder, olderCursor, start])

  const onScroll = useCallback(() => {
    const scrollContainer = scrollRef.current
    if (scrollContainer === null) return
    if (programmaticScrollTopRef.current === scrollContainer.scrollTop) {
      programmaticScrollTopRef.current = null
      return
    }
    programmaticScrollTopRef.current = null
    const pendingAnchor = pendingScrollAnchorRef.current
    if (pendingAnchor !== null) {
      pendingScrollAnchorRef.current = {
        scrollHeight: scrollContainer.scrollHeight,
        scrollTop: scrollContainer.scrollTop,
        start: pendingAnchor.start,
        epoch: pendingAnchor.epoch,
      }
    }
    followLatestRef.current = isAtBottom(scrollContainer)
    if (followLatestRef.current) onPruneOldMessages()
    if (scrollContainer.scrollTop <= 120) loadOlder()
  }, [loadOlder, onPruneOldMessages])

  return { loadOlder, onScroll, scrollRef }
}
