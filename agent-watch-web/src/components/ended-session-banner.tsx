import '#components/ended-session-banner.css'

import { Button } from '@fohte/ui/button'

import type { SessionActions } from '#types'

type ResumeDisabledReason = NonNullable<SessionActions['resumeDisabledReason']>

type EndedSessionBannerProps = {
  endedAt: string | undefined
  isMobile: boolean
  isResuming: boolean
  canResume: boolean
  resumeDisabledReason?: ResumeDisabledReason | undefined
  onResume: () => void
}

export function EndedSessionBanner({
  endedAt,
  isMobile,
  isResuming,
  canResume,
  resumeDisabledReason,
  onResume,
}: EndedSessionBannerProps) {
  const endedTime = formatEndedTime(endedAt)
  const resumeHint = canResume
    ? 'Resume it to keep talking'
    : resumeDisabledReason === 'workingDirectoryMissing'
      ? 'Resume is unavailable because this session’s working directory does not exist'
      : 'Resume is unavailable because this session has no tmux location'

  return (
    <div className="conversation-ended" role="status">
      <span className="conversation-ended-dot" aria-hidden="true" />
      <span className="conversation-ended-copy">
        <span className="conversation-ended-title">This session has ended</span>
        {endedTime === null ? null : (
          <span className="conversation-ended-time">
            {' · '}
            <time dateTime={endedAt}>{endedTime}</time>
          </span>
        )}
        {canResume && isMobile ? null : (
          <span className="conversation-ended-hint">
            {' · '}
            {resumeHint}
          </span>
        )}
      </span>
      <Button
        className="conversation-ended-resume"
        type="button"
        disabled={isResuming || !canResume}
        title={
          canResume
            ? undefined
            : resumeDisabledReason === 'workingDirectoryMissing'
              ? 'This session’s working directory does not exist'
              : 'This session has no tmux location'
        }
        onClick={onResume}
      >
        {isResuming ? 'Resuming…' : isMobile ? 'Resume session' : 'Resume'}
      </Button>
    </div>
  )
}

function formatEndedTime(value?: string) {
  if (value === undefined) return null
  const date = new Date(value)
  if (Number.isNaN(date.getTime())) return null
  return new Intl.DateTimeFormat(undefined, {
    hour: '2-digit',
    minute: '2-digit',
    hourCycle: 'h23',
  }).format(date)
}
