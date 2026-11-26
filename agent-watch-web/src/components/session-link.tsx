import '#components/session-link.css'

import { Tooltip } from '@base-ui/react/tooltip'
import { type ReactNode, useContext, useState } from 'react'

import {
  getCommandPaletteModifierLabel,
  getCommandPaletteShortcut,
} from '#command-palette-shortcut'
import { formatElapsedTime } from '#components/format-elapsed-time'
import { useSessionElapsedStartedAt } from '#components/session-elapsed-context'
import { SessionLinkElapsedSecondsContext } from '#components/session-link-elapsed-seconds'
import type { SessionLinkTarget } from '#types'
import { useLiveElapsedOffsetSeconds } from '#use-live-elapsed-offset-seconds'

export type SessionLinkProps = {
  children: ReactNode
  onSelectSession?: ((id: string) => void) | undefined
  open?: boolean
  session: SessionLinkTarget
}

function SessionLinkCard({
  openBesideModifier,
  session,
  title,
}: {
  openBesideModifier: string
  session: SessionLinkTarget
  title: ReactNode
}) {
  const elapsedSecondsById = useContext(SessionLinkElapsedSecondsContext)
  const elapsedSeconds =
    elapsedSecondsById.get(session.id) ?? session.elapsedSeconds
  const workDetails = [session.workType, session.engine]
    .filter(
      (value) => value !== null && value !== undefined && value.length > 0,
    )
    .join(' · ')
  const taskPriority = session.task?.priority
  const taskReference =
    session.task === null
      ? undefined
      : taskPriority === null || taskPriority === undefined
        ? `#${String(session.task.number)}`
        : `P${String(taskPriority)} #${String(session.task.number)}`
  const linkedTaskCount =
    session.linkedTaskCount !== undefined && session.linkedTaskCount > 1
      ? ` +${String(session.linkedTaskCount - 1)}`
      : ''

  return (
    <>
      <div className="session-link-card-heading">
        {title !== undefined ? (
          <span className="session-link-card-title">{title}</span>
        ) : null}
        {workDetails.length > 0 ? (
          <span className="session-link-card-work-details">{workDetails}</span>
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
          <SessionLinkStatusDetails
            preview={session.preview}
            elapsedSeconds={elapsedSeconds}
            leadingSeparator
          />
        </div>
      ) : session.preview !== null &&
        session.preview !== undefined &&
        session.preview.length > 0 ? (
        <div className="session-link-card-status">
          <SessionLinkStatusDetails
            preview={session.preview}
            elapsedSeconds={elapsedSeconds}
          />
        </div>
      ) : elapsedSeconds !== undefined ? (
        <div className="session-link-card-status">
          <SessionLinkStatusDetails elapsedSeconds={elapsedSeconds} />
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
    </>
  )
}

function SessionLinkStatusDetails({
  preview,
  elapsedSeconds,
  leadingSeparator = false,
}: {
  preview?: string | null | undefined
  elapsedSeconds?: number | undefined
  leadingSeparator?: boolean
}) {
  const startedAt = useSessionElapsedStartedAt()
  const hasPreview =
    preview !== null && preview !== undefined && preview.length > 0
  const hasElapsed = elapsedSeconds !== undefined
  if (!hasPreview && !hasElapsed) return null
  return (
    <span className="session-link-card-status-details">
      {leadingSeparator ? ' · ' : null}
      {hasPreview ? preview : null}
      {hasPreview && hasElapsed ? ' · ' : null}
      {elapsedSeconds === undefined ? null : startedAt === null ? (
        formatElapsedTime(elapsedSeconds)
      ) : (
        <LiveElapsedLabel
          elapsedSeconds={elapsedSeconds}
          startedAt={startedAt}
        />
      )}
    </span>
  )
}

function LiveElapsedLabel({
  elapsedSeconds,
  startedAt,
}: {
  elapsedSeconds: number
  startedAt: number
}) {
  const elapsedOffsetSeconds = useLiveElapsedOffsetSeconds(startedAt)
  return formatElapsedTime(elapsedSeconds + elapsedOffsetSeconds)
}

export function SessionLink({
  children,
  onSelectSession,
  open,
  session,
}: SessionLinkProps) {
  const [internalOpen, setInternalOpen] = useState(false)
  const isOpen = open ?? internalOpen
  const openBesideModifier = getCommandPaletteModifierLabel(
    getCommandPaletteShortcut(navigator.platform),
  )

  return (
    <Tooltip.Root open={isOpen} onOpenChange={setInternalOpen}>
      <Tooltip.Trigger
        delay={200}
        render={
          <a
            className="session-link-trigger"
            href={`?session=${encodeURIComponent(session.id)}`}
            rel={
              onSelectSession === undefined ? undefined : 'noopener noreferrer'
            }
            target={onSelectSession === undefined ? undefined : '_blank'}
            onClick={(event) => {
              if (event.button !== 0) {
                return
              }
              event.stopPropagation()
              if (
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
            {isOpen ? (
              <SessionLinkCard
                openBesideModifier={openBesideModifier}
                session={session}
                title={session.title ?? children}
              />
            ) : null}
          </Tooltip.Popup>
        </Tooltip.Positioner>
      </Tooltip.Portal>
    </Tooltip.Root>
  )
}
