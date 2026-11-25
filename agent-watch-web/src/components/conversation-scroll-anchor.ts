export type ConversationChildLayout = {
  element: HTMLElement
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
  preferPendingCards: boolean,
  followLatest: boolean,
): ConversationScrollAnchor | null {
  const bounds = scrollContainer.getBoundingClientRect()
  const currentByElement = new Map(
    current.map((child) => [child.element, child]),
  )
  const isVisible = ({
    top,
    height,
  }: Pick<ConversationChildLayout, 'top' | 'height'>) =>
    top < bounds.bottom && top + height > bounds.top
  const pendingAnchors = previous.filter(({ element, ...layout }) => {
    const currentChild = currentByElement.get(element)
    if (
      currentChild === undefined ||
      recentResolutionCards.has(element) ||
      !isPendingConversationCard(element)
    ) {
      return false
    }
    return followLatest || isVisible(layout)
  })
  const candidates =
    preferPendingCards && pendingAnchors.length > 0
      ? pendingAnchors
      : previous.filter(({ element, ...layout }) => {
          const currentChild = currentByElement.get(element)
          return (
            currentChild !== undefined &&
            !recentResolutionCards.has(element) &&
            isVisible(layout)
          )
        })
  const anchor = candidates.reduce<{
    child: ConversationChildLayout
    offset: number
  } | null>((maximum, child) => {
    const currentChild = currentByElement.get(child.element)
    const offset =
      currentChild === undefined ? null : currentChild.top - child.top
    return offset === null || (maximum !== null && offset <= maximum.offset)
      ? maximum
      : { child, offset }
  }, null)
  return anchor === null
    ? null
    : {
        element: anchor.child.element,
        top: anchor.child.top,
        scrollTop: scrollContainer.scrollTop + anchor.offset,
      }
}

export function preserveConversationChildren(
  previous: readonly ConversationChildLayout[],
  current: readonly ConversationChildLayout[],
  scrollContainer: HTMLDivElement,
  recentResolutionCards: ReadonlySet<HTMLElement>,
  preferPendingCards: boolean,
  anchorOffsets: Map<HTMLElement, ConversationAnchorSpacing>,
) {
  const bounds = scrollContainer.getBoundingClientRect()
  const previousByElement = new Map(
    previous.map((child) => [child.element, child]),
  )
  for (const child of current) {
    const previousChild = previousByElement.get(child.element)
    if (
      previousChild === undefined ||
      recentResolutionCards.has(child.element)
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
    if (Math.abs(offset) <= 0.5) {
      if (previousOffset !== undefined) {
        restoreConversationAnchorSpacing(child.element, previousOffset)
        anchorOffsets.delete(child.element)
      }
      continue
    }
    if (offset < 0) continue
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
