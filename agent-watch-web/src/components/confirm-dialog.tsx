import '#components/confirm-dialog.css'

import { type ReactNode, useId } from 'react'
import { createPortal } from 'react-dom'

type ConfirmDialogProps = {
  title: string
  description?: ReactNode
  children?: ReactNode
  confirmLabel: string
  cancelLabel?: string
  pendingLabel?: string
  pending?: boolean
  confirmDisabled?: boolean
  confirmVariant?: 'default' | 'danger'
  hideCancel?: boolean
  hideConfirm?: boolean
  presentation?: 'modal' | 'inline'
  mobile?: boolean
  className?: string
  titleClassName?: string
  confirmButtonClassName?: string
  cancelButtonClassName?: string
  dialogRole?: 'dialog' | 'alertdialog'
  portal?: boolean
  onCancel: () => void
  onConfirm: () => void
}

export function ConfirmDialog({
  title,
  description,
  children,
  confirmLabel,
  cancelLabel = 'Cancel',
  pendingLabel,
  pending = false,
  confirmDisabled = false,
  confirmVariant = 'default',
  hideCancel = false,
  hideConfirm = false,
  presentation = 'modal',
  mobile = false,
  className,
  titleClassName,
  confirmButtonClassName,
  cancelButtonClassName,
  dialogRole = 'alertdialog',
  portal = true,
  onCancel,
  onConfirm,
}: ConfirmDialogProps) {
  const titleId = useId()
  const content = (
    <section
      aria-labelledby={titleId}
      aria-modal={presentation === 'modal' || mobile || undefined}
      className={`confirm-dialog${presentation === 'inline' ? ' confirm-dialog--inline' : ''}${mobile ? ' confirm-dialog--mobile' : ''}${className === undefined ? '' : ` ${className}`}`}
      role={dialogRole}
      onKeyDown={(event) => {
        if (presentation === 'inline') return
        event.stopPropagation()
        if (event.key === 'Escape') {
          event.preventDefault()
          if (!pending) onCancel()
        } else if (
          event.key === 'Enter' &&
          event.target === event.currentTarget &&
          !pending &&
          !confirmDisabled
        ) {
          event.preventDefault()
          onConfirm()
        }
      }}
    >
      <h2 className={titleClassName} id={titleId}>
        {title}
      </h2>
      {description === undefined ? null : <p>{description}</p>}
      {children}
      {pending && pendingLabel !== undefined ? (
        <p aria-live="polite" role="status">
          {pendingLabel}
        </p>
      ) : null}
      <div className="confirm-dialog-actions">
        {hideCancel ? null : (
          <button
            className={cancelButtonClassName}
            disabled={pending}
            onClick={onCancel}
            type="button"
          >
            {cancelLabel}
          </button>
        )}
        {hideConfirm ? null : (
          <button
            autoFocus
            className={`confirm-dialog-confirm confirm-dialog-confirm--${confirmVariant}${confirmButtonClassName === undefined ? '' : ` ${confirmButtonClassName}`}`}
            disabled={pending || confirmDisabled}
            onClick={onConfirm}
            type="button"
          >
            {pending && pendingLabel !== undefined
              ? pendingLabel
              : confirmLabel}
          </button>
        )}
      </div>
    </section>
  )

  if (presentation === 'inline') return content

  const dialog = (
    <div
      className={`confirm-dialog-backdrop${mobile ? ' confirm-dialog-backdrop--mobile' : ''}`}
      onMouseDown={(event) => {
        if (event.target === event.currentTarget && !pending) onCancel()
      }}
    >
      {content}
    </div>
  )

  return !portal || typeof document === 'undefined'
    ? dialog
    : createPortal(dialog, document.body)
}
