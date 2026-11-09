import { Button } from '@fohte/ui/button'
import { Chip } from '@fohte/ui/chip'

import { cn } from '#lib/utils'
import type { Session } from '#types'

type SessionRowProps = {
  session: Session
  selected: boolean
  onSelect: (id: string) => void
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
  return `${priority} #${String(session.task.number)} · ${session.task.title}`
}

function sessionKind(session: Session) {
  return session.kind ?? 'Stopped'
}

export function SessionRow({ session, selected, onSelect }: SessionRowProps) {
  const kind = sessionKind(session)

  return (
    <li className="session-list-item">
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
            <span className="session-repo">{session.repo || '—'}</span>
          </span>
        </span>
      </Button>
    </li>
  )
}
