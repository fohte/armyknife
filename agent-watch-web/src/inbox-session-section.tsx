import { Button } from '@fohte/ui/button'

import { InboxSessionList } from '#components/inbox-session-list'
import type { Session } from '#types'

export function renderInboxSessionSection({
  title,
  sessions,
  count = sessions.length,
  selectedId,
  onSelect,
  isMobile,
  onLater,
  onCloseSession,
  onOpenActions,
  className = '',
  onCloseAll,
  closeAllDisabled,
  groupByProject = false,
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
  groupByProject?: boolean
  onLater?: ((id: string) => void) | undefined
  onCloseSession?: ((id: string) => void) | undefined
  onOpenActions?: ((session: Session) => void) | undefined
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
        <InboxSessionList
          sessions={sessions}
          selectedId={selectedId}
          onSelect={onSelect}
          isMobile={isMobile}
          groupByProject={groupByProject}
          onLater={onLater}
          onCloseSession={onCloseSession}
          onOpenActions={onOpenActions}
        />
      ) : null}
    </section>
  )
}
