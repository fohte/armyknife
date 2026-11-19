import '#components/session-close-confirmation.css'

import { ConfirmDialog } from '#components/confirm-dialog'
import { getSessionCloseCopy } from '#session-close-copy'
import type { Session } from '#types'

type SessionCloseConfirmationProps = {
  session: Session
  mobile?: boolean
  onCancel: () => void
  onConfirm: () => void
}

export function SessionCloseConfirmation({
  session,
  mobile = false,
  onCancel,
  onConfirm,
}: SessionCloseConfirmationProps) {
  const copy = getSessionCloseCopy(session)

  return (
    <ConfirmDialog
      className="session-close-confirmation"
      title={copy.title}
      description={copy.description}
      confirmLabel={copy.confirmLabel}
      confirmVariant="danger"
      mobile={mobile}
      onCancel={onCancel}
      onConfirm={onConfirm}
    >
      <div className="session-close-confirmation-session">
        <span>{copy.state}</span>
        <strong title={session.title}>{session.title}</strong>
      </div>
      <p className="session-close-confirmation-shortcuts">
        ↵ {copy.confirmLabel} · Esc Cancel
      </p>
    </ConfirmDialog>
  )
}
