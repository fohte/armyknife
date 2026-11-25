export type ConversationChildLayout = {
  element: HTMLElement
  top: number
  height: number
  cardResolution: 'pending' | 'resolved' | null
}

export type ConversationAnchorOffset = {
  offset: number
  originalTranslate: string
}

export function readConversationChildLayout(
  scrollContainer: HTMLDivElement,
): ConversationChildLayout[] {
  return Array.from(scrollContainer.children).flatMap((child) =>
    child instanceof HTMLElement
      ? [
          {
            element: child,
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
  return previous.some((previousChild) => {
    const currentChild = currentByElement.get(previousChild.element)
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
  return new Set(
    previous.flatMap((previousChild) => {
      const currentChild = currentByElement.get(previousChild.element)
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
): number | null {
  const bounds = scrollContainer.getBoundingClientRect()
  const currentByElement = new Map(
    current.map((child) => [child.element, child]),
  )
  const previousSharedElements = previous
    .filter(({ element }) => currentByElement.has(element))
    .map(({ element }) => element)
  const previousOrder = new Map(
    previousSharedElements.map((element, index) => [element, index]),
  )
  const currentOrder = new Map(
    current
      .filter(({ element }) => previousOrder.has(element))
      .map(({ element }, index) => [element, index]),
  )
  const hasKeptItsRelativeOrder = (element: HTMLElement) =>
    previousOrder.get(element) === currentOrder.get(element)
  const isVisible = ({ top, height }: ConversationChildLayout) =>
    top < bounds.bottom && top + height > bounds.top
  const anchor =
    previous.findLast(({ element, height, ...layout }) => {
      const currentChild = currentByElement.get(element)
      return (
        currentChild !== undefined &&
        !recentResolutionCards.has(element) &&
        hasKeptItsRelativeOrder(element) &&
        isVisible({ element, height, ...layout }) &&
        Math.abs(currentChild.height - height) <= 0.5
      )
    }) ??
    previous.findLast(
      ({ element, ...layout }) =>
        currentByElement.has(element) &&
        !recentResolutionCards.has(element) &&
        isVisible({ element, ...layout }),
    )
  if (anchor === undefined) return null
  const currentAnchor = currentByElement.get(anchor.element)
  if (currentAnchor === undefined) return null
  const offset = currentAnchor.top - anchor.top
  return Math.abs(offset) > 0.5 ? scrollContainer.scrollTop + offset : null
}

export function preserveConversationChildren(
  previous: readonly ConversationChildLayout[],
  current: readonly ConversationChildLayout[],
  scrollContainer: HTMLDivElement,
  recentResolutionCards: ReadonlySet<HTMLElement>,
  anchorOffsets: Map<HTMLElement, ConversationAnchorOffset>,
) {
  const bounds = scrollContainer.getBoundingClientRect()
  const currentByElement = new Map(
    current.map((child) => [child.element, child]),
  )
  for (const child of previous) {
    const visible =
      child.top < bounds.bottom && child.top + child.height > bounds.top
    if (
      (!visible && !isPendingConversationCard(child.element)) ||
      recentResolutionCards.has(child.element)
    ) {
      continue
    }
    const currentChild = currentByElement.get(child.element)
    if (currentChild === undefined) continue
    const previousOffset = anchorOffsets.get(child.element)
    if (previousOffset === undefined) {
      if (window.getComputedStyle(child.element).translate !== 'none') continue
      if (Math.abs(child.top - currentChild.top) <= 0.5) continue
      const offset = child.top - currentChild.top
      anchorOffsets.set(child.element, {
        offset,
        originalTranslate: child.element.style.translate,
      })
      child.element.style.translate = `0 ${String(offset)}px`
      continue
    }
    const offset = previousOffset.offset + child.top - currentChild.top
    if (Math.abs(offset) <= 0.5) {
      restoreConversationAnchorOffset(child.element, previousOffset)
      anchorOffsets.delete(child.element)
      continue
    }
    anchorOffsets.set(child.element, { ...previousOffset, offset })
    child.element.style.translate = `0 ${String(offset)}px`
  }
}

export function clearConversationAnchorOffset(
  element: HTMLElement,
  anchorOffsets: Map<HTMLElement, ConversationAnchorOffset>,
) {
  const offset = anchorOffsets.get(element)
  if (offset === undefined) return
  restoreConversationAnchorOffset(element, offset)
  anchorOffsets.delete(element)
}

export function clearConversationAnchorOffsets(
  anchorOffsets: Map<HTMLElement, ConversationAnchorOffset>,
) {
  for (const element of anchorOffsets.keys()) {
    clearConversationAnchorOffset(element, anchorOffsets)
  }
}

function conversationCardResolution(
  element: HTMLElement,
): ConversationChildLayout['cardResolution'] {
  if (element.classList.contains('chat-proposal-card')) {
    return element.dataset['answerable'] === 'true' ? 'pending' : 'resolved'
  }
  if (element.classList.contains('chat-todo-card')) {
    return element.dataset['resolution'] === 'pending' ? 'pending' : 'resolved'
  }
  return null
}

function isPendingConversationCard(element: HTMLElement): boolean {
  return (
    element.matches('.chat-proposal-card[data-answerable="true"]') ||
    element.matches('.chat-todo-card[data-resolution="pending"]')
  )
}

function restoreConversationAnchorOffset(
  element: HTMLElement,
  offset: ConversationAnchorOffset,
) {
  if (offset.originalTranslate === '') {
    element.style.removeProperty('translate')
  } else {
    element.style.translate = offset.originalTranslate
  }
}
