import { useCallback, useLayoutEffect, useRef } from 'react'

import {
  clearConversationAnchorOffsets,
  clearOffscreenConversationScrollCompensation,
  type ConversationAnchorSpacing,
  type ConversationChildLayout,
  hasConversationChildrenInsertedBeforePendingCards,
  hasConversationResolutionChanged,
  isConversationAtBottom,
  movedConversationChildren,
  newlyResolvedConversationCards,
  preserveConversationChildPosition,
  preserveConversationChildren,
  preserveConversationScrollPosition,
  readConversationChildLayout,
} from '#components/conversation-scroll-anchor'
import { useConversationScrollObservers } from '#components/use-conversation-scroll-observers'
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

function hasConversationScrollCompensation(
  scrollContainer: HTMLDivElement,
  anchorOffsets: ReadonlyMap<HTMLElement, ConversationAnchorSpacing>,
  extraPaddingBottom: number,
) {
  return (
    anchorOffsets.size > 0 ||
    extraPaddingBottom > 0 ||
    scrollContainer.style.getPropertyValue(
      '--conversation-anchor-extra-padding-bottom',
    ) !== ''
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
  const userScrollInputRef = useRef(false)
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
    new Map<HTMLElement, ConversationAnchorSpacing>(),
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
      if (scrollContainer.scrollTop === scrollTop) {
        programmaticScrollTopRef.current = null
        return
      }
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
    scrollRef.current?.style.removeProperty(
      '--conversation-anchor-extra-padding-bottom',
    )
    anchorExtraPaddingBottomRef.current = 0
    userLayoutChangeRef.current = false
    if (userLayoutChangeTimeoutRef.current !== null) {
      window.clearTimeout(userLayoutChangeTimeoutRef.current)
      userLayoutChangeTimeoutRef.current = null
    }
    programmaticScrollTopRef.current = null
    userScrollInputRef.current = false
    suppressResizeRef.current = false
    userResizeGestureRef.current = false
    userResizeCandidateRef.current = null
  }, [sessionId])

  useLayoutEffect(() => {
    const scrollContainer = scrollRef.current
    if (scrollContainer === null) return
    const previousMessages = previousMessagesRef.current
    const previousChildLayout = previousChildLayoutRef.current
    if (userScrollInputRef.current) {
      const userScrolledUp =
        scrollContainer.scrollTop < previousScrollTopRef.current - 0.5
      userScrollInputRef.current = false
      followLatestRef.current =
        !userScrolledUp && isConversationAtBottom(scrollContainer)
    }
    const currentChildLayout = readConversationChildLayout(scrollContainer)
    const scrollBounds = scrollContainer.getBoundingClientRect()
    const hasVisiblePendingCard = previousChildLayout.some(
      ({ cardResolution, top, height }) =>
        cardResolution === 'pending' &&
        top < scrollBounds.bottom &&
        top + height > scrollBounds.top,
    )
    const preservePendingCards =
      followLatestRef.current || hasVisiblePendingCard
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
    const clearOffscreenCompensation = () => {
      if (
        !hasConversationScrollCompensation(
          scrollContainer,
          conversationAnchorOffsetsRef.current,
          anchorExtraPaddingBottomRef.current,
        )
      ) {
        return
      }
      anchorExtraPaddingBottomRef.current =
        clearOffscreenConversationScrollCompensation(
          scrollContainer,
          readConversationChildLayout(scrollContainer),
          conversationAnchorOffsetsRef.current,
          anchorExtraPaddingBottomRef.current,
          recentResolutionCardsRef.current,
          false,
          setAnchoredScrollTop,
        )
    }

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
        const preservationAnchor = preserveConversationChildPosition(
          scrollContainer,
          previousChildLayout,
          readConversationChildLayout(scrollContainer),
          recentResolutionCards,
          preservePendingCards,
        )
        preserveConversationChildren(
          previousChildLayout,
          readConversationChildLayout(scrollContainer),
          scrollContainer,
          recentResolutionCards,
          preservePendingCards,
          preservationAnchor?.element ?? null,
          conversationAnchorOffsetsRef.current,
        )
      }
      clearOffscreenCompensation()
      previousChildLayoutRef.current =
        readConversationChildLayout(scrollContainer)
      previousScrollTopRef.current = scrollContainer.scrollTop
      return
    }

    if (conversationLayoutChanged) {
      anchorExtraPaddingBottomRef.current = preserveConversationScrollPosition({
        scrollContainer,
        previous: previousChildLayout,
        current: currentChildLayout,
        recentResolutionCards,
        preferPendingCards: preservePendingCards,
        followLatest: followLatestRef.current,
        previousScrollTop: previousScrollTopRef.current,
        extraPaddingBottom: anchorExtraPaddingBottomRef.current,
        setScrollTop: setAnchoredScrollTop,
        anchorOffsets: conversationAnchorOffsetsRef.current,
      })
      clearOffscreenCompensation()
      previousChildLayoutRef.current =
        readConversationChildLayout(scrollContainer)
      previousScrollTopRef.current = scrollContainer.scrollTop
      followLatestRef.current = isConversationAtBottom(scrollContainer)
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
      followLatestRef.current = isConversationAtBottom(scrollContainer)
    } else if (
      previousMessages === null ||
      appendedMessages ||
      messageItemsChanged ||
      start !== previousMessages.start ||
      epoch !== previousMessages.epoch
    ) {
      if (!followLatestRef.current) {
        clearOffscreenCompensation()
        previousChildLayoutRef.current =
          readConversationChildLayout(scrollContainer)
        previousScrollTopRef.current = scrollContainer.scrollTop
        return
      }
      if (!loadingOlder) {
        const scrollTop =
          scrollContainer.scrollHeight - scrollContainer.clientHeight
        scrollContainer.scrollTop = scrollTop
        onPruneOldMessages()
      }
      clearOffscreenCompensation()
      previousChildLayoutRef.current =
        readConversationChildLayout(scrollContainer)
      previousScrollTopRef.current = scrollContainer.scrollTop
      return
    }
    clearOffscreenCompensation()
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

  useConversationScrollObservers({
    scrollRef,
    suppressResizeRef,
    loadingOlderRef,
    userResizeCandidateRef,
    userResizeGestureRef,
    programmaticScrollTopRef,
    userScrollInputRef,
    followLatestRef,
    userLayoutChangeRef,
    userLayoutChangeTimeoutRef,
    onPruneOldMessagesRef,
    recentResolutionCardsRef,
    conversationAnchorOffsetsRef,
  })

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
    if (
      programmaticScrollTopRef.current === scrollContainer.scrollTop &&
      !userScrollInputRef.current
    ) {
      programmaticScrollTopRef.current = null
      previousChildLayoutRef.current =
        readConversationChildLayout(scrollContainer)
      previousScrollTopRef.current = scrollContainer.scrollTop
      return
    }
    programmaticScrollTopRef.current = null
    const userScrolledUp =
      userScrollInputRef.current &&
      scrollContainer.scrollTop < previousScrollTopRef.current - 0.5
    userScrollInputRef.current = false
    if (
      hasConversationScrollCompensation(
        scrollContainer,
        conversationAnchorOffsetsRef.current,
        anchorExtraPaddingBottomRef.current,
      )
    ) {
      const wasAtBottom =
        !userScrolledUp && isConversationAtBottom(scrollContainer)
      anchorExtraPaddingBottomRef.current =
        clearOffscreenConversationScrollCompensation(
          scrollContainer,
          readConversationChildLayout(scrollContainer),
          conversationAnchorOffsetsRef.current,
          anchorExtraPaddingBottomRef.current,
          recentResolutionCardsRef.current,
          wasAtBottom,
          setAnchoredScrollTop,
        )
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
    followLatestRef.current =
      !userScrolledUp && isConversationAtBottom(scrollContainer)
    if (followLatestRef.current) onPruneOldMessages()
    if (scrollContainer.scrollTop <= 120) loadOlder()
  }, [loadOlder, onPruneOldMessages, setAnchoredScrollTop])

  return { loadOlder, onScroll, scrollRef }
}
