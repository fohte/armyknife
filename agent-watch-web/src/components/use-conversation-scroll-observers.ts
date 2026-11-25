import { useLayoutEffect } from 'react'

import {
  clearConversationAnchorOffset,
  type ConversationAnchorSpacing,
  isConversationAtBottom,
} from '#components/conversation-scroll-anchor'

type MutableRef<T> = { current: T }

type UseConversationScrollObserversOptions = {
  scrollRef: MutableRef<HTMLDivElement | null>
  suppressResizeRef: MutableRef<boolean>
  loadingOlderRef: MutableRef<boolean>
  userResizeCandidateRef: MutableRef<{
    element: HTMLTextAreaElement
    initialHeight: number
  } | null>
  userResizeGestureRef: MutableRef<boolean>
  followLatestRef: MutableRef<boolean>
  userLayoutChangeRef: MutableRef<boolean>
  userLayoutChangeTimeoutRef: MutableRef<number | null>
  onPruneOldMessagesRef: MutableRef<() => void>
  recentResolutionCardsRef: MutableRef<Set<HTMLElement>>
  conversationAnchorOffsetsRef: MutableRef<
    Map<HTMLElement, ConversationAnchorSpacing>
  >
}

export function useConversationScrollObservers({
  scrollRef,
  suppressResizeRef,
  loadingOlderRef,
  userResizeCandidateRef,
  userResizeGestureRef,
  followLatestRef,
  userLayoutChangeRef,
  userLayoutChangeTimeoutRef,
  onPruneOldMessagesRef,
  recentResolutionCardsRef,
  conversationAnchorOffsetsRef,
}: UseConversationScrollObserversOptions) {
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
        followLatestRef.current = isConversationAtBottom(container)
        return
      }
      if (userLayoutChangeRef.current) {
        userLayoutChangeRef.current = false
        if (userLayoutChangeTimeoutRef.current !== null) {
          window.clearTimeout(userLayoutChangeTimeoutRef.current)
          userLayoutChangeTimeoutRef.current = null
        }
        followLatestRef.current = isConversationAtBottom(container)
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
}
