import { useCallback, useLayoutEffect, useRef } from 'react'

import {
  clearConversationAnchorOffset,
  clearConversationAnchorOffsets,
  type ConversationAnchorOffset,
  type ConversationChildLayout,
  hasConversationChildrenInsertedBeforePendingCards,
  hasConversationResolutionChanged,
  movedConversationChildren,
  newlyResolvedConversationCards,
  preserveConversationChildPosition,
  preserveConversationChildren,
  readConversationChildLayout,
} from '#components/conversation-scroll-anchor'
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
  const userResizeGestureRef = useRef(false)
  const userResizeCandidateRef = useRef<{
    element: HTMLTextAreaElement
    initialHeight: number
  } | null>(null)
  const previousMessagesRef = useRef<{
    messages: readonly ChatItem[]
    start: number
    end: number
    epoch: string | undefined
  } | null>(null)
  const previousChildLayoutRef = useRef<readonly ConversationChildLayout[]>([])
  const previousScrollTopRef = useRef(0)
  const conversationAnchorOffsetsRef = useRef(
    new Map<HTMLElement, ConversationAnchorOffset>(),
  )
  const recentResolutionCardsRef = useRef(new Set<HTMLElement>())
  const anchorExtraPaddingBottomRef = useRef(0)
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
    previousChildLayoutRef.current = []
    previousScrollTopRef.current = 0
    clearConversationAnchorOffsets(conversationAnchorOffsetsRef.current)
    recentResolutionCardsRef.current.clear()
    scrollRef.current?.style.removeProperty('padding-bottom')
    anchorExtraPaddingBottomRef.current = 0
    userLayoutChangeRef.current = false
    if (userLayoutChangeTimeoutRef.current !== null) {
      window.clearTimeout(userLayoutChangeTimeoutRef.current)
      userLayoutChangeTimeoutRef.current = null
    }
    programmaticScrollTopRef.current = null
    suppressResizeRef.current = false
    userResizeGestureRef.current = false
    userResizeCandidateRef.current = null
  }, [sessionId])

  useLayoutEffect(() => {
    const scrollContainer = scrollRef.current
    if (scrollContainer === null) return
    const previousMessages = previousMessagesRef.current
    const previousChildLayout = previousChildLayoutRef.current
    const currentChildLayout = readConversationChildLayout(scrollContainer)
    const movedChildren = movedConversationChildren(
      previousChildLayout,
      currentChildLayout,
    )
    const resolvedCards = newlyResolvedConversationCards(
      previousChildLayout,
      currentChildLayout,
    )
    const currentChildrenByElement = new Map(
      currentChildLayout.map((child) => [child.element, child]),
    )
    const becamePendingCards = new Set(
      previousChildLayout.flatMap((previousChild) => {
        const currentChild = currentChildrenByElement.get(previousChild.element)
        return previousChild.cardResolution === 'resolved' &&
          currentChild?.cardResolution === 'pending'
          ? [previousChild.element]
          : []
      }),
    )
    for (const element of becamePendingCards) {
      recentResolutionCardsRef.current.delete(element)
    }
    for (const element of resolvedCards) {
      recentResolutionCardsRef.current.add(element)
    }
    const recentResolutionCards = new Set(recentResolutionCardsRef.current)
    const conversationResolutionChanged = hasConversationResolutionChanged(
      previousChildLayout,
      currentChildLayout,
    )
    const conversationLayoutChanged =
      movedChildren.size > 0 ||
      conversationResolutionChanged ||
      hasConversationChildrenInsertedBeforePendingCards(
        previousChildLayout,
        currentChildLayout,
      )
    const end = start + messages.length
    const appendedMessages =
      previousMessages !== null &&
      end > previousMessages.end &&
      start >= previousMessages.start &&
      (epoch === previousMessages.epoch ||
        previousMessages.messages.length === 0)
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
      if (conversationLayoutChanged) {
        preserveConversationChildren(
          previousChildLayout,
          readConversationChildLayout(scrollContainer),
          scrollContainer,
          recentResolutionCards,
          conversationAnchorOffsetsRef.current,
        )
        for (const element of movedChildren) {
          recentResolutionCardsRef.current.delete(element)
        }
      }
      previousChildLayoutRef.current =
        readConversationChildLayout(scrollContainer)
      previousScrollTopRef.current = scrollContainer.scrollTop
      return
    }

    if (conversationLayoutChanged) {
      const anchoredScrollTop = preserveConversationChildPosition(
        scrollContainer,
        previousChildLayout,
        currentChildLayout,
        recentResolutionCards,
      )
      const currentMaximum =
        scrollContainer.scrollHeight - scrollContainer.clientHeight
      const wasAtBottom =
        followLatestRef.current &&
        !(conversationResolutionChanged && userLayoutChangeRef.current)
      const desiredScrollTop = wasAtBottom
        ? Math.max(previousScrollTopRef.current, currentMaximum)
        : (anchoredScrollTop ?? previousScrollTopRef.current)
      setAnchoredScrollTop(scrollContainer, desiredScrollTop)
      const scrollTopShortfall = desiredScrollTop - scrollContainer.scrollTop
      if (scrollTopShortfall > 0.5) {
        // The resolved card can leave the tail and reduce the available scroll range.
        const basePaddingBottom =
          Number.parseFloat(
            window.getComputedStyle(scrollContainer).paddingBottom,
          ) - anchorExtraPaddingBottomRef.current
        anchorExtraPaddingBottomRef.current += scrollTopShortfall
        scrollContainer.style.paddingBottom = `${String(basePaddingBottom + anchorExtraPaddingBottomRef.current)}px`
        setAnchoredScrollTop(scrollContainer, desiredScrollTop)
      }
      preserveConversationChildren(
        previousChildLayout,
        readConversationChildLayout(scrollContainer),
        scrollContainer,
        recentResolutionCards,
        conversationAnchorOffsetsRef.current,
      )
      for (const element of movedChildren) {
        recentResolutionCardsRef.current.delete(element)
      }
      previousChildLayoutRef.current =
        readConversationChildLayout(scrollContainer)
      previousScrollTopRef.current = scrollContainer.scrollTop
      followLatestRef.current = isAtBottom(scrollContainer)
      return
    }

    const resizeCandidate = userResizeCandidateRef.current
    if (
      resizeCandidate !== null &&
      Math.abs(
        resizeCandidate.element.getBoundingClientRect().height -
          resizeCandidate.initialHeight,
      ) > 0.5
    ) {
      userResizeGestureRef.current = true
    }

    if (userLayoutChangeRef.current || userResizeGestureRef.current) {
      followLatestRef.current = isAtBottom(scrollContainer)
    } else if (
      previousMessages === null ||
      appendedMessages ||
      messageItemsChanged ||
      start !== previousMessages.start ||
      epoch !== previousMessages.epoch
    ) {
      if (!followLatestRef.current) {
        previousChildLayoutRef.current =
          readConversationChildLayout(scrollContainer)
        previousScrollTopRef.current = scrollContainer.scrollTop
        return
      }
      if (appendedMessages && anchorExtraPaddingBottomRef.current > 0) {
        scrollContainer.style.removeProperty('padding-bottom')
        anchorExtraPaddingBottomRef.current = 0
      }
      if (!loadingOlder) {
        const scrollTop =
          scrollContainer.scrollHeight - scrollContainer.clientHeight
        scrollContainer.scrollTop = scrollTop
        onPruneOldMessages()
      }
      previousChildLayoutRef.current =
        readConversationChildLayout(scrollContainer)
      previousScrollTopRef.current = scrollContainer.scrollTop
      return
    }
    previousChildLayoutRef.current =
      readConversationChildLayout(scrollContainer)
    previousScrollTopRef.current = scrollContainer.scrollTop
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

    const initialResizeHeights = new WeakMap<Element, number>()
    const resizeObserver = new ResizeObserver((entries) => {
      let resizedExistingChild = false
      for (const entry of entries) {
        const initialHeight = initialResizeHeights.get(entry.target)
        initialResizeHeights.delete(entry.target)
        const currentHeight =
          entry.borderBoxSize[0]?.blockSize ?? entry.contentRect.height
        if (
          initialHeight === undefined ||
          Math.abs(currentHeight - initialHeight) > 0.5
        ) {
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
      const resizeCandidate = userResizeCandidateRef.current
      if (
        resizeCandidate !== null &&
        entries.some((entry) =>
          entry.target.contains(resizeCandidate.element),
        ) &&
        Math.abs(
          resizeCandidate.element.getBoundingClientRect().height -
            resizeCandidate.initialHeight,
        ) > 0.5
      ) {
        userResizeGestureRef.current = true
      }
      if (userResizeGestureRef.current) {
        followLatestRef.current = isAtBottom(container)
        return
      }
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
        initialResizeHeights.set(node, node.getBoundingClientRect().height)
        resizeObserver.observe(node, { box: 'border-box' })
      }
    }
    observeChildren(scrollContainer.children)

    const mutationObserver = new MutationObserver((records) => {
      for (const record of records) {
        for (const node of record.removedNodes) {
          if (!(node instanceof Element)) continue
          resizeObserver.unobserve(node)
          initialResizeHeights.delete(node)
          if (node instanceof HTMLElement && !scrollContainer.contains(node)) {
            clearConversationAnchorOffset(
              node,
              conversationAnchorOffsetsRef.current,
            )
            recentResolutionCardsRef.current.delete(node)
          }
        }
        observeChildren(record.addedNodes)
      }
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
    const startUserResizeGesture = (event: Event) => {
      if (!(event.target instanceof HTMLTextAreaElement)) return
      if (window.getComputedStyle(event.target).resize === 'none') return
      userResizeCandidateRef.current = {
        element: event.target,
        initialHeight: event.target.getBoundingClientRect().height,
      }
    }
    const finishUserResizeGesture = () => {
      const resizeCandidate = userResizeCandidateRef.current
      const resized =
        resizeCandidate !== null &&
        Math.abs(
          resizeCandidate.element.getBoundingClientRect().height -
            resizeCandidate.initialHeight,
        ) > 0.5
      if (userResizeGestureRef.current || resized) {
        markUserLayoutChange()
      }
      userResizeGestureRef.current = false
      userResizeCandidateRef.current = null
    }
    scrollContainer.addEventListener('click', markUserLayoutChange, true)
    scrollContainer.addEventListener('input', markUserLayoutChange, true)
    scrollContainer.addEventListener('change', markUserLayoutChange, true)
    scrollContainer.addEventListener('paste', markUserLayoutChange, true)
    scrollContainer.addEventListener('drop', markUserLayoutChange, true)
    scrollContainer.addEventListener(
      'pointerdown',
      startUserResizeGesture,
      true,
    )
    window.addEventListener('pointerup', finishUserResizeGesture, true)
    window.addEventListener('pointercancel', finishUserResizeGesture, true)

    return () => {
      resizeObserver.disconnect()
      mutationObserver.disconnect()
      scrollContainer.removeEventListener('click', markUserLayoutChange, true)
      scrollContainer.removeEventListener('input', markUserLayoutChange, true)
      scrollContainer.removeEventListener('change', markUserLayoutChange, true)
      scrollContainer.removeEventListener('paste', markUserLayoutChange, true)
      scrollContainer.removeEventListener('drop', markUserLayoutChange, true)
      scrollContainer.removeEventListener(
        'pointerdown',
        startUserResizeGesture,
        true,
      )
      window.removeEventListener('pointerup', finishUserResizeGesture, true)
      window.removeEventListener('pointercancel', finishUserResizeGesture, true)
      userResizeGestureRef.current = false
      userResizeCandidateRef.current = null
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
      previousChildLayoutRef.current =
        readConversationChildLayout(scrollContainer)
      previousScrollTopRef.current = scrollContainer.scrollTop
      return
    }
    programmaticScrollTopRef.current = null
    if (anchorExtraPaddingBottomRef.current > 0) {
      scrollContainer.style.removeProperty('padding-bottom')
      anchorExtraPaddingBottomRef.current = 0
    }
    const pendingAnchor = pendingScrollAnchorRef.current
    if (pendingAnchor !== null) {
      pendingScrollAnchorRef.current = {
        scrollHeight: scrollContainer.scrollHeight,
        scrollTop: scrollContainer.scrollTop,
        start: pendingAnchor.start,
        epoch: pendingAnchor.epoch,
      }
    }
    previousChildLayoutRef.current =
      readConversationChildLayout(scrollContainer)
    previousScrollTopRef.current = scrollContainer.scrollTop
    followLatestRef.current = isAtBottom(scrollContainer)
    if (followLatestRef.current) onPruneOldMessages()
    if (scrollContainer.scrollTop <= 120) loadOlder()
  }, [loadOlder, onPruneOldMessages])

  return { loadOlder, onScroll, scrollRef }
}
