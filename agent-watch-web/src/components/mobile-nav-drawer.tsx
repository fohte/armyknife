import '#components/mobile-nav-drawer.css'

import { type ReactNode, useEffect, useRef } from 'react'

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
  const dialogRef = useRef<HTMLDialogElement>(null)

  useEffect(() => {
    const dialog = dialogRef.current
    if (dialog === null) return

    if (open && !dialog.open) {
      dialog.showModal()
    } else if (!open && dialog.open) {
      dialog.close()
    }
  }, [open])

  return (
    <dialog
      aria-label="Mobile navigation"
      className="mobile-nav-drawer"
      onCancel={(event) => {
        event.preventDefault()
        onClose()
      }}
      onClick={(event) => {
        if (event.target === event.currentTarget) onClose()
      }}
      ref={dialogRef}
    >
      {children}
    </dialog>
  )
}
