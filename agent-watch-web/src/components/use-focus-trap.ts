import { type RefObject, useEffect } from 'react'

const FOCUSABLE_SELECTOR =
  'a[href], button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])'

type UseFocusTrapOptions<T extends HTMLElement> = {
  containerRef: RefObject<T | null>
  initialFocusRef?: RefObject<HTMLElement | null>
  isActive: boolean
}

export function useFocusTrap<T extends HTMLElement>({
  containerRef,
  initialFocusRef,
  isActive,
}: UseFocusTrapOptions<T>) {
  useEffect(() => {
    if (!isActive) return
    const container = containerRef.current
    if (container === null) return

    const previouslyFocused = document.activeElement
    const getFocusableElements = () =>
      Array.from(
        container.querySelectorAll<HTMLElement>(FOCUSABLE_SELECTOR),
      ).filter((element) => element.getClientRects().length > 0)
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key !== 'Tab') return
      const focusableElements = getFocusableElements()
      const first = focusableElements[0]
      const last = focusableElements.at(-1)
      if (first === undefined || last === undefined) {
        event.preventDefault()
        container.focus()
        return
      }

      const focusIsOutsideContainer = !container.contains(
        document.activeElement,
      )
      if (
        (event.shiftKey &&
          (document.activeElement === first || focusIsOutsideContainer)) ||
        (!event.shiftKey &&
          (document.activeElement === last || focusIsOutsideContainer))
      ) {
        event.preventDefault()
        const focusTarget = event.shiftKey ? last : first
        focusTarget.focus()
      }
    }

    const firstFocusableElement = getFocusableElements()[0]
    const initialFocusTarget =
      initialFocusRef?.current ?? firstFocusableElement ?? container
    initialFocusTarget.focus()
    document.addEventListener('keydown', onKeyDown)
    return () => {
      document.removeEventListener('keydown', onKeyDown)
      if (
        previouslyFocused instanceof HTMLElement &&
        previouslyFocused.isConnected
      ) {
        previouslyFocused.focus()
      }
    }
  }, [containerRef, initialFocusRef, isActive])
}
