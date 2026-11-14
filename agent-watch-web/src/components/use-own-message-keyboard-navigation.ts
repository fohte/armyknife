import { useEffect } from 'react'

type UseOwnMessageKeyboardNavigationOptions = {
  isMobile: boolean
  scrollRef: { current: HTMLDivElement | null }
}

export function useOwnMessageKeyboardNavigation({
  isMobile,
  scrollRef,
}: UseOwnMessageKeyboardNavigationOptions) {
  useEffect(() => {
    if (isMobile) return

    const onKeyDown = (event: KeyboardEvent) => {
      if (
        event.defaultPrevented ||
        !event.altKey ||
        event.ctrlKey ||
        event.metaKey ||
        event.shiftKey ||
        (event.key !== 'ArrowUp' && event.key !== 'ArrowDown')
      ) {
        return
      }

      const target = event.target
      if (
        (target instanceof HTMLInputElement ||
          target instanceof HTMLTextAreaElement) &&
        target.value.length > 0
      ) {
        return
      }
      if (
        target instanceof HTMLElement &&
        target.isContentEditable &&
        target.textContent.length > 0
      ) {
        return
      }

      const scrollContainer = scrollRef.current
      if (scrollContainer === null) return
      const ownMessages = Array.from(
        scrollContainer.querySelectorAll<HTMLElement>(
          '[data-own-message="true"]',
        ),
      )
      if (ownMessages.length === 0) return

      const viewportTop =
        scrollContainer.getBoundingClientRect().top + scrollContainer.clientTop
      const viewportBottom = viewportTop + scrollContainer.clientHeight
      const messageBounds = ownMessages.map((message) =>
        message.getBoundingClientRect(),
      )
      const firstVisibleIndex = messageBounds.findIndex(
        (bounds) => bounds.bottom > viewportTop && bounds.top < viewportBottom,
      )
      const targetIndex =
        firstVisibleIndex !== -1
          ? event.key === 'ArrowUp'
            ? firstVisibleIndex - 1
            : firstVisibleIndex + 1
          : event.key === 'ArrowUp'
            ? messageBounds.findLastIndex(
                (bounds) => bounds.bottom <= viewportTop,
              )
            : messageBounds.findIndex((bounds) => bounds.top >= viewportBottom)
      const message = ownMessages[targetIndex]
      if (message === undefined) return

      event.preventDefault()
      message.scrollIntoView({ block: 'center', behavior: 'smooth' })
    }

    window.addEventListener('keydown', onKeyDown)
    return () => {
      window.removeEventListener('keydown', onKeyDown)
    }
  }, [isMobile, scrollRef])
}
