import '#components/mobile-sheet.css'

import type { ReactNode, RefObject } from 'react'
import { useEffect, useRef } from 'react'

import { useFocusTrap } from '#components/use-focus-trap'

type MobileSheetProps = {
  label: string
  children: ReactNode
  onClose: () => void
  className?: string | undefined
  initialFocusRef?: RefObject<HTMLElement | null> | undefined
  focusContainer?: boolean | undefined
  presentation?: 'bottom' | 'fullscreen' | undefined
  canDismiss?: boolean | undefined
}

export function MobileSheet({
  label,
  children,
  onClose,
  className,
  initialFocusRef,
  focusContainer = false,
  presentation = 'bottom',
  canDismiss = true,
}: MobileSheetProps) {
  const sheetRef = useRef<HTMLElement>(null)
  useFocusTrap({
    containerRef: sheetRef,
    isActive: true,
    ...(focusContainer
      ? { initialFocusRef: sheetRef }
      : initialFocusRef === undefined
        ? {}
        : { initialFocusRef }),
  })
  useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key !== 'Escape' || event.defaultPrevented || !canDismiss) {
        return
      }
      event.preventDefault()
      onClose()
    }

    window.addEventListener('keydown', handleKeyDown)
    return () => {
      window.removeEventListener('keydown', handleKeyDown)
    }
  }, [canDismiss, onClose])

  return (
    <div
      className="mobile-sheet-overlay"
      data-presentation={presentation}
      onClick={(event) => {
        if (canDismiss && event.target === event.currentTarget) onClose()
      }}
    >
      <section
        aria-label={label}
        aria-modal="true"
        className={`mobile-sheet mobile-sheet--${presentation}${className === undefined ? '' : ` ${className}`}`}
        ref={sheetRef}
        role="dialog"
        tabIndex={-1}
      >
        {children}
      </section>
    </div>
  )
}
