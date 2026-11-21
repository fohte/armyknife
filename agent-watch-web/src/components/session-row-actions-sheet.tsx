import '#components/session-row-actions-sheet.css'

import { Button } from '@fohte/ui/button'
import { Clock3, X } from 'lucide-react'

import { MobileSheet } from '#components/mobile-sheet'
import { getSessionCloseCopy } from '#session-close-copy'
import type { Session } from '#types'

type SessionRowActionsSheetProps = {
  session: Session
  onLater: () => void
  onCloseSession?: (() => void) | undefined
  onCancel: () => void
}

export function SessionRowActionsSheet({
  session,
  onLater,
  onCloseSession,
  onCancel,
}: SessionRowActionsSheetProps) {
  const state = getSessionCloseCopy(session).state

  return (
    <MobileSheet
      className="session-row-actions-sheet"
      label="Session actions"
      onClose={onCancel}
    >
      <div className="session-row-actions-heading">
        <span aria-hidden="true" className="session-row-actions-dot" />
        <span>{state}</span>
        <strong title={session.title}>{session.title}</strong>
      </div>
      <div
        aria-label="Session actions"
        className="session-row-actions-menu"
        role="menu"
      >
        <Button type="button" variant="plain" role="menuitem" onClick={onLater}>
          <Clock3 aria-hidden="true" size={18} />
          <span>Later</span>
          <span aria-hidden="true" className="session-row-actions-chevron">
            ›
          </span>
        </Button>
        {onCloseSession === undefined || session.category === 'done' ? null : (
          <Button
            type="button"
            variant="plain"
            role="menuitem"
            className="session-row-actions-close"
            onClick={onCloseSession}
          >
            <X aria-hidden="true" size={18} />
            Close session…
          </Button>
        )}
      </div>
      <div className="session-row-actions-cancel">
        <Button type="button" variant="plain" onClick={onCancel}>
          Cancel
        </Button>
      </div>
    </MobileSheet>
  )
}
