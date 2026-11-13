import '#components/mobile-nav-drawer.css'

import { type ReactNode, useEffect, useRef } from 'react'

const focusableSelector =
  'a[href], button:not(:disabled), input:not(:disabled), select:not(:disabled), textarea:not(:disabled), [tabindex]:not([tabindex="-1"])'

type MobileNavDrawerProps = {
  open: boolean
  onClose: () => void
  children: ReactNode
}

export function MobileNavDrawer({
  open,
  onClose,
  children,
}: MobileNavDrawerProps) {
  const drawerRef = useRef<HTMLElement>(null)

  useEffect(() => {
    if (!open) return

    const drawer = drawerRef.current
    if (drawer === null) return

    const previouslyFocusedElement =
      document.activeElement instanceof HTMLElement
        ? document.activeElement
        : null
    const focusableElements = () =>
      Array.from(drawer.querySelectorAll<HTMLElement>(focusableSelector))
    focusableElements()[0]?.focus()

    const keepFocusInDrawer = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        event.preventDefault()
        onClose()
        return
      }
      if (event.key !== 'Tab') return

      const elements = focusableElements()
      const first = elements[0]
      const last = elements.at(-1)
      if (first === undefined || last === undefined) {
        event.preventDefault()
        drawer.focus()
        return
      }

      const activeElement = document.activeElement
      if (
        event.shiftKey &&
        (activeElement === first || !drawer.contains(activeElement))
      ) {
        event.preventDefault()
        last.focus()
      } else if (
        !event.shiftKey &&
        (activeElement === last || !drawer.contains(activeElement))
      ) {
        event.preventDefault()
        first.focus()
      }
    }
    window.addEventListener('keydown', keepFocusInDrawer)
    return () => {
      window.removeEventListener('keydown', keepFocusInDrawer)
      if (
        previouslyFocusedElement !== null &&
        previouslyFocusedElement.isConnected
      ) {
        previouslyFocusedElement.focus()
      }
    }
  }, [onClose, open])

  if (!open) return null

  return (
    <div className="mobile-nav-overlay">
      <button
        aria-label="Close navigation"
        className="mobile-nav-backdrop"
        onClick={onClose}
        type="button"
      />
      <aside
        aria-label="Mobile navigation"
        aria-modal="true"
        className="mobile-nav-drawer"
        ref={drawerRef}
        role="dialog"
        tabIndex={-1}
      >
        {children}
      </aside>
    </div>
  )
}
