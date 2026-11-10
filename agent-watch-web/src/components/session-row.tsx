import '#components/later-today.css'

import { Button } from '@fohte/ui/button'
import { Chip } from '@fohte/ui/chip'

import { SessionPausedBadge } from '#components/session-paused-badge'
import { cn } from '#lib/utils'
import { formatLinkedTaskCount } from '#session-task-count'
import type { Session } from '#types'

type SessionRowProps = {
  session: Session
  selected: boolean
  onSelect: (id: string) => void
  onLater?: (id: string) => void
  onToggleToday?: (id: string, isToday: boolean) => void
  compactRepo?: boolean
}

function formatElapsedTime(totalSeconds: number) {
  if (totalSeconds < 60) {
    return `${String(totalSeconds)}s`
  }

  const minutes = Math.floor(totalSeconds / 60)
  if (minutes < 60) {
    return `${String(minutes)}m`
  }

  const hours = Math.floor(minutes / 60)
  if (hours < 24) {
    return `${String(hours)}h`
  }

  return `${String(Math.floor(hours / 24))}d`
}

function sessionStatus(session: Session) {
  if (session.category === 'in_progress') {
    return `Working · ${formatElapsedTime(session.elapsedSeconds)}`
  }

  if (session.category === 'done') {
    if (session.isPaused && session.pullRequest?.state !== 'Merged') {
      return formatElapsedTime(session.elapsedSeconds)
    }
    const detail =
      session.pullRequest?.state === 'Merged'
        ? `Merged #${String(session.pullRequest.number)}`
        : 'Finished'
    return `${detail} · ${formatElapsedTime(session.elapsedSeconds)}`
  }

  return formatElapsedTime(session.elapsedSeconds)
}

function sessionTaskLabel(session: Session) {
  if (session.task === null) {
    return '— No task'
  }

  const priority =
    session.task.priority === null ? '—' : `P${String(session.task.priority)}`
  const linkedTaskCount = formatLinkedTaskCount(session.tasks)
  return `${priority} #${String(session.task.number)} · ${session.task.title}${linkedTaskCount}`
}

function sessionKind(session: Session) {
  return session.kind ?? 'Stopped'
}

export function SessionRow({
  session,
  selected,
  onSelect,
  onLater,
  onToggleToday,
  compactRepo = false,
}: SessionRowProps) {
  const kind = sessionKind(session)

  return (
    <li className="session-list-item">
      <div className="session-row-layout">
        <Button
          type="button"
          variant="plain"
          className={cn(
            'session-row cursor-pointer',
            `category-${session.category}`,
            selected && 'is-selected',
          )}
          data-session-row=""
          aria-current={selected ? 'true' : undefined}
          onClick={() => {
            onSelect(session.id)
          }}
        >
          <span className="session-status-dot" aria-hidden="true" />
          <span className="session-content">
            <span className="session-title-line">
              <span
                className={`session-title${session.unread ? ' is-unread' : ''}`}
              >
                {session.title}
              </span>
              <SessionPausedBadge isPaused={session.isPaused} />
              <span className="session-age">{sessionStatus(session)}</span>
            </span>
            {session.category === 'needs_action' ? (
              <span className="session-preview">
                <Chip
                  data-session-kind={kind.toLowerCase()}
                  size="sm"
                  tone="strong"
                >
                  {kind}
                </Chip>
                {session.preview !== null && session.preview.length > 0 ? (
                  <span> · {session.preview}</span>
                ) : null}
              </span>
            ) : null}
            <span className="session-meta-line">
              <span className="session-task">{sessionTaskLabel(session)}</span>
              <span className="session-repo">
                {compactRepo
                  ? session.repo.split('/').at(-1)
                  : session.repo || '—'}
              </span>
            </span>
          </span>
        </Button>
        <div className="session-row-actions">
          {onToggleToday ? (
            <Button
              type="button"
              variant="plain"
              aria-label={
                session.isToday === true ? 'Remove from TODAY' : 'Add to TODAY'
              }
              aria-pressed={session.isToday ?? false}
              className="session-action-button cursor-pointer"
              onClick={() => {
                onToggleToday(session.id, session.isToday !== true)
              }}
            >
              {session.isToday === true ? '★' : '☆'}
            </Button>
          ) : null}
          {onLater ? (
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
          ) : null}
        </div>
      </div>
    </li>
  )
}
