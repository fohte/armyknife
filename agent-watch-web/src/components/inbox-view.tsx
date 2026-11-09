import { Button } from '@fohte/ui/button'

import { SessionRow } from '#components/session-row'
import type { Session } from '#types'

type InboxViewProps = {
  sessions: readonly Session[]
  selectedId: string | null
  loaded: boolean
  error: string | null
  showAllInProgress: boolean
  onShowMore: () => void
  onSelect: (id: string) => void
}

function renderSessionSection({
  title,
  sessions,
  count = sessions.length,
  selectedId,
  onSelect,
  className = '',
}: {
  title: string
  sessions: readonly Session[]
  count?: number
  selectedId: string | null
  onSelect: (id: string) => void
  className?: string
}) {
  return (
    <section className={`session-section ${className}`} aria-label={title}>
      <h2 className="section-heading">
        <span>{title}</span>
        <span className="section-count">{count}</span>
      </h2>
      {sessions.length > 0 ? (
        <ul className="session-list">
          {sessions.map((session) => (
            <SessionRow
              key={session.id}
              session={session}
              selected={session.id === selectedId}
              onSelect={onSelect}
            />
          ))}
        </ul>
      ) : null}
    </section>
  )
}

export function InboxView({
  sessions,
  selectedId,
  loaded,
  error,
  showAllInProgress,
  onShowMore,
  onSelect,
}: InboxViewProps) {
  const needsAction = sessions.filter(
    (session) => session.category === 'needs_action',
  )
  const inProgress = sessions.filter(
    (session) => session.category === 'in_progress',
  )
  const done = sessions.filter((session) => session.category === 'done')
  const visibleInProgress = showAllInProgress
    ? inProgress
    : inProgress.slice(0, 2)
  const hiddenInProgressCount = inProgress.length - visibleInProgress.length

  return (
    <section className="inbox-view" aria-labelledby="inbox-title">
      <header className="view-header">
        <h1 id="inbox-title">Inbox</h1>
        <span className="sort-description">Sorted by tq priority</span>
      </header>
      <div className="inbox-scroll-area">
        {error !== null ? (
          <p className="load-message is-error" role="status">
            {error}
          </p>
        ) : null}
        {!loaded && error === null ? (
          <p className="load-message" role="status">
            Loading sessions…
          </p>
        ) : null}
        {loaded && sessions.length === 0 ? (
          <p className="load-message">No sessions to show.</p>
        ) : null}
        {renderSessionSection({
          title: 'Needs action',
          className: 'needs-action-section',
          sessions: needsAction,
          selectedId,
          onSelect,
        })}
        {renderSessionSection({
          title: 'In progress',
          className: 'in-progress-section',
          sessions: visibleInProgress,
          count: inProgress.length,
          selectedId,
          onSelect,
        })}
        {hiddenInProgressCount > 0 ? (
          <Button
            className="show-more-button cursor-pointer"
            type="button"
            variant="plain"
            aria-expanded={showAllInProgress}
            onClick={onShowMore}
          >
            {hiddenInProgressCount} more ▸
          </Button>
        ) : null}
        {renderSessionSection({
          title: 'Done',
          className: 'done-section',
          sessions: done,
          selectedId,
          onSelect,
        })}
      </div>
      <footer className="inbox-footer" aria-label="Keyboard shortcuts">
        <span>
          <kbd>j</kbd> <kbd>k</kbd> Move
        </span>
        <span>
          <kbd>⇧B</kbd> Board
        </span>
      </footer>
    </section>
  )
}
