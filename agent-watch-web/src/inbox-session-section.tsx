import { Button } from '@fohte/ui/button'

import { InboxSessionListItem } from '#inbox-session-list-item'
import type { Session } from '#types'

export function renderInboxSessionSection({
  title,
  sessions,
  count = sessions.length,
  selectedId,
  onSelect,
  isMobile,
  className = '',
  onCloseAll,
  closeAllDisabled,
}: {
  title: string
  sessions: readonly Session[]
  count?: number
  selectedId: string | null
  onSelect: (id: string) => void
  isMobile: boolean
  className?: string
  onCloseAll?: () => void
  closeAllDisabled?: boolean
}) {
  return (
    <section className={`session-section ${className}`} aria-label={title}>
      <h2 className="section-heading">
        <span>{title}</span>
        <span className="section-count">{count}</span>
        {onCloseAll !== undefined ? (
          <Button
            type="button"
            variant="plain"
            className="inbox-close-all cursor-pointer"
            disabled={closeAllDisabled ?? sessions.length === 0}
            onClick={onCloseAll}
          >
            Close all
          </Button>
        ) : null}
      </h2>
      {sessions.length > 0 ? (
        <ul className="session-list">
          {sessions.map((session) => (
            <InboxSessionListItem
              key={session.id}
              session={session}
              selectedId={selectedId}
              onSelect={onSelect}
              isMobile={isMobile}
            />
          ))}
        </ul>
      ) : null}
    </section>
  )
}
