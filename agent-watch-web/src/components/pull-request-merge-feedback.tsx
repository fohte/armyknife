import '#components/pull-request-merge-feedback.css'

export type PullRequestMergeFeedbackKind = 'success' | 'queued' | 'error'

type PullRequestMergeFeedbackProps = {
  kind: PullRequestMergeFeedbackKind
  message: string
  isMobile?: boolean
}

export function PullRequestMergeFeedback({
  kind,
  message,
  isMobile = false,
}: PullRequestMergeFeedbackProps) {
  const mobileClass = isMobile ? ' conversation-pr-merge-feedback--mobile' : ''
  return (
    <span
      className={`conversation-pr-merge-feedback conversation-pr-merge-feedback--${kind}${mobileClass}`}
      role={kind === 'error' ? 'alert' : 'status'}
    >
      {message}
    </span>
  )
}
