import { type RefObject, useEffect } from 'react'

const FOCUSABLE_SELECTOR =
  'a[href], button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), iframe:not([tabindex="-1"]), [tabindex]:not([tabindex="-1"])'

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
      const focusIsOnContainer = document.activeElement === container
      if (
        (event.shiftKey &&
          (document.activeElement === first ||
            focusIsOutsideContainer ||
            focusIsOnContainer)) ||
        (!event.shiftKey &&
          (document.activeElement === last ||
            focusIsOutsideContainer ||
            focusIsOnContainer))
      ) {
        event.preventDefault()
        const focusTarget = event.shiftKey ? last : first
        focusTarget.focus()
      }
    }
    const onFocusIn = (event: FocusEvent) => {
      if (!(event.target instanceof Node) || container.contains(event.target)) {
        return
      }
      const firstFocusableElement = getFocusableElements()[0]
      const focusTarget = firstFocusableElement ?? container
      focusTarget.focus()
    }

    const firstFocusableElement = getFocusableElements()[0]
    const initialFocusTarget =
      initialFocusRef?.current ?? firstFocusableElement ?? container
    initialFocusTarget.focus()
    document.addEventListener('keydown', onKeyDown)
    document.addEventListener('focusin', onFocusIn)
    return () => {
      document.removeEventListener('keydown', onKeyDown)
      document.removeEventListener('focusin', onFocusIn)
      if (
        previouslyFocused instanceof HTMLElement &&
        previouslyFocused.isConnected
      ) {
        previouslyFocused.focus()
      }
    }
  }, [containerRef, initialFocusRef, isActive])
}
