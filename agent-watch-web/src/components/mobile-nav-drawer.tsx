import '#components/mobile-nav-drawer.css'

import { type ReactNode, useEffect } from 'react'

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
  useEffect(() => {
    if (!open) return

    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key === 'Escape') onClose()
    }
    window.addEventListener('keydown', closeOnEscape)
    return () => {
      window.removeEventListener('keydown', closeOnEscape)
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
        role="dialog"
      >
        {children}
      </aside>
    </div>
  )
}
