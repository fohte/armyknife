import '#components/ended-session-banner.css'

import { Button } from '@fohte/ui/button'

import type { SessionActions } from '#types'

type ResumeDisabledReason = NonNullable<SessionActions['resumeDisabledReason']>

const RESUME_DISABLED_COPY: Record<
  ResumeDisabledReason,
  { hint: string; title: string }
> = {
  noTmux: {
    hint: 'Resume is unavailable because this session has no tmux location',
    title: 'This session has no tmux location',
  },
  workingDirectoryMissing: {
    hint: 'Resume is unavailable because this session’s working directory does not exist',
    title: 'This session’s working directory does not exist',
  },
}

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
  const disabledCopy = canResume
    ? undefined
    : RESUME_DISABLED_COPY[resumeDisabledReason ?? 'noTmux']
  const resumeHint = disabledCopy?.hint ?? 'Resume it to keep talking'

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
        title={disabledCopy?.title}
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
