import '#components/session-heading.css'

import { CopySessionIdControl } from '#components/copy-session-id-control'
import { SessionPausedBadge } from '#components/session-paused-badge'

export function SessionHeading({
  title,
  isPaused,
  sessionId,
}: {
  title: string
  isPaused: boolean
  sessionId?: string
}) {
  return (
    <div className="session-heading-title-row">
      <h1>{title || 'Untitled session'}</h1>
      {sessionId === undefined ? null : (
        <CopySessionIdControl sessionId={sessionId} />
      )}
      <SessionPausedBadge isPaused={isPaused} />
    </div>
  )
}
