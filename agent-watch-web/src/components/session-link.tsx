import '#components/session-link.css'

import { Tooltip } from '@base-ui/react/tooltip'
import type { ReactNode } from 'react'

import { getCommandPaletteShortcut } from '#command-palette-shortcut'
import { formatElapsedTime } from '#components/format-elapsed-time'
import { formatLinkedTaskCount } from '#session-task-count'
import type { SessionLinkTarget } from '#types'

export type SessionLinkProps = {
  children: ReactNode
  onSelectSession?: ((id: string) => void) | undefined
  open?: boolean
  session: SessionLinkTarget
}

export function SessionLink({
  children,
  onSelectSession,
  open,
  session,
}: SessionLinkProps) {
  const title = session.title ?? children
  const workDetails = [session.workType, session.engine]
    .filter(
      (value) => value !== null && value !== undefined && value.length > 0,
    )
    .join(' · ')
  const statusDetails = [
    session.preview,
    session.elapsedSeconds === undefined
      ? undefined
      : formatElapsedTime(session.elapsedSeconds),
  ].filter((value) => value !== null && value !== undefined && value.length > 0)
  const taskPriority = session.task?.priority
  const taskReference =
    session.task === null
      ? undefined
      : taskPriority === null || taskPriority === undefined
        ? `#${String(session.task.number)}`
        : `P${String(taskPriority)} #${String(session.task.number)}`
  const linkedTaskCount = formatLinkedTaskCount(session.tasks)
  const openBesideModifier =
    getCommandPaletteShortcut(navigator.platform).modifier === 'metaKey'
      ? '⌘'
      : 'Ctrl'

  return (
    <Tooltip.Root open={open}>
      <Tooltip.Trigger
        delay={200}
        render={
          <a
            className="session-link-trigger"
            href={`?session=${encodeURIComponent(session.id)}`}
            onClick={(event) => {
              if (
                event.button !== 0 ||
                event.metaKey ||
                event.ctrlKey ||
                event.shiftKey ||
                event.altKey ||
                onSelectSession === undefined
              ) {
                return
              }
              event.preventDefault()
              onSelectSession(session.id)
            }}
          >
            {children}
          </a>
        }
      />
      <Tooltip.Portal>
        <Tooltip.Positioner className="session-link-positioner" sideOffset={8}>
          <Tooltip.Popup className="session-link-card">
            <div className="session-link-card-heading">
              <span className="session-link-card-title">{title}</span>
              {workDetails.length > 0 ? (
                <span className="session-link-card-work-details">
                  {workDetails}
                </span>
              ) : null}
            </div>
            {session.kind !== null && session.kind !== undefined ? (
              <div className="session-link-card-status">
                <span
                  className="session-link-card-kind"
                  data-session-kind={session.kind.toLowerCase()}
                >
                  {session.kind}
                </span>
                {statusDetails.length > 0 ? (
                  <span className="session-link-card-status-details">
                    {' · '}
                    {statusDetails.join(' · ')}
                  </span>
                ) : null}
              </div>
            ) : statusDetails.length > 0 ? (
              <div className="session-link-card-status">
                {statusDetails.join(' · ')}
              </div>
            ) : null}
            {session.task !== null || session.repo.length > 0 ? (
              <div className="session-link-card-footer-row">
                {session.task !== null ? (
                  <div className="session-link-card-task-row">
                    <span className="session-link-card-task-reference">
                      {taskReference}
                    </span>
                    <span className="session-link-card-task-title">
                      {session.task.title}
                    </span>
                    {linkedTaskCount.length > 0 ? (
                      <span className="session-link-card-task-count">
                        {linkedTaskCount}
                      </span>
                    ) : null}
                  </div>
                ) : null}
                {session.repo.length > 0 ? (
                  <span className="session-link-card-repo">{session.repo}</span>
                ) : null}
              </div>
            ) : null}
            <span className="session-link-card-hint">
              Click to open · {openBesideModifier}-click to open beside
            </span>
          </Tooltip.Popup>
        </Tooltip.Positioner>
      </Tooltip.Portal>
    </Tooltip.Root>
  )
}
