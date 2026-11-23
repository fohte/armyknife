import '#components/pull-request-merge-button.css'

import { Button } from '@fohte/ui/button'
import { AlertCircle, LoaderCircle } from 'lucide-react'

type PullRequestMergeButtonProps = {
  status:
    | 'ready'
    | 'merging'
    | 'queued'
    | 'mergeFailed'
    | 'merged'
    | 'alreadyMerged'
    | 'unavailable'
  pullRequestNumber: number
  pullRequestRepo?: string
  isMobile: boolean
  placement: 'band' | 'row'
  unavailableReason?: string
  title?: string
  canRetry?: boolean
  onMerge?: () => void
  onRetry?: () => void
}

const statusLabels = {
  merged: 'Merged',
  alreadyMerged: 'Already merged',
  merging: 'Merging…',
  queued: 'Queued',
  mergeFailed: 'Merge failed',
} as const

export function PullRequestMergeButton({
  status,
  pullRequestNumber,
  pullRequestRepo,
  isMobile,
  placement,
  unavailableReason,
  title,
  canRetry = true,
  onMerge,
  onRetry,
}: PullRequestMergeButtonProps) {
  const className = `pr-merge-entry is-${placement}${placement === 'row' ? ' session-pr-row-state' : ''}${isMobile ? ' is-mobile' : ''}`
  if (status === 'unavailable') {
    return (
      <span className={`${className} is-unavailable`}>{unavailableReason}</span>
    )
  }
  if (status === 'merged' || status === 'alreadyMerged') {
    return (
      <span className={`${className} is-${status}`} role="status">
        {placement === 'row' ? '✓ ' : ''}
        {statusLabels[status]}
      </span>
    )
  }
  if (status === 'mergeFailed' && !canRetry) {
    return (
      <span className={`${className} is-mergeFailed`} role="status">
        {statusLabels.mergeFailed}
      </span>
    )
  }

  const ariaRepo = pullRequestRepo === undefined ? '' : ` in ${pullRequestRepo}`
  const disabled = status === 'merging' || status === 'queued'
  const bandActionClass =
    placement === 'band'
      ? ` merge-band-action ${
          status === 'ready'
            ? 'is-ready'
            : status === 'mergeFailed'
              ? 'is-failed'
              : 'is-pending'
        }`
      : ''

  const button = (
    <Button
      className={`${className} is-${status}${bandActionClass}`}
      title={title}
      type="button"
      variant={
        status === 'mergeFailed' || placement === 'row'
          ? 'outline'
          : 'secondary'
      }
      disabled={disabled}
      aria-label={
        status === 'ready'
          ? `Merge PR #${String(pullRequestNumber)}${ariaRepo}`
          : status === 'mergeFailed'
            ? `Retry merging PR #${String(pullRequestNumber)}`
            : `${statusLabels[status]} PR #${String(pullRequestNumber)}`
      }
      onClick={
        status === 'ready'
          ? onMerge
          : status === 'mergeFailed'
            ? onRetry
            : undefined
      }
    >
      {disabled ? <LoaderCircle aria-hidden="true" size={13} /> : null}
      {status === 'ready'
        ? 'Merge'
        : status === 'mergeFailed'
          ? 'Retry'
          : statusLabels[status]}
    </Button>
  )

  if (status === 'mergeFailed' && placement === 'row') {
    return (
      <span
        className={`pr-merge-entry-row-group${isMobile ? ' is-mobile' : ''}`}
      >
        <span
          className="pr-merge-entry-failure-label session-pr-row-state"
          role="status"
        >
          <AlertCircle aria-hidden="true" size={12} strokeWidth={2.5} />
          Failed
        </span>
        {button}
      </span>
    )
  }

  return button
}
