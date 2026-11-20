import { Button } from '@fohte/ui/button'

type PullRequestMergeActionProps = {
  ariaLabel?: string
  merging: boolean
  result?: 'merged' | 'alreadyMerged' | 'queued' | 'failed' | null
  onClick: () => void
}

export function PullRequestMergeAction({
  ariaLabel,
  merging,
  result = null,
  onClick,
}: PullRequestMergeActionProps) {
  const label = merging
    ? 'Merging…'
    : result === 'merged'
      ? 'Merged'
      : result === 'alreadyMerged'
        ? 'Already merged'
        : result === 'queued'
          ? 'In merge queue'
          : result === 'failed'
            ? 'Retry merge'
            : 'Merge'
  return (
    <Button
      className="conversation-pr-merge-button"
      type="button"
      variant="outline"
      disabled={merging || (result !== null && result !== 'failed')}
      aria-label={ariaLabel}
      onClick={onClick}
    >
      {label}
    </Button>
  )
}
