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
  onArchiveAll,
  archiveAllDisabled,
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
  onArchiveAll?: () => void
  archiveAllDisabled?: boolean
}) {
  return (
    <section className={`session-section ${className}`} aria-label={title}>
      <h2 className="section-heading">
        <span>{title}</span>
        <span className="section-count">{count}</span>
        {onArchiveAll !== undefined ? (
          <Button
            type="button"
            variant="plain"
            className="inbox-archive-all cursor-pointer"
            disabled={archiveAllDisabled ?? sessions.length === 0}
            onClick={onArchiveAll}
          >
            Archive all
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
