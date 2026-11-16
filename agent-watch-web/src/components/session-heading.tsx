import '#components/session-heading.css'

import { CopySessionIdControl } from '#components/copy-session-id-control'

export function SessionHeading({
  title,
  sessionId,
}: {
  title: string
  sessionId?: string
}) {
  return (
    <div className="session-heading-title-row">
      <h1>{title || 'Untitled session'}</h1>
      {sessionId === undefined ? null : (
        <CopySessionIdControl sessionId={sessionId} />
      )}
    </div>
  )
}
