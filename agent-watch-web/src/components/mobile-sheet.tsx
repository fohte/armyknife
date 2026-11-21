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
  const overlayRef = useRef<HTMLDivElement>(null)
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
    if (!canDismiss) return

    const handlePointerDown = (event: PointerEvent) => {
      const overlay = overlayRef.current
      if (
        event.target instanceof Node &&
        overlay !== null &&
        !overlay.contains(event.target)
      ) {
        onClose()
      }
    }

    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key !== 'Escape' || event.defaultPrevented) {
        return
      }
      event.preventDefault()
      onClose()
    }

    document.addEventListener('pointerdown', handlePointerDown, true)
    window.addEventListener('keydown', handleKeyDown)
    return () => {
      document.removeEventListener('pointerdown', handlePointerDown, true)
      window.removeEventListener('keydown', handleKeyDown)
    }
  }, [canDismiss, onClose])

  return (
    <div
      className="mobile-sheet-overlay"
      data-presentation={presentation}
      ref={overlayRef}
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
