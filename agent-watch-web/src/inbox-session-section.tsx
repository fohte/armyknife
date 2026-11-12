import { Button } from '@fohte/ui/button'

import { SessionRow } from '#components/session-row'
import type { Session } from '#types'

export function renderInboxSessionSection({
  title,
  sessions,
  count = sessions.length,
  selectedId,
  onSelect,
  onLater,
  onToggleToday,
  compactRepo,
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
  onLater: (id: string) => void
  onToggleToday: (id: string, isToday: boolean) => void
  compactRepo: boolean
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
            <SessionRow
              key={session.id}
              session={session}
              selected={session.id === selectedId}
              onSelect={onSelect}
              onLater={onLater}
              onToggleToday={onToggleToday}
              compactRepo={compactRepo}
              isMobile={isMobile}
            />
          ))}
        </ul>
      ) : null}
    </section>
  )
}
