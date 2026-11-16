import { Button } from '@fohte/ui/button'

import { SessionRow } from '#components/session-row'
import type { Session } from '#types'

export function InboxSessionListItem({
  session,
  selectedId,
  onSelect,
  onLater,
  onToggleToday,
  isMobile,
}: {
  session: Session
  selectedId: string | null
  onSelect: (id: string) => void
  onLater: (id: string) => void
  onToggleToday: (id: string, isToday: boolean) => void
  isMobile: boolean
}) {
  return (
    <li className="session-list-item">
      <div
        className={`session-row-layout${session.isToday === true ? ' is-starred' : ''}`}
      >
        <Button
          type="button"
          variant="plain"
          className={`session-row cursor-pointer category-${session.category}${session.id === selectedId ? ' is-selected' : ''}`}
          data-session-row=""
          aria-current={session.id === selectedId ? 'true' : undefined}
          onClick={() => {
            onSelect(session.id)
          }}
        >
          <SessionRow session={session} density="full" isMobile={isMobile} />
        </Button>
        {!isMobile ? (
          <div className="session-row-actions">
            <Button
              type="button"
              variant="plain"
              aria-label={
                session.isToday === true
                  ? 'Remove from Starred'
                  : 'Add to Starred'
              }
              aria-pressed={session.isToday ?? false}
              className="session-action-button cursor-pointer"
              onClick={() => {
                onToggleToday(session.id, session.isToday !== true)
              }}
            >
              {session.isToday === true ? '★' : '☆'}
            </Button>
            <Button
              type="button"
              variant="plain"
              aria-label={`Later: ${session.title}`}
              className="session-action-button cursor-pointer"
              onClick={() => {
                onLater(session.id)
              }}
            >
              ◷
            </Button>
          </div>
        ) : null}
      </div>
    </li>
  )
}
