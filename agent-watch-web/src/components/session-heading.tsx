import { SessionPausedBadge } from '#components/session-paused-badge'

export function SessionHeading({
  title,
  isPaused,
}: {
  title: string
  isPaused: boolean
}) {
  return (
    <>
      <h1>{title || 'Untitled session'}</h1>
      <SessionPausedBadge isPaused={isPaused} />
    </>
  )
}
