export type ConversationChildLayout = {
  element: HTMLElement
  cardId: string | null
  top: number
  height: number
  cardResolution: 'pending' | 'resolved' | null
}

export type ConversationAnchorSpacing = {
  offset: number
  baseMarginBlockStart: number
  originalMarginBlockStart: string
}

export type ConversationScrollAnchor = {
  element: HTMLElement
  top: number
  scrollTop: number
}

type PreserveConversationScrollPositionOptions = {
  scrollContainer: HTMLDivElement
  previous: readonly ConversationChildLayout[]
  current: readonly ConversationChildLayout[]
  recentResolutionCards: ReadonlySet<HTMLElement>
  preferPendingCards: boolean
  followLatest: boolean
  previousScrollTop: number
  extraPaddingBottom: number
  setScrollTop: (scrollContainer: HTMLDivElement, scrollTop: number) => void
  anchorOffsets: Map<HTMLElement, ConversationAnchorSpacing>
}

export function isConversationAtBottom(scrollContainer: HTMLDivElement) {
  return (
    scrollContainer.scrollHeight -
      scrollContainer.scrollTop -
      scrollContainer.clientHeight <=
    24
  )
}

export function readConversationChildLayout(
  scrollContainer: HTMLDivElement,
): ConversationChildLayout[] {
  return Array.from(scrollContainer.children).flatMap((child) =>
    child instanceof HTMLElement
      ? [
          {
            element: child,
            cardId: child.dataset['conversationCardId'] ?? null,
            top: child.getBoundingClientRect().top,
            height: child.getBoundingClientRect().height,
            cardResolution: conversationCardResolution(child),
          },
        ]
      : [],
  )
}

export function movedConversationChildren(
  previous: readonly ConversationChildLayout[],
  current: readonly ConversationChildLayout[],
): Set<HTMLElement> {
  const currentElements = new Set(current.map(({ element }) => element))
  const previousShared = previous.filter(({ element }) =>
    currentElements.has(element),
  )
  const previousElements = new Set(previousShared.map(({ element }) => element))
  const currentShared = current.filter(({ element }) =>
    previousElements.has(element),
  )
  const currentPositions = new Map(
    currentShared.map(({ element }, index) => [element, index]),
  )
  return new Set(
    previousShared.flatMap(({ element }, index) => {
      const currentIndex = currentPositions.get(element)
      return currentIndex !== undefined && currentIndex !== index
        ? [element]
        : []
    }),
  )
}

export function hasConversationChildrenInsertedBeforePendingCards(
  previous: readonly ConversationChildLayout[],
  current: readonly ConversationChildLayout[],
): boolean {
  if (!previous.some(({ element }) => isPendingConversationCard(element))) {
    return false
  }
  const previousElements = new Set(previous.map(({ element }) => element))
  let pendingCardFollows = false
  for (const { element } of current.toReversed()) {
    if (!previousElements.has(element) && pendingCardFollows) return true
    if (isPendingConversationCard(element)) {
      pendingCardFollows = true
    }
  }
  return false
}

export function hasConversationResolutionChanged(
  previous: readonly ConversationChildLayout[],
  current: readonly ConversationChildLayout[],
): boolean {
  const currentByElement = new Map(
    current.map((child) => [child.element, child]),
  )
  const currentByCardId = indexConversationChildrenByCardId(current)
  return previous.some((previousChild) => {
    const currentChild = findConversationChild(
      previousChild,
      currentByElement,
      currentByCardId,
    )
    return (
      currentChild !== undefined &&
      previousChild.cardResolution !== null &&
      currentChild.cardResolution !== previousChild.cardResolution
    )
  })
}

export function newlyResolvedConversationCards(
  previous: readonly ConversationChildLayout[],
  current: readonly ConversationChildLayout[],
): Set<HTMLElement> {
  const currentByElement = new Map(
    current.map((child) => [child.element, child]),
  )
  const currentByCardId = indexConversationChildrenByCardId(current)
  return new Set(
    previous.flatMap((previousChild) => {
      const currentChild = findConversationChild(
        previousChild,
        currentByElement,
        currentByCardId,
      )
      return previousChild.cardResolution === 'pending' &&
        currentChild?.cardResolution === 'resolved'
        ? [previousChild.element]
        : []
    }),
  )
}

export function preserveConversationChildPosition(
  scrollContainer: HTMLDivElement,
  previous: readonly ConversationChildLayout[],
  current: readonly ConversationChildLayout[],
  recentResolutionCards: ReadonlySet<HTMLElement>,
  preferPendingCards: boolean,
): ConversationScrollAnchor | null {
  const bounds = scrollContainer.getBoundingClientRect()
  const currentByElement = new Map(
    current.map((child) => [child.element, child]),
  )
  const currentByCardId = indexConversationChildrenByCardId(current)
  const isVisible = ({
    top,
    height,
  }: Pick<ConversationChildLayout, 'top' | 'height'>) =>
    top < bounds.bottom && top + height > bounds.top
  const visibleChildren = previous.filter((previousChild) => {
    const currentChild = findConversationChild(
      previousChild,
      currentByElement,
      currentByCardId,
    )
    if (
      currentChild === undefined ||
      recentResolutionCards.has(previousChild.element) ||
      !isVisible(previousChild)
    ) {
      return false
    }
    return true
  })
  const pendingChildren = visibleChildren.filter(({ element }) =>
    isPendingConversationCard(element),
  )
  const candidates =
    preferPendingCards && pendingChildren.length > 0
      ? pendingChildren
      : visibleChildren
  const firstCandidate = candidates[0]
  const firstCandidateCurrent =
    firstCandidate === undefined
      ? undefined
      : findConversationChild(firstCandidate, currentByElement, currentByCardId)
  return firstCandidate === undefined || firstCandidateCurrent === undefined
    ? null
    : {
        element: firstCandidate.element,
        top: firstCandidate.top,
        scrollTop:
          scrollContainer.scrollTop +
          firstCandidateCurrent.top -
          firstCandidate.top,
      }
}

export function preserveConversationScrollPosition({
  scrollContainer,
  previous,
  current,
  recentResolutionCards,
  preferPendingCards,
  followLatest,
  previousScrollTop,
  extraPaddingBottom: initialExtraPaddingBottom,
  setScrollTop,
  anchorOffsets,
}: PreserveConversationScrollPositionOptions): number {
  const scrollAnchor = preserveConversationChildPosition(
    scrollContainer,
    previous,
    current,
    recentResolutionCards,
    preferPendingCards,
  )
  let desiredScrollTop = scrollAnchor?.scrollTop ?? previousScrollTop
  const maximumScrollTop =
    scrollContainer.scrollHeight - scrollContainer.clientHeight
  if (
    scrollAnchor === null &&
    followLatest &&
    desiredScrollTop > maximumScrollTop
  ) {
    desiredScrollTop = maximumScrollTop
  }
  let extraPaddingBottom = initialExtraPaddingBottom
  const setScrollTopWithPadding = (scrollTop: number) => {
    setScrollTop(scrollContainer, scrollTop)
    const shortfall = scrollTop - scrollContainer.scrollTop
    if (shortfall <= 0.5) return
    extraPaddingBottom += shortfall
    scrollContainer.style.setProperty(
      '--conversation-anchor-extra-padding-bottom',
      `${String(extraPaddingBottom)}px`,
    )
    setScrollTop(scrollContainer, scrollTop)
  }

  setScrollTopWithPadding(desiredScrollTop)
  if (scrollAnchor !== null) {
    const correction =
      scrollAnchor.element.getBoundingClientRect().top - scrollAnchor.top
    if (Math.abs(correction) > 0.5) {
      desiredScrollTop = scrollContainer.scrollTop + correction
      setScrollTopWithPadding(desiredScrollTop)
    }
  }
  preserveConversationChildren(
    previous,
    readConversationChildLayout(scrollContainer),
    scrollContainer,
    recentResolutionCards,
    preferPendingCards,
    scrollAnchor?.element ?? null,
    anchorOffsets,
  )

  return extraPaddingBottom
}

export function preserveConversationChildren(
  previous: readonly ConversationChildLayout[],
  current: readonly ConversationChildLayout[],
  scrollContainer: HTMLDivElement,
  recentResolutionCards: ReadonlySet<HTMLElement>,
  preferPendingCards: boolean,
  anchorElement: HTMLElement | null,
  anchorOffsets: Map<HTMLElement, ConversationAnchorSpacing>,
) {
  const bounds = scrollContainer.getBoundingClientRect()
  const previousByElement = new Map(
    previous.map((child) => [child.element, child]),
  )
  const anchorIndex = current.findIndex(
    ({ element }) => element === anchorElement,
  )
  for (const [index, child] of current.entries()) {
    const previousChild = previousByElement.get(child.element)
    if (
      previousChild === undefined ||
      recentResolutionCards.has(child.element) ||
      (anchorIndex >= 0 && index < anchorIndex)
    ) {
      continue
    }
    const isPending = isPendingConversationCard(child.element)
    const visible =
      previousChild.top < bounds.bottom &&
      previousChild.top + previousChild.height > bounds.top
    if (!visible && !(preferPendingCards && isPending)) continue

    const previousOffset = anchorOffsets.get(child.element)
    const offset =
      (previousOffset?.offset ?? 0) +
      previousChild.top -
      child.element.getBoundingClientRect().top
    if (offset <= 0.5) {
      if (previousOffset !== undefined) {
        restoreConversationAnchorSpacing(child.element, previousOffset)
        anchorOffsets.delete(child.element)
      }
      continue
    }
    const spacing = previousOffset ?? {
      offset: 0,
      baseMarginBlockStart:
        Number.parseFloat(
          window.getComputedStyle(child.element).marginBlockStart,
        ) || 0,
      originalMarginBlockStart: child.element.style.marginBlockStart,
    }
    anchorOffsets.set(child.element, { ...spacing, offset })
    child.element.style.marginBlockStart = `${String(spacing.baseMarginBlockStart + offset)}px`
  }
}

export function clearConversationAnchorOffset(
  element: HTMLElement,
  anchorOffsets: Map<HTMLElement, ConversationAnchorSpacing>,
) {
  const offset = anchorOffsets.get(element)
  if (offset === undefined) return
  restoreConversationAnchorSpacing(element, offset)
  anchorOffsets.delete(element)
}

export function clearConversationAnchorOffsets(
  anchorOffsets: Map<HTMLElement, ConversationAnchorSpacing>,
) {
  for (const element of anchorOffsets.keys()) {
    clearConversationAnchorOffset(element, anchorOffsets)
  }
}

export function clearConversationScrollCompensation(
  scrollContainer: HTMLDivElement,
  anchorLayout: readonly ConversationChildLayout[],
  anchorOffsets: Map<HTMLElement, ConversationAnchorSpacing>,
  recentResolutionCards: ReadonlySet<HTMLElement>,
  wasAtBottom: boolean,
  setScrollTop: (scrollContainer: HTMLDivElement, scrollTop: number) => void,
) {
  const bounds = scrollContainer.getBoundingClientRect()
  const visibleChildren = anchorLayout.filter(
    ({ top, height }) => top < bounds.bottom && top + height > bounds.top,
  )
  const anchor =
    visibleChildren.find(
      ({ element }) => !recentResolutionCards.has(element),
    ) ?? visibleChildren[0]

  clearConversationAnchorOffsets(anchorOffsets)
  scrollContainer.style.removeProperty(
    '--conversation-anchor-extra-padding-bottom',
  )

  if (wasAtBottom) {
    setScrollTop(
      scrollContainer,
      scrollContainer.scrollHeight - scrollContainer.clientHeight,
    )
  } else if (anchor !== undefined) {
    const correction = anchor.element.getBoundingClientRect().top - anchor.top
    if (Math.abs(correction) > 0.5) {
      setScrollTop(scrollContainer, scrollContainer.scrollTop + correction)
    }
  }
}

export function clearOffscreenConversationScrollCompensation(
  scrollContainer: HTMLDivElement,
  anchorLayout: readonly ConversationChildLayout[],
  anchorOffsets: Map<HTMLElement, ConversationAnchorSpacing>,
  extraPaddingBottom: number,
  recentResolutionCards: ReadonlySet<HTMLElement>,
  wasAtBottom: boolean,
  setScrollTop: (scrollContainer: HTMLDivElement, scrollTop: number) => void,
): number {
  if (wasAtBottom) {
    clearConversationScrollCompensation(
      scrollContainer,
      anchorLayout,
      anchorOffsets,
      recentResolutionCards,
      true,
      setScrollTop,
    )
    return 0
  }

  const bounds = scrollContainer.getBoundingClientRect()
  const previousScrollTop = scrollContainer.scrollTop
  let removedAboveViewport = 0
  const clearOffsets = [...anchorOffsets].flatMap(([element, spacing]) => {
    if (!scrollContainer.contains(element)) return [element]
    const top = element.getBoundingClientRect().top
    const spacingTop = top - spacing.offset
    const height = element.getBoundingClientRect().height
    if (top + height <= bounds.top) {
      removedAboveViewport += spacing.offset
      return [element]
    }
    return spacingTop >= bounds.bottom ? [element] : []
  })
  for (const element of clearOffsets) {
    clearConversationAnchorOffset(element, anchorOffsets)
  }

  let remainingExtraPaddingBottom = extraPaddingBottom
  if (
    remainingExtraPaddingBottom <= 0 &&
    scrollContainer.style.getPropertyValue(
      '--conversation-anchor-extra-padding-bottom',
    ) !== ''
  ) {
    scrollContainer.style.removeProperty(
      '--conversation-anchor-extra-padding-bottom',
    )
  }
  const viewportBottom =
    scrollContainer.scrollTop + scrollContainer.clientHeight
  const extraPaddingTop =
    scrollContainer.scrollHeight - remainingExtraPaddingBottom
  const maximumWithoutExtraPadding =
    scrollContainer.scrollHeight -
    scrollContainer.clientHeight -
    remainingExtraPaddingBottom
  if (
    remainingExtraPaddingBottom > 0 &&
    extraPaddingTop >= viewportBottom &&
    scrollContainer.scrollTop <= maximumWithoutExtraPadding + 0.5
  ) {
    scrollContainer.style.removeProperty(
      '--conversation-anchor-extra-padding-bottom',
    )
    remainingExtraPaddingBottom = 0
  }

  if (removedAboveViewport > 0.5) {
    setScrollTop(scrollContainer, previousScrollTop - removedAboveViewport)
  }
  return remainingExtraPaddingBottom
}

function conversationCardResolution(
  element: HTMLElement,
): ConversationChildLayout['cardResolution'] {
  if (element.classList.contains('chat-withdrawn-card')) return 'resolved'
  if (element.classList.contains('chat-proposal-card')) {
    return element.dataset['answerable'] === 'true' ? 'pending' : 'resolved'
  }
  if (element.classList.contains('chat-todo-card')) {
    return element.dataset['resolution'] === 'pending' ? 'pending' : 'resolved'
  }
  return null
}

export function indexConversationChildrenByCardId(
  children: readonly ConversationChildLayout[],
) {
  return new Map(
    children.flatMap((child) =>
      child.cardId === null ? [] : [[child.cardId, child] as const],
    ),
  )
}

export function findConversationChild(
  previous: ConversationChildLayout,
  currentByElement: ReadonlyMap<HTMLElement, ConversationChildLayout>,
  currentByCardId: ReadonlyMap<string, ConversationChildLayout>,
): ConversationChildLayout | undefined {
  return (
    currentByElement.get(previous.element) ??
    (previous.cardId === null
      ? undefined
      : currentByCardId.get(previous.cardId))
  )
}

function isPendingConversationCard(element: HTMLElement): boolean {
  return (
    element.matches('.chat-proposal-card[data-answerable="true"]') ||
    element.matches('.chat-todo-card[data-resolution="pending"]')
  )
}

function restoreConversationAnchorSpacing(
  element: HTMLElement,
  offset: ConversationAnchorSpacing,
) {
  if (offset.originalMarginBlockStart === '') {
    element.style.removeProperty('margin-block-start')
  } else {
    element.style.marginBlockStart = offset.originalMarginBlockStart
  }
}
