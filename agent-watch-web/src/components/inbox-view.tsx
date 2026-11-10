import '#components/later-today.css'

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
  laterExpanded: boolean
  onToggleLater: () => void
  onLater: (id: string) => void
  onToggleToday: (id: string, isToday: boolean) => void
}

function renderSessionSection({
  title,
  sessions,
  count = sessions.length,
  selectedId,
  onSelect,
  onLater,
  onToggleToday,
  className = '',
}: {
  title: string
  sessions: readonly Session[]
  count?: number
  selectedId: string | null
  onSelect: (id: string) => void
  onLater: (id: string) => void
  onToggleToday: (id: string, isToday: boolean) => void
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
              onLater={onLater}
              onToggleToday={onToggleToday}
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
  laterExpanded,
  onToggleLater,
  onLater,
  onToggleToday,
}: InboxViewProps) {
  const needsAction = sessions.filter(
    (session) => session.category === 'needs_action',
  )
  const inProgress = sessions.filter(
    (session) => session.category === 'in_progress',
  )
  const done = sessions.filter((session) => session.category === 'done')
  const later = sessions.filter((session) => session.category === 'later')
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
          onLater,
          onToggleToday,
        })}
        <Button
          type="button"
          variant="plain"
          className="later-collapsed-row cursor-pointer"
          aria-expanded={laterExpanded}
          onClick={onToggleLater}
        >
          <span>◷ Later</span>
          <span className="later-section-count">{later.length}</span>
          {later.length > 0 ? (
            <span className="later-next-return">
              {nextReturnLabel(later)} {laterExpanded ? '▾' : '▸'}
            </span>
          ) : null}
        </Button>
        {laterExpanded && later.length > 0 ? (
          <ul className="session-list later-session-list">
            {later.map((session) => (
              <SessionRow
                key={session.id}
                session={session}
                selected={session.id === selectedId}
                onSelect={onSelect}
                onLater={onLater}
                onToggleToday={onToggleToday}
              />
            ))}
          </ul>
        ) : null}
        {renderSessionSection({
          title: 'In progress',
          className: 'in-progress-section',
          sessions: visibleInProgress,
          count: inProgress.length,
          selectedId,
          onSelect,
          onLater,
          onToggleToday,
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
          onLater,
          onToggleToday,
        })}
      </div>
      <footer className="inbox-footer" aria-label="Keyboard shortcuts">
        <span>
          <kbd>j</kbd> <kbd>k</kbd> Move
        </span>
        <span>
          <kbd>L</kbd> Later
        </span>
        <span>
          <kbd>⇧B</kbd> Board
        </span>
      </footer>
    </section>
  )
}

function nextReturnLabel(sessions: readonly Session[]) {
  const timed = sessions
    .map((session) => session.laterUntil)
    .filter((value): value is string => value !== null && value !== undefined)
    .sort((left, right) => Date.parse(left) - Date.parse(right))[0]
  if (timed === undefined) {
    return 'Next activity'
  }
  const deadline = new Date(timed)
  const time = new Intl.DateTimeFormat('en', {
    hour: 'numeric',
    minute: '2-digit',
    hourCycle: 'h23',
  }).format(deadline)
  const tomorrow = new Date()
  tomorrow.setDate(tomorrow.getDate() + 1)
  if (deadline.toDateString() === tomorrow.toDateString()) {
    return `Next back tomorrow ${time}`
  }
  if (deadline.getTime() - Date.now() > 86_400_000) {
    const date = new Intl.DateTimeFormat('en', {
      weekday: 'short',
      month: 'numeric',
      day: 'numeric',
    }).format(deadline)
    return `Next back ${date} ${time}`
  }
  return `Next back ${time}`
}
